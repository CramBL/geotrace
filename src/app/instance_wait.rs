//! Waiting for the GeoTrace instance that holds the data directory.
//!
//! A second instance opens its window like any other run, but opens no
//! database: archive recovery here would run against archives the instance
//! holding the directory is part-way through rewriting. It retries the lock
//! instead, and shows what that instance is doing until it lets go.
//!
//! The wait ends in one of two ways: this instance takes the lock, or the
//! user takes write access from the instance holding it, after a confirmation
//! naming what that costs. A lock file that will not open ends nothing by
//! itself: it leaves the databases closed until one of those two happens.

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
use super::storage::StorageOpen;

/// How often the wait tries the data directory again, and with it re-reads
/// why it does not have it.
pub(in crate::app) const DATA_DIRECTORY_RETRY_INTERVAL: Duration = Duration::from_millis(250);

pub(in crate::app) const DATA_DIRECTORY_HELD_TITLE: &str =
    "Another GeoTrace is using this data directory";

pub(in crate::app) const LOCK_FILE_UNUSABLE_TITLE: &str =
    "GeoTrace cannot lock this data directory";

pub(in crate::app) const TAKE_OVER_BUTTON_LABEL: &str = "Take over write access…";

pub(in crate::app) const TAKE_OVER_CONFIRMATION_TITLE: &str = "Take over write access?";

pub(in crate::app) const TAKE_OVER_WARNING: &str = "Opening the recordings and archives here can discard archived days: GeoTrace \
     recovers any archive the other GeoTrace is part-way through deleting from, and \
     that recovery drops every archived day the archive holds.";

const TAKE_OVER_BUTTON_HOVER: &str =
    "Open the recordings and archives here without waiting for the other GeoTrace";

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
}

/// Whether the user took write access from the instance holding the data
/// directory.
#[derive(Debug, PartialEq, Eq)]
enum WriteAccessTakeOver {
    Taken,
    NotTaken,
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
    confirming_take_over: bool,
}

/// The retry that goes on after the user took write access, until the data
/// directory is marked as in use by this instance.
///
/// Taking the mark then is a promotion and nothing else: the databases are
/// already open and the wait is over.
#[derive(Debug)]
pub(in crate::app) struct MarkRetryAfterTakeOver {
    last_retry: Instant,
}

impl DataDirectoryWait {
    pub(in crate::app) fn new(instance_lock: &SharedDataDirectoryLock) -> Self {
        Self {
            unavailable: DataDirectoryUnavailable::read_from(instance_lock),
            last_retry: Instant::now(),
            confirming_take_over: false,
        }
    }

    /// Tries the data directory again once the interval has passed, reading
    /// afresh why this instance does not have it.
    ///
    /// A lock file that would not open says nothing about who has the
    /// directory, so the wait goes on: an attempt that never reached the lock
    /// is no ground for opening a database here.
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
        if let DataDirectoryUnavailable::UnusableLockFile(cause) = &self.unavailable
            && was_held_by_another_instance
        {
            log::warn!("Cannot lock the data directory: {cause}");
        }
        DataDirectoryRetry::StillWaiting
    }

    /// Shows the wait, and reports the frame the user takes write access.
    ///
    /// The confirmation stands in for the wait dialog while it is up: both
    /// are anchored to the center of the window, and the confirmation names
    /// the same holder state the wait dialog does.
    fn ui(&mut self, ui: &egui::Ui) -> WriteAccessTakeOver {
        if self.confirming_take_over {
            return match show_take_over_confirmation(ui, &self.unavailable) {
                Some(TakeOverChoice::TakeOver) => WriteAccessTakeOver::Taken,
                Some(TakeOverChoice::Cancel) => {
                    self.confirming_take_over = false;
                    WriteAccessTakeOver::NotTaken
                }
                None => WriteAccessTakeOver::NotTaken,
            };
        }

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
                });
            });
        WriteAccessTakeOver::NotTaken
    }
}

impl MarkRetryAfterTakeOver {
    fn new() -> Self {
        Self {
            last_retry: Instant::now(),
        }
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
                        "Its window is closed and it is finishing these writes, after which \
                         GeoTrace opens the recordings and archives here.",
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
                    "The lock file cannot be used: {cause}. GeoTrace opens no recordings or \
                     archives here until that clears.",
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
                        "The other GeoTrace's window is closed and it is still finishing these \
                         writes",
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
            }
            DataDirectoryRetry::StillWaiting => match wait.ui(ui) {
                WriteAccessTakeOver::NotTaken => {
                    ui.ctx()
                        .request_repaint_after(DATA_DIRECTORY_RETRY_INTERVAL);
                    return;
                }
                WriteAccessTakeOver::Taken => {
                    log::warn!(
                        "The user took write access: opening the databases while another \
                         instance still holds the data directory"
                    );
                    self.mark_retry_after_take_over = Some(MarkRetryAfterTakeOver::new());
                }
            },
        }
        let queued_loads = mem::take(queued_loads);
        self.storage_open =
            self.storage
                .open_in_background(ui.ctx(), self.pending_writes.clone(), queued_loads);
    }

    /// Goes on retrying the mark after the user took write access, so this
    /// instance is the one the data directory names as soon as the other one
    /// exits.
    ///
    /// Nothing else follows from taking the mark here: the databases opened
    /// when the user took write access, and they stay as they are.
    pub(in crate::app) fn retry_marking_the_data_directory_after_take_over(
        &mut self,
        ctx: &egui::Context,
    ) {
        if self.shutdown.has_begun() {
            return;
        }
        let Some(retry) = &mut self.mark_retry_after_take_over else {
            return;
        };
        if retry.last_retry.elapsed() >= DATA_DIRECTORY_RETRY_INTERVAL {
            retry.last_retry = Instant::now();
            if self.instance_lock.retry_marking_the_data_directory()
                == DataDirectoryOwnership::MarkedByThisInstance
            {
                log::info!(
                    "The other instance let go of the data directory: this instance now marks it \
                     as in use"
                );
                self.mark_retry_after_take_over = None;
                return;
            }
        }
        ctx.request_repaint_after(DATA_DIRECTORY_RETRY_INTERVAL);
    }
}
