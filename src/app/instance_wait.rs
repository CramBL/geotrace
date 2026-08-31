//! Waiting for the GeoTrace instance that holds the data directory.
//!
//! A second instance opens its window like any other run, but opens no
//! database: archive recovery here would run against archives the instance
//! holding the directory is part-way through rewriting. It retries the lock
//! instead, and shows what that instance is doing until it lets go.
//!
//! The wait ends in one of four ways: this instance takes the lock, the user
//! takes write access after a confirmation, the user starts a read-only
//! session, or `open_lock_file` fails for
//! [`UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS`] retries in a row. The
//! last case opens the databases, matching what `App::new` does when the
//! first attempt fails the same way: a failed open reports nothing about
//! which process holds the lock.

use std::mem;
use std::time::{Duration, Instant};

use egui::{RichText, Window};
use egui_phosphor::regular::WARNING as ICON_WARNING;
use gt_instance_lock::{
    DataDirectoryLock, DataDirectoryOwnership, InstanceState, InstanceStatusRead, StatusFreshness,
    TakeOverRecord,
};
use gt_pending_writes::WriteKind;
use gt_ui_theme::warning_amber;

use super::App;
use super::modals::{self, DialogActions, DialogBody};
use super::storage::{QueuedLoad, StorageOpen};

/// How often the wait tries the data directory again, and with it re-reads
/// why it does not have it.
pub(in crate::app) const DATA_DIRECTORY_RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// How many attempts the background retry makes at
/// [`DATA_DIRECTORY_RETRY_INTERVAL`] before the interval starts doubling.
const BACKGROUND_MARK_ATTEMPTS_AT_THE_WAIT_RATE: u32 = 20;

/// The ceiling the doubling background-retry interval stops at.
const SLOWEST_BACKGROUND_MARK_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// How many retries in a row may fail to open the lock file before the wait
/// ends and the databases open without the mark. At
/// [`DATA_DIRECTORY_RETRY_INTERVAL`] this is five seconds, enough to cover a
/// remount or a lock daemon restart.
const UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS: u32 = 20;

pub(in crate::app) const DATA_DIRECTORY_HELD_TITLE: &str =
    "Another GeoTrace is using this data directory";

pub(in crate::app) const LOCK_FILE_UNUSABLE_TITLE: &str =
    "GeoTrace cannot lock this data directory";

pub(in crate::app) const TAKE_OVER_BUTTON_LABEL: &str = "Take over write access…";

/// No suffix: the button needs no further input, because a read-only session
/// destroys nothing and overrides nothing.
pub(in crate::app) const START_READ_ONLY_BUTTON_LABEL: &str = "Start read-only";

pub(in crate::app) const TAKE_OVER_CONFIRMATION_TITLE: &str = "Take over write access?";

pub(in crate::app) const TAKE_OVER_WARNING: &str = "Writing to the recordings and archives while the other GeoTrace is still \
     writing to them can leave either of them inconsistent. GeoTrace asks first about \
     an archive the other GeoTrace is part-way through deleting from. Both windows also \
     write the same settings file, and the settings of whichever closes last are the \
     ones kept.";

const TAKE_OVER_BUTTON_HOVER: &str =
    "Open the recordings and archives here without waiting for the other GeoTrace";

const START_READ_ONLY_BUTTON_HOVER: &str = "Read the recordings and archives here while the other GeoTrace keeps them: \
     nothing is stored, downloaded, deleted or saved. The session stays read-only until \
     GeoTrace is restarted.";

/// Ends every wait dialog line about a status file that produced no status.
const OPENS_HERE_ONCE_THE_HOLDER_LETS_GO: &str =
    "GeoTrace opens the recordings and archives here as soon as it lets go.";

/// What the write recording a take-over registers under.
const RECORDING_THE_TAKE_OVER: &str = "Recording the take-over";

const WAIT_DIALOG_MIN_WIDTH: f32 = 360.0;

const TAKE_OVER_CONFIRMATION_MAX_WIDTH: f32 = 460.0;

