//! Waiting for the GeoTrace instance that holds the data directory.
//!
//! A second instance opens its window like any other run, but opens no
//! database: archive recovery here would run against archives the instance
//! holding the directory is part-way through rewriting. It retries the lock
//! instead, and shows what that instance is doing until it lets go. Once the
//! wait has begun, only taking the lock ends it: a lock file that will not
//! open leaves the databases closed.

use std::mem;
use std::time::{Duration, Instant};

use egui::{RichText, Window};
use gt_instance_lock::{
    DataDirectoryOwnership, InstanceState, InstanceStatus, SharedDataDirectoryLock,
};

use super::App;
use super::storage::StorageOpen;

/// How often the wait tries the data directory again, and with it re-reads
/// why it does not have it.
pub(in crate::app) const DATA_DIRECTORY_RETRY_INTERVAL: Duration = Duration::from_millis(250);

pub(in crate::app) const DATA_DIRECTORY_HELD_TITLE: &str =
    "Another GeoTrace is using this data directory";

pub(in crate::app) const LOCK_FILE_UNUSABLE_TITLE: &str =
    "GeoTrace cannot lock this data directory";

const WAIT_DIALOG_MIN_WIDTH: f32 = 360.0;

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

/// The wait for the data directory this instance has yet to take.
#[derive(Debug)]
pub(in crate::app) struct DataDirectoryWait {
    unavailable: DataDirectoryUnavailable,
    last_retry: Instant,
}

impl DataDirectoryWait {
    pub(in crate::app) fn new(instance_lock: &SharedDataDirectoryLock) -> Self {
        Self {
            unavailable: DataDirectoryUnavailable::read_from(instance_lock),
            last_retry: Instant::now(),
        }
    }

    /// Tries the data directory again once the interval has passed, reading
    /// afresh why this instance does not have it.
    ///
    /// Only taking the lock ends the wait. A lock file that would not open
    /// says nothing about who has the directory, so the wait goes on: an
    /// attempt that never reached the lock is no ground for opening a
    /// database here.
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

    fn ui(&self, ui: &egui::Ui) {
        Window::new(self.unavailable.dialog_title())
            .collapsible(false)
            .resizable(false)
            .min_width(WAIT_DIALOG_MIN_WIDTH)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                self.unavailable.ui(ui);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Waiting for the data directory").weak());
                });
            });
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

    fn ui(&self, ui: &mut egui::Ui) {
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
}

impl App {
    /// Retries the data directory another instance holds and shows what that
    /// instance is doing, until the directory is this instance's to open.
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
        if wait.retry_when_the_interval_has_passed(Instant::now(), &self.instance_lock)
            == DataDirectoryRetry::StillWaiting
        {
            wait.ui(ui);
            ui.ctx()
                .request_repaint_after(DATA_DIRECTORY_RETRY_INTERVAL);
            return;
        }
        let queued_loads = mem::take(queued_loads);
        log::info!("This instance now has the data directory: opening the databases");
        self.storage_open =
            self.storage
                .open_in_background(ui.ctx(), self.pending_writes.clone(), queued_loads);
    }
}
