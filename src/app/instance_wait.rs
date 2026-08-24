//! Waiting for the GeoTrace instance that holds the data directory.
//!
//! A second instance opens its window like any other run, but opens no
//! database: archive recovery here would run against archives the instance
//! holding the directory is part-way through rewriting. It retries the lock
//! instead, and shows what that instance is doing until it lets go.
//!
//! The wait ends in one of four ways: this instance takes the lock, the user
//! takes write access from the instance holding it after a confirmation
//! naming what that costs, the user starts a read-only session beside it, or
//! the lock file stops opening for long enough that there is no instance left
//! to wait for. The last of those is the rule a run that starts on an
//! unusable lock file follows as well: an attempt that never reached the lock
//! names no owner, so the databases open here.

use std::mem;
use std::time::{Duration, Instant};

use egui::{RichText, Window};
use egui_phosphor::regular::WARNING as ICON_WARNING;
use gt_instance_lock::{
    DataDirectoryOwnership, InstanceState, InstanceStatus, SharedDataDirectoryLock,
};
use gt_ui_theme::warning_amber;

use super::App;
use super::modals;
use super::storage::{QueuedLoad, StorageOpen};

/// How often the wait tries the data directory again, and with it re-reads
/// why it does not have it.
pub(in crate::app) const DATA_DIRECTORY_RETRY_INTERVAL: Duration = Duration::from_millis(250);

/// How many retries in a row may find the lock file unusable before the wait
/// ends and the databases open without the mark. Five seconds of them rides
/// out a remount or a lock daemon restart, and nothing longer than that names
/// an instance to keep waiting for.
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
     an archive the other GeoTrace is part-way through deleting from.";

const TAKE_OVER_BUTTON_HOVER: &str =
    "Open the recordings and archives here without waiting for the other GeoTrace";

const START_READ_ONLY_BUTTON_HOVER: &str = "Read the recordings and archives here while the other GeoTrace keeps them: \
     nothing is stored, downloaded, deleted or saved. The session stays read-only until \
     GeoTrace is restarted.";

const WAIT_DIALOG_MIN_WIDTH: f32 = 360.0;

const TAKE_OVER_CONFIRMATION_MAX_WIDTH: f32 = 460.0;

/// Why this instance does not have the data directory, as the wait last
/// found it.
#[derive(Debug)]
enum DataDirectoryUnavailable {
    /// Another instance holds the lock, with what it last wrote about
    /// itself and [`None`] where that is unreadable.
    HeldByAnotherInstance(Option<InstanceStatus>),
    /// The lock file could not be opened or locked, with the cause of the
    /// last attempt. What has the directory is unknown.
    UnusableLockFile(String),
}

/// Whether the retry ended the wait.
#[derive(Debug, PartialEq, Eq)]
enum DataDirectoryRetry {
    Taken,
    StillWaiting,
    /// The lock file has not opened for
    /// [`UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS`] retries in a row,
    /// with the cause of the last one. What has the directory is unknown.
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

/// The retry that goes on once the databases are open without the mark,
/// until the data directory is marked as in use by this instance.
///
/// Taking the mark then is a promotion and nothing else: the databases are
/// already open and the wait is over.
#[derive(Debug)]
pub(in crate::app) struct BackgroundMarkRetry {
    last_attempt: Instant,
    attempts: u32,
}

impl DataDirectoryWait {
    pub(in crate::app) fn new(instance_lock: &SharedDataDirectoryLock) -> Self {
        Self {
            unavailable: DataDirectoryUnavailable::read_from(instance_lock),
            last_retry: Instant::now(),
            consecutive_unusable_lock_file_retries: 0,
            confirming_take_over: false,
        }
    }