/// Why this instance does not have the data directory, as the wait last
/// found it.
#[derive(Debug)]
enum DataDirectoryUnavailable {
    /// Another instance holds the lock, with the read of what it last wrote
    /// about itself.
    HeldByAnotherInstance(InstanceStatusRead),
    /// The lock file could not be opened or locked, with the cause of the
    /// last attempt. What has the directory is unknown.
    UnusableLockFile(String),
}

/// Whether the retry ended the wait.
#[derive(Debug, PartialEq, Eq)]
enum DataDirectoryRetry {
    Taken,
    StillWaiting,
    /// `open_lock_file` failed for
    /// [`UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS`] retries in a row,
    /// with the cause of the last one. Which process holds the directory is
    /// unknown.
    GaveUpOnTheLockFile {
        cause: String,
    },
}

/// How the user answered the wait dialog this frame.
#[derive(Debug, PartialEq, Eq)]
enum WaitAnswer {
    /// The dialog is up and the wait goes on.
    KeepWaiting,
    /// Open the databases here, overriding the instance holding the data
    /// directory.
    TakeOverWriteAccess,
    /// Read the databases beside the instance holding the data directory,
    /// writing nothing.
    StartReadOnly,
}

#[derive(Debug, PartialEq, Eq)]
enum TakeOverChoice {
    TakeOver,
    Cancel,
}

/// The wait for the data directory this instance has yet to take.
#[derive(Debug)]
pub(in crate::app) struct DataDirectoryWait {
    unavailable: DataDirectoryUnavailable,
    last_retry: Instant,
    consecutive_unusable_lock_file_retries: u32,
    confirming_take_over: bool,
}

/// Retries the mark after the databases were opened without it, until this
/// instance holds it.
///
/// Taking the mark here changes nothing else: the databases are already open.
/// The interval doubles as attempts accumulate, so a session that never gets
/// the mark stops requesting a repaint every
/// [`DATA_DIRECTORY_RETRY_INTERVAL`] for the rest of its run.
#[derive(Debug)]
pub(in crate::app) struct BackgroundMarkRetry {
    last_attempt: Instant,
    attempts: u32,
}

/// The instance the user took write access from, as its status file named it
/// at that moment. It may still have the recordings database and the archives
/// open for the rest of this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) struct TakenOverInstance {
    /// [`None`] where the status file could not be read.
    pub(in crate::app) process_id: Option<u32>,
}

impl TakenOverInstance {
    /// Names the instance as the subject of a sentence, e.g.
    /// `"Another GeoTrace (process 4210)"`.
    pub(in crate::app) fn sentence_subject(self) -> String {
        match self.process_id {
            Some(process_id) => format!("Another GeoTrace (process {process_id})"),
            None => "The other GeoTrace".to_owned(),
        }
    }
}

impl DataDirectoryWait {
    pub(in crate::app) fn new(instance_lock: &DataDirectoryLock) -> Self {
        Self {
            unavailable: DataDirectoryUnavailable::read_from(instance_lock),
            last_retry: Instant::now(),
            consecutive_unusable_lock_file_retries: 0,
            confirming_take_over: false,
        }
    }

    /// Tries the data directory again once the interval has passed, reading
    /// afresh why this instance does not have it.
    fn retry_when_the_interval_has_passed(
        &mut self,
        now: Instant,
        instance_lock: &DataDirectoryLock,
    ) -> DataDirectoryRetry {
        if now.saturating_duration_since(self.last_retry) < DATA_DIRECTORY_RETRY_INTERVAL {
            return DataDirectoryRetry::StillWaiting;
        }
        self.last_retry = now;
        if instance_lock.retry_marking_the_data_directory()
            == DataDirectoryOwnership::MarkedByThisInstance
        {
            return DataDirectoryRetry::Taken;
        }
        self.outcome_of_a_fresh_read(DataDirectoryUnavailable::read_from(instance_lock))
    }

