use std::sync::Arc;

use gt_history_types::{DatabaseRef, LogAttachmentId, StoredLogSettings};
use gt_loaded_files::LoadedFileId;
use gt_ui_types::LoadedLogId;
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogCapabilities {
    pub select_position_source: bool,
    pub save_with_recording: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PositionSourceSelection {
    NoSource,
    Recording(LoadedFileId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialLogSaveOutcome {
    AlreadyLoaded(LoadedLogId),
    Failed,
    Saved(LoadedLogId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialLogSaveStatus<'a> {
    Failed(&'a str),
    Idle,
    Saving,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum LogOperationError {
    #[error("This log is already saved with a recording")]
    AlreadySaved,
    #[error("This log has been unloaded")]
    LogUnloaded,
    #[error("This log has no failed save to retry")]
    NoFailedSave,
    #[error("Recording history is read-only")]
    ReadOnly,
    #[error("Wait for this log to finish saving")]
    SavePending,
    #[error("The selected recording is not stored in recording history")]
    TargetNotStored,
    #[error("The selected recording has been unloaded")]
    TargetUnloaded,
}

#[derive(Debug)]
pub struct LogSaveRequest {
    pub(crate) operation: LogSaveOperation,
    name: String,
    text: Arc<str>,
    settings: StoredLogSettings,
}

impl LogSaveRequest {
    pub(crate) fn new(
        log: LoadedLogId,
        recording: DatabaseRef,
        name: String,
        text: Arc<str>,
        settings: StoredLogSettings,
    ) -> Self {
        Self {
            operation: LogSaveOperation {
                log,
                recording,
                token: Uuid::new_v4(),
            },
            name,
            text,
            settings,
        }
    }

    pub fn complete(self, result: Result<LogSaveReceipt, String>) -> LogSaveCompletion {
        LogSaveCompletion {
            operation: self.operation,
            result,
        }
    }

    pub fn log(&self) -> LoadedLogId {
        self.operation.log
    }

    pub fn recording(&self) -> &DatabaseRef {
        &self.operation.recording
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    pub fn settings(&self) -> &StoredLogSettings {
        &self.settings
    }
}

#[derive(Clone, Debug)]
pub struct LogSaveReceipt {
    pub attachment_id: LogAttachmentId,
    pub stored_settings: StoredLogSettings,
}

#[derive(Clone, Debug)]
pub struct LogSaveCompletion {
    pub(crate) operation: LogSaveOperation,
    pub(crate) result: Result<LogSaveReceipt, String>,
}

impl LogSaveCompletion {
    pub fn log(&self) -> LoadedLogId {
        self.operation.log
    }

    pub fn recording(&self) -> &DatabaseRef {
        &self.operation.recording
    }

    pub fn result(&self) -> Result<&LogSaveReceipt, &str> {
        self.result.as_ref().map_err(String::as_str)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LogSaveOperation {
    pub log: LoadedLogId,
    pub recording: DatabaseRef,
    token: Uuid,
}

#[derive(Debug, Default)]
pub(crate) enum InitialLogSaveState {
    Failed {
        operation: LogSaveOperation,
        error: String,
    },
    #[default]
    Idle,
    Saving(LogSaveOperation),
}