    /// Tries the data directory again once the interval has passed, reading
    /// afresh why this instance does not have it.
    ///
    /// A lock file that would not open is retried for as long as whatever
    /// stopped it may pass. Past
    /// [`UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS`] the wait gives up
    /// on it: an attempt that never reaches the lock names no instance to
    /// wait for, and a wait that cannot end is worse than the open a fresh
    /// run would make here.
    fn retry_when_the_interval_has_passed(
        &mut self,
        now: Instant,
        instance_lock: &SharedDataDirectoryLock,
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
        let was_held_by_another_instance = matches!(
            self.unavailable,
            DataDirectoryUnavailable::HeldByAnotherInstance(_)
        );
        self.unavailable = DataDirectoryUnavailable::read_from(instance_lock);
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
            DataDirectoryUnavailable::HeldByAnotherInstance(status) => {
                status.as_ref().map(|status| status.process_id)
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

        let mut answer = WaitAnswer::KeepWaiting;
        Window::new(self.unavailable.dialog_title())
            .collapsible(false)
            .resizable(false)
            .min_width(WAIT_DIALOG_MIN_WIDTH)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                self.unavailable.wait_dialog_ui(ui);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Waiting for the data directory").weak());
                    ui.add_space(8.0);
                    if ui
                        .button(TAKE_OVER_BUTTON_LABEL)
                        .on_hover_text(TAKE_OVER_BUTTON_HOVER)
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

    fn interval_before_the_next_attempt(&self) -> Duration {
        DATA_DIRECTORY_RETRY_INTERVAL
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
    fn read_from(instance_lock: &SharedDataDirectoryLock) -> Self {
        match instance_lock.lock_file_failure() {
            Some(cause) => Self::UnusableLockFile(cause),
            None => Self::HeldByAnotherInstance(instance_lock.status_of_the_holding_instance()),
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
            Self::HeldByAnotherInstance(Some(status)) => match status.state {
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
            Self::HeldByAnotherInstance(None) => {
                ui.label(
                    "What it is doing is unknown. GeoTrace opens the recordings and archives \
                     here as soon as it lets go.",
                );
            }
            Self::UnusableLockFile(cause) => {
                ui.label(format!(
                    "The lock file cannot be used: {cause}. GeoTrace opens the recordings and \
                     archives here if it stays that way, since nothing names another GeoTrace to \
                     wait for.",
                ));
            }
        }
    }

    /// What taking write access overrides, as the instance holding the data
    /// directory last reported it.
    fn take_over_confirmation_ui(&self, ui: &mut egui::Ui) {
        match self {
            Self::HeldByAnotherInstance(Some(status)) => match status.state {
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
            Self::HeldByAnotherInstance(None) => {
                ui.label(
                    "What the other GeoTrace is doing is unknown, and it may be writing to the \
                     recordings and archives right now",
                );
            }
            Self::UnusableLockFile(cause) => {
                ui.label(format!(
                    "The lock file cannot be used: {cause}. Whether another GeoTrace is using \
                     this data directory is unknown.",
                ));
            }
        }
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
    let mut choice = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        .then_some(TakeOverChoice::Cancel);

    Window::new(TAKE_OVER_CONFIRMATION_TITLE)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(TAKE_OVER_CONFIRMATION_MAX_WIDTH);
            unavailable.take_over_confirmation_ui(ui);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(ICON_WARNING).color(warning_amber(ui.visuals().dark_mode)));
                ui.label(TAKE_OVER_WARNING);
            });
            ui.add_space(6.0);
            modals::dialog_button_row(ui, |ui| {
                if ui
                    .button(RichText::new("Take over").color(warning_amber(ui.visuals().dark_mode)))
                    .on_hover_text("This cannot be undone")
                    .clicked()
                {
                    choice = Some(TakeOverChoice::TakeOver);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(TakeOverChoice::Cancel);
                }
            });
        });

    choice
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
                self.open_the_databases_without_the_mark(ui.ctx(), queued_loads);
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
                    let queued_loads = mem::take(queued_loads);
                    self.open_the_databases_without_the_mark(ui.ctx(), queued_loads);
                }
                WaitAnswer::StartReadOnly => {
                    let owner_process_id = wait.owner_process_id();
                    let queued_loads = mem::take(queued_loads);
                    self.start_read_only_session(ui.ctx(), owner_process_id, queued_loads);
                }
            },
        }
    }

    /// Leaves the wait with the databases open while another instance may
    /// still hold the data directory: the archives are read for a delete that
    /// instance left part-way through, which is the user's to answer, and the
    /// mark is retried in the background until this instance holds it.
    fn open_the_databases_without_the_mark(
        &mut self,
        ctx: &egui::Context,
        queued_loads: Vec<QueuedLoad>,
    ) {
        self.background_mark_retry = Some(BackgroundMarkRetry::new());
        self.storage_open = self
            .storage
            .inspect_archives_in_background(ctx, queued_loads);
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

    /// Goes on retrying the mark behind an open window, so this instance is
    /// the one the data directory names as soon as it can be.
    ///
    /// Nothing else follows from taking the mark here: the databases opened
    /// when the wait ended, and they stay as they are.
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
        ctx.request_repaint_after(DATA_DIRECTORY_RETRY_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use gt_instance_lock::DataDirectoryLock;

    use super::*;

    /// A wait, on a directory another instance holds, whose lock file this
    /// instance can still reach.
    struct WaitOnAHeldDirectory {
        parent: tempfile::TempDir,
        directory: PathBuf,
        /// The instance holding the directory, which lives as long as the
        /// wait on it does.
        _holder: DataDirectoryLock,
        instance_lock: SharedDataDirectoryLock,
        wait: DataDirectoryWait,
        started: Instant,
    }

    impl WaitOnAHeldDirectory {
        fn new() -> Self {
            let parent = tempfile::tempdir().expect("temp dir");
            let directory = parent.path().join("data");
            let _holder = DataDirectoryLock::acquire(Some(&directory));
            let instance_lock = SharedDataDirectoryLock::acquire(Some(&directory));
            assert_eq!(
                instance_lock.ownership(),
                DataDirectoryOwnership::HeldByAnotherInstance,
                "the wait is meant to start out with another instance holding the directory"
            );
            let wait = DataDirectoryWait::new(&instance_lock);
            Self {
                parent,
                directory,
                _holder,
                instance_lock,
                wait,
                started: Instant::now(),
            }
        }

        /// Puts a file where the data directory was, which is what every
        /// later attempt to open the lock file under it runs into. The
        /// directory itself is moved aside with the lock the holder still has
        /// open in it, so [`Self::let_the_lock_file_open_again`] can put it
        /// back.
        fn make_the_lock_file_unusable(&self) {
            fs::rename(&self.directory, self.moved_directory()).expect("move the data directory");
            fs::write(&self.directory, b"not a directory").expect("put a file in its place");
        }

        fn let_the_lock_file_open_again(&self) {
            fs::remove_file(&self.directory).expect("clear the way");
            fs::rename(self.moved_directory(), &self.directory).expect("put the directory back");
        }

        fn moved_directory(&self) -> PathBuf {
            self.parent.path().join("moved")
        }

        /// The outcome of the `attempt`th retry of the wait, each one an
        /// interval after the last.
        fn retry(&mut self, attempt: u32) -> DataDirectoryRetry {
            self.wait.retry_when_the_interval_has_passed(
                self.started + DATA_DIRECTORY_RETRY_INTERVAL * attempt,
                &self.instance_lock,
            )
        }
    }

    /// Another instance holds the directory and this one waits, and then the
    /// lock file stops opening at all - a mount turned read-only, a lock
    /// daemon gone. Nothing names an instance to wait for any more, so the
    /// wait ends and the databases open.
    #[test]
    fn a_lock_file_that_stops_opening_mid_wait_ends_the_wait() {
        let mut wait = WaitOnAHeldDirectory::new();
        wait.make_the_lock_file_unusable();

        for attempt in 1..UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS {
            assert_eq!(
                wait.retry(attempt),
                DataDirectoryRetry::StillWaiting,
                "the wait gave up on attempt {attempt}, before the lock file had its retries"
            );
        }

        let outcome = wait.retry(UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS);

        assert!(
            matches!(outcome, DataDirectoryRetry::GaveUpOnTheLockFile { .. }),
            "a lock file that never opened left the wait at {outcome:?}"
        );
    }

    /// The retries the lock file gets are consecutive ones: a directory that
    /// reads as held again is an instance to wait for, and the count of
    /// unusable attempts starts over.
    #[test]
    fn a_directory_that_reads_as_held_again_gives_the_lock_file_its_retries_afresh() {
        let mut wait = WaitOnAHeldDirectory::new();
        wait.make_the_lock_file_unusable();
        for attempt in 1..UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS {
            assert_eq!(wait.retry(attempt), DataDirectoryRetry::StillWaiting);
        }
        wait.let_the_lock_file_open_again();
        assert_eq!(
            wait.retry(UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS),
            DataDirectoryRetry::StillWaiting,
            "the instance holding the directory is there to be waited for"
        );

        wait.make_the_lock_file_unusable();

        assert_eq!(
            wait.retry(UNUSABLE_LOCK_FILE_RETRIES_BEFORE_THE_WAIT_ENDS + 1),
            DataDirectoryRetry::StillWaiting,
            "one unusable attempt after the directory was held ended the wait"
        );
    }
}