    /// Where a retry that did not take the data directory leaves the wait.
    ///
    /// A failed `open_lock_file` keeps the wait going, since the cause may
    /// clear. Past [`UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS`] in a
    /// row the wait ends instead: a failed open reports nothing about which
    /// process holds the lock, so there is no instance to keep waiting for.
    fn outcome_of_a_fresh_read(&mut self, fresh: DataDirectoryUnavailable) -> DataDirectoryRetry {
        let was_held_by_another_instance = matches!(
            self.unavailable,
            DataDirectoryUnavailable::HeldByAnotherInstance(_)
        );
        self.unavailable = fresh;
        let DataDirectoryUnavailable::UnusableLockFile(cause) = &self.unavailable else {
            self.consecutive_unusable_lock_file_retries = 0;
            return DataDirectoryRetry::StillWaiting;
        };
        if was_held_by_another_instance {
            log::warn!("Cannot lock the data directory: {cause}");
        }
        self.consecutive_unusable_lock_file_retries = self
            .consecutive_unusable_lock_file_retries
            .saturating_add(1);
        if self.consecutive_unusable_lock_file_retries
            < UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS
        {
            return DataDirectoryRetry::StillWaiting;
        }
        DataDirectoryRetry::GaveUpOnTheLockFile {
            cause: cause.clone(),
        }
    }

    /// The process id the instance holding the data directory wrote to its
    /// status file. A read-only session names it as the owner.
    fn owner_process_id(&self) -> Option<u32> {
        match &self.unavailable {
            DataDirectoryUnavailable::HeldByAnotherInstance(read) => {
                read.status().map(|status| status.process_id)
            }
            DataDirectoryUnavailable::UnusableLockFile(_) => None,
        }
    }

    /// Shows the wait, and reports the frame the user answers it.
    ///
    /// The confirmation stands in for the wait dialog while it is up: both
    /// are anchored to the center of the window, and the confirmation names
    /// the same holder state the wait dialog does.
    fn ui(&mut self, ui: &egui::Ui) -> WaitAnswer {
        if self.confirming_take_over {
            return match show_take_over_confirmation(ui, &self.unavailable) {
                Some(TakeOverChoice::TakeOver) => WaitAnswer::TakeOverWriteAccess,
                Some(TakeOverChoice::Cancel) => {
                    self.confirming_take_over = false;
                    WaitAnswer::KeepWaiting
                }
                None => WaitAnswer::KeepWaiting,
            };
        }

        let take_over_hover = match self.unavailable.freshness_note() {
            Some(note) => format!("{TAKE_OVER_BUTTON_HOVER}. {note}."),
            None => TAKE_OVER_BUTTON_HOVER.to_owned(),
        };
        let mut answer = WaitAnswer::KeepWaiting;
        Window::new(self.unavailable.dialog_title())
            .collapsible(false)
            .resizable(false)
            .min_width(WAIT_DIALOG_MIN_WIDTH)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                modals::dialog_body_above_actions(
                    ui,
                    DialogBody::new(|ui| self.unavailable.wait_dialog_ui(ui)),
                    DialogActions::new(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Waiting for the data directory").weak());
                            ui.add_space(8.0);
                            if ui
                                .button(TAKE_OVER_BUTTON_LABEL)
                                .on_hover_text(&take_over_hover)
                                .clicked()
                            {
                                self.confirming_take_over = true;
                            }
                            if ui
                                .button(START_READ_ONLY_BUTTON_LABEL)
                                .on_hover_text(START_READ_ONLY_BUTTON_HOVER)
                                .clicked()
                            {
                                answer = WaitAnswer::StartReadOnly;
                            }
                        });
                    }),
                );
            });
        answer
    }
}

impl BackgroundMarkRetry {
    fn new() -> Self {
        Self {
            last_attempt: Instant::now(),
            attempts: 0,
        }
    }

    /// The interval before the next attempt: the first
    /// [`BACKGROUND_MARK_ATTEMPTS_AT_THE_WAIT_RATE`] run at
    /// [`DATA_DIRECTORY_RETRY_INTERVAL`], and each one after that doubles up
    /// to [`SLOWEST_BACKGROUND_MARK_RETRY_INTERVAL`].
    fn interval_before_the_next_attempt(&self) -> Duration {
        let next_attempt = self.attempts.saturating_add(1);
        let doublings = next_attempt.saturating_sub(BACKGROUND_MARK_ATTEMPTS_AT_THE_WAIT_RATE);
        DATA_DIRECTORY_RETRY_INTERVAL
            .saturating_mul(2u32.saturating_pow(doublings))
            .min(SLOWEST_BACKGROUND_MARK_RETRY_INTERVAL)
    }

    fn time_until_the_next_attempt(&self, now: Instant) -> Duration {
        self.interval_before_the_next_attempt()
            .saturating_sub(now.saturating_duration_since(self.last_attempt))
    }

    fn record_attempt(&mut self, now: Instant) {
        self.last_attempt = now;
        self.attempts = self.attempts.saturating_add(1);
    }
}

impl DataDirectoryUnavailable {
    /// `status_of_the_holding_instance` returns [`None`] unless another
    /// instance holds the data directory, which is the only state the wait
    /// this feeds runs in.
    fn read_from(instance_lock: &DataDirectoryLock) -> Self {
        match instance_lock.lock_file_failure() {
            Some(cause) => Self::UnusableLockFile(cause),
            None => Self::HeldByAnotherInstance(
                instance_lock
                    .status_of_the_holding_instance()
                    .unwrap_or(InstanceStatusRead::Absent),
            ),
        }
    }

    fn dialog_title(&self) -> &'static str {
        match self {
            Self::HeldByAnotherInstance(_) => DATA_DIRECTORY_HELD_TITLE,
            Self::UnusableLockFile(_) => LOCK_FILE_UNUSABLE_TITLE,
        }
    }

    fn wait_dialog_ui(&self, ui: &mut egui::Ui) {
        match self {
            Self::HeldByAnotherInstance(InstanceStatusRead::Status(status)) => match status.state {
                InstanceState::Running => {
                    ui.label(
                        "Its window is open: switch to it to keep working there. GeoTrace opens \
                         the recordings and archives here as soon as it closes.",
                    );
                }
                InstanceState::ShuttingDown => {
                    ui.label(
                        "It is shutting down and finishing these writes, after which GeoTrace \
                         opens the recordings and archives here.",
                    );
                    ui.add_space(4.0);
                    for write in &status.pending_writes {
                        ui.label(RichText::new(&write.label).strong());
                    }
                }
            },
            Self::HeldByAnotherInstance(InstanceStatusRead::Absent) => {
                ui.label(format!(
                    "It has not reported what it is doing yet. \
                     {OPENS_HERE_ONCE_THE_HOLDER_LETS_GO}"
                ));
            }
            Self::HeldByAnotherInstance(InstanceStatusRead::Unreadable(error)) => {
                ui.label(format!(
                    "Its status file cannot be read: {error}. \
                     {OPENS_HERE_ONCE_THE_HOLDER_LETS_GO}"
                ));
            }
            Self::HeldByAnotherInstance(InstanceStatusRead::Malformed(error)) => {
                ui.label(format!(
                    "Its status file is damaged: {error}. \
                     {OPENS_HERE_ONCE_THE_HOLDER_LETS_GO}"
                ));
            }
            Self::UnusableLockFile(cause) => {
                ui.label(format!(
                    "The lock file cannot be used: {cause}. GeoTrace opens the recordings and \
                     archives here if it stays that way, since nothing names another GeoTrace to \
                     wait for.",
                ));
            }
        }
        self.freshness_note_ui(ui);
    }

    /// What taking write access overrides, as the instance holding the data
    /// directory last reported it.
    fn take_over_confirmation_ui(&self, ui: &mut egui::Ui) {
        match self {
            Self::HeldByAnotherInstance(InstanceStatusRead::Status(status)) => match status.state {
                InstanceState::Running => {
                    ui.label(
                        "The other GeoTrace's window is open, and it may be writing to the \
                         recordings and archives right now",
                    );
                }
                InstanceState::ShuttingDown => {
                    ui.label(
                        "The other GeoTrace is shutting down and is still finishing these writes",
                    );
                    ui.add_space(4.0);
                    for write in &status.pending_writes {
                        ui.label(RichText::new(&write.label).strong());
                    }
                }
            },
            Self::HeldByAnotherInstance(InstanceStatusRead::Absent) => {
                ui.label(
                    "The other GeoTrace has not reported what it is doing, and it may be writing \
                     to the recordings and archives right now",
                );
            }
            Self::HeldByAnotherInstance(InstanceStatusRead::Unreadable(error)) => {
                ui.label(format!(
                    "The other GeoTrace's status file cannot be read: {error}. It may be writing \
                     to the recordings and archives right now."
                ));
            }
            Self::HeldByAnotherInstance(InstanceStatusRead::Malformed(error)) => {
                ui.label(format!(
                    "The other GeoTrace's status file is damaged: {error}. It may be writing to \
                     the recordings and archives right now."
                ));
            }
            Self::UnusableLockFile(cause) => {
                ui.label(format!(
                    "The lock file cannot be used: {cause}. Whether another GeoTrace is using \
                     this data directory is unknown.",
                ));
            }
        }
        self.freshness_note_ui(ui);
    }

    /// The line marking a [`StatusFreshness::Stale`] or
    /// [`StatusFreshness::UnknownAge`] report, and [`None`] for a report
    /// whose age says nothing.
    fn freshness_note(&self) -> Option<String> {
        let status = match self {
            Self::HeldByAnotherInstance(read) => read.status()?,
            Self::UnusableLockFile(_) => return None,
        };
        match status.freshness() {
            StatusFreshness::Current | StatusFreshness::NotRefreshedWhileRunning => None,
            StatusFreshness::Stale { age } => {
                let age_in_words = match age.as_secs() / 60 {
                    0 => format!("{} seconds", age.as_secs()),
                    1 => "a minute".to_owned(),
                    minutes => format!("{minutes} minutes"),
                };
                Some(format!(
                    "What it is doing now may differ from this: its last report is \
                     {age_in_words} old"
                ))
            }
            StatusFreshness::UnknownAge => Some(
                "The age of this report is unknown: the other GeoTrace stamped no time on it"
                    .to_owned(),
            ),
        }
    }

    fn freshness_note_ui(&self, ui: &mut egui::Ui) {
        let Some(note) = self.freshness_note() else {
            return;
        };
        ui.add_space(4.0);
        ui.label(RichText::new(note).color(warning_amber(ui.visuals().dark_mode)));
    }
}

/// Confirm opening the recordings and archives while another instance still
/// holds the data directory, naming what that instance is doing and what
/// opening here can discard.
///
/// Returns the choice in the frame the user makes it, and [`None`] while the
/// dialog is still open.
fn show_take_over_confirmation(
    ui: &egui::Ui,
    unavailable: &DataDirectoryUnavailable,
) -> Option<TakeOverChoice> {
    modals::confirmation_dialog(
        ui,
        TAKE_OVER_CONFIRMATION_TITLE,
        TAKE_OVER_CONFIRMATION_MAX_WIDTH,
        TakeOverChoice::Cancel,
        |ui| {
            unavailable.take_over_confirmation_ui(ui);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(ICON_WARNING).color(warning_amber(ui.visuals().dark_mode)));
                ui.label(TAKE_OVER_WARNING);
            });
        },
        |ui| {
            let mut choice = None;
            if modals::destructive_button(ui, "Take over").clicked() {
                choice = Some(TakeOverChoice::TakeOver);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(TakeOverChoice::Cancel);
            }
            choice
        },
    )
}

impl App {
    /// Retries the data directory another instance holds and shows what that
    /// instance is doing, until the directory is this instance's to open or
    /// the user takes write access from the instance holding it.
    ///
    /// A close ends the wait where it stands: an app on its way out opens no
    /// database.
    pub(in crate::app) fn wait_for_the_data_directory(&mut self, ui: &egui::Ui) {
        if self.shutdown.has_begun() {
            return;
        }
        let StorageOpen::WaitingForTheDataDirectory { wait, queued_loads } = &mut self.storage_open
        else {
            return;
        };
        match wait.retry_when_the_interval_has_passed(Instant::now(), &self.instance_lock) {
            DataDirectoryRetry::Taken => {
                log::info!("This instance now has the data directory: opening the databases");
                let queued_loads = mem::take(queued_loads);
                self.storage_open = self.storage.open_in_background(
                    ui.ctx(),
                    self.pending_writes.clone(),
                    queued_loads,
                );
            }
            DataDirectoryRetry::GaveUpOnTheLockFile { cause } => {
                log::warn!(
                    "The data directory's lock file has not opened for \
                     {UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS} attempts ({cause}): \
                     opening the recordings and archives here, since no instance is known to hold \
                     the data directory"
                );
                let queued_loads = mem::take(queued_loads);
                self.open_the_databases_without_the_mark(ui.ctx(), None, queued_loads);
            }
            DataDirectoryRetry::StillWaiting => match wait.ui(ui) {
                WaitAnswer::KeepWaiting => {
                    ui.ctx()
                        .request_repaint_after(DATA_DIRECTORY_RETRY_INTERVAL);
                }
                WaitAnswer::TakeOverWriteAccess => {
                    log::warn!(
                        "The user took write access: reading the archives for an interrupted \
                         delete while another instance still holds the data directory"
                    );
                    let taken_over = TakenOverInstance {
                        process_id: wait.owner_process_id(),
                    };
                    let queued_loads = mem::take(queued_loads);
                    self.open_the_databases_without_the_mark(
                        ui.ctx(),
                        Some(taken_over),
                        queued_loads,
                    );
                }
                WaitAnswer::StartReadOnly => {
                    let owner_process_id = wait.owner_process_id();
                    let queued_loads = mem::take(queued_loads);
                    self.start_read_only_session(ui.ctx(), owner_process_id, queued_loads);
                }
            },
        }
    }

    /// Ends the wait by opening the databases while another instance may
    /// still hold the data directory. The archives are inspected for an
    /// interrupted delete, and [`BackgroundMarkRetry`] keeps retrying the
    /// mark.
    ///
    /// `taken_over` is [`None`] where the wait ended because the lock file
    /// stopped opening, which leaves the holder of the directory unknown.
    /// A take-over is recorded in the data directory first, and the record it
    /// replaces is passed to the archive inspection.
    fn open_the_databases_without_the_mark(
        &mut self,
        ctx: &egui::Context,
        taken_over: Option<TakenOverInstance>,
        queued_loads: Vec<QueuedLoad>,
    ) {
        let previous_take_over =
            taken_over.and_then(|taken_over| self.record_take_over(taken_over));
        self.instance_taken_over_from = taken_over;
        self.background_mark_retry = Some(BackgroundMarkRetry::new());
        self.storage_open =
            self.storage
                .inspect_archives_in_background(ctx, previous_take_over, queued_loads);
    }

    /// Records this take-over in the data directory, and reports the
    /// take-over recorded there before it.
    ///
    /// A read-only session records nothing: the registry rejects the write.
    fn record_take_over(&self, taken_over: TakenOverInstance) -> Option<TakeOverRecord> {
        match self
            .pending_writes
            .try_begin(RECORDING_THE_TAKE_OVER, WriteKind::TakeOverRecord)
        {
            Ok(_write) => self.instance_lock.record_take_over(taken_over.process_id),
            Err(refusal) => {
                log::warn!("The take-over is not recorded in the data directory: {refusal}");
                None
            }
        }
    }

    /// Leaves the wait as a read-only session: the databases are opened
    /// without writing to or creating any of them, and the data directory is
    /// left to the instance that owns it.
    ///
    /// Giving up the mark keeps anything from promoting this session to the
    /// owner later. It stays read-only until GeoTrace is restarted, as the
    /// user chose.
    fn start_read_only_session(
        &mut self,
        ctx: &egui::Context,
        owner_process_id: Option<u32>,
        queued_loads: Vec<QueuedLoad>,
    ) {
        log::info!(
            "Starting read-only beside the GeoTrace that owns the data directory: this session \
             opens the recordings and archives without writing to any of them"
        );
        self.data_directory_owner_process_id = owner_process_id;
        self.pending_writes
            .become_read_only_for_the_rest_of_the_run();
        self.instance_lock.give_up_marking_the_data_directory();
        self.storage_open =
            self.storage
                .open_in_background(ctx, self.pending_writes.clone(), queued_loads);
    }

    /// Both instances are [`gt_pending_writes::WriteAccess::Owner`] until
    /// this one takes the mark, and both write the same `config.toml`.
    pub(in crate::app) fn took_write_access_from_an_instance_that_still_holds_the_mark(
        &self,
    ) -> bool {
        self.instance_taken_over_from.is_some() && self.background_mark_retry.is_some()
    }

    /// Retries the mark behind an open window, so the data directory's status
    /// file names this instance once the mark can be taken.
    ///
    /// Taking the mark here changes nothing else: the databases opened when
    /// the wait ended and stay as they are.
    pub(in crate::app) fn retry_marking_the_data_directory_in_the_background(
        &mut self,
        ctx: &egui::Context,
    ) {
        if self.shutdown.has_begun() {
            return;
        }
        let Some(retry) = &mut self.background_mark_retry else {
            return;
        };
        let now = Instant::now();
        if retry.time_until_the_next_attempt(now).is_zero() {
            retry.record_attempt(now);
            if self.instance_lock.retry_marking_the_data_directory()
                == DataDirectoryOwnership::MarkedByThisInstance
            {
                log::info!("This instance now marks the data directory as in use");
                self.background_mark_retry = None;
                return;
            }
        }
        ctx.request_repaint_after(retry.time_until_the_next_attempt(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNUSABLE_LOCK_FILE_CAUSE: &str = "the lock file cannot be opened";

    /// A wait as it stands the moment another instance is found holding the
    /// data directory, with nothing readable about that instance yet.
    fn wait_on_a_directory_held_by_another_instance() -> DataDirectoryWait {
        DataDirectoryWait {
            unavailable: DataDirectoryUnavailable::HeldByAnotherInstance(
                InstanceStatusRead::Absent,
            ),
            last_retry: Instant::now(),
            consecutive_unusable_lock_file_retries: 0,
            confirming_take_over: false,
        }
    }

    /// The wait ends and the databases open: a lock file that stops opening
    /// at all - a mount turned read-only, a lock daemon gone - leaves nothing
    /// naming an instance to wait for.
    #[test]
    fn a_lock_file_that_stops_opening_mid_wait_ends_the_wait() {
        let mut wait = wait_on_a_directory_held_by_another_instance();

        for attempt in 1..UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS {
            assert_eq!(
                wait.outcome_of_a_fresh_read(DataDirectoryUnavailable::UnusableLockFile(
                    UNUSABLE_LOCK_FILE_CAUSE.to_owned()
                )),
                DataDirectoryRetry::StillWaiting,
                "the wait gave up on attempt {attempt}, before the lock file had its retries"
            );
        }

        assert_eq!(
            wait.outcome_of_a_fresh_read(DataDirectoryUnavailable::UnusableLockFile(
                UNUSABLE_LOCK_FILE_CAUSE.to_owned()
            )),
            DataDirectoryRetry::GaveUpOnTheLockFile {
                cause: UNUSABLE_LOCK_FILE_CAUSE.to_owned()
            }
        );
    }

    /// A session that spends the rest of its run beside the instance holding
    /// the directory attempts the mark ever more rarely, and the window it
    /// paints behind goes idle between attempts.
    #[test]
    fn the_background_mark_retry_spreads_its_attempts_out_until_they_settle() {
        let mut retry = BackgroundMarkRetry::new();
        let mut now = retry.last_attempt;

        let intervals: Vec<Duration> = (0..30)
            .map(|_| {
                let interval = retry.time_until_the_next_attempt(now);
                now += interval;
                retry.record_attempt(now);
                interval
            })
            .collect();

        let (at_the_wait_rate, spread_out) =
            intervals.split_at(BACKGROUND_MARK_ATTEMPTS_AT_THE_WAIT_RATE as usize);
        assert!(
            at_the_wait_rate
                .iter()
                .all(|interval| *interval == DATA_DIRECTORY_RETRY_INTERVAL),
            "the first attempts run at the wait's rate: {at_the_wait_rate:?}"
        );
        assert_eq!(
            spread_out,
            [
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                SLOWEST_BACKGROUND_MARK_RETRY_INTERVAL,
                SLOWEST_BACKGROUND_MARK_RETRY_INTERVAL,
                SLOWEST_BACKGROUND_MARK_RETRY_INTERVAL,
                SLOWEST_BACKGROUND_MARK_RETRY_INTERVAL,
            ]
        );
    }

    /// The retries the lock file gets are consecutive ones: a directory that
    /// reads as held again is an instance to wait for, and the count of
    /// unusable attempts starts over.
    #[test]
    fn a_directory_that_reads_as_held_again_gives_the_lock_file_its_retries_afresh() {
        let mut wait = wait_on_a_directory_held_by_another_instance();
        for _ in 1..UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS {
            assert_eq!(
                wait.outcome_of_a_fresh_read(DataDirectoryUnavailable::UnusableLockFile(
                    UNUSABLE_LOCK_FILE_CAUSE.to_owned()
                )),
                DataDirectoryRetry::StillWaiting
            );
        }
        assert_eq!(
            wait.outcome_of_a_fresh_read(DataDirectoryUnavailable::HeldByAnotherInstance(
                InstanceStatusRead::Absent
            )),
            DataDirectoryRetry::StillWaiting,
            "the instance holding the directory is there to be waited for"
        );

        assert_eq!(
            wait.outcome_of_a_fresh_read(DataDirectoryUnavailable::UnusableLockFile(
                UNUSABLE_LOCK_FILE_CAUSE.to_owned()
            )),
            DataDirectoryRetry::StillWaiting,
            "one unusable attempt after the directory was held ended the wait"
        );
    }

    /// The wait dialog and its take-over confirmation both stay inside the
    /// screen and keep their actions reachable, however long the lock file's
    /// failure reads and however many writes the holding instance lists.
    #[rstest::rstest]
    fn the_wait_dialogs_fit_every_viewport(
        #[values(false, true)] confirming_take_over: bool,
        #[values(
            gt_test_utils::window_fit::CRAMPED_VIEWPORT,
            gt_test_utils::window_fit::NARROW_VIEWPORT,
            gt_test_utils::window_fit::SHORT_VIEWPORT
        )]
        viewport: egui::Vec2,
    ) {
        use gt_test_utils::{AuditedWindow, ControlLabel, WindowFitAssertions as _};

        let wait = DataDirectoryWait {
            unavailable: DataDirectoryUnavailable::UnusableLockFile(gt_test_utils::oversized_text(
                'l',
            )),
            last_retry: Instant::now(),
            consecutive_unusable_lock_file_retries: 0,
            confirming_take_over,
        };
        let mut harness = gt_test_utils::TestHarness::builder()
            .size(viewport)
            .ui_state(
                |ui: &mut egui::Ui, wait: &mut DataDirectoryWait| {
                    wait.ui(ui);
                },
                wait,
            );
        harness.inner.run_steps(8);

        let (title, control) = if confirming_take_over {
            (TAKE_OVER_CONFIRMATION_TITLE, "Cancel")
        } else {
            (LOCK_FILE_UNUSABLE_TITLE, START_READ_ONLY_BUTTON_LABEL)
        };
        harness
            .inner
            .assert_window_fits_the_viewport(AuditedWindow::titled(title));
        harness
            .inner
            .assert_control_is_reachable(AuditedWindow::titled(title), ControlLabel(control));
    }
}
