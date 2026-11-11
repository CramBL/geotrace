use gt_history_types::StoredLogSettings;
use gt_pending_writes::WriteAccess;
use uuid::Uuid;

use crate::LogAttachmentRef;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttachmentSettingsRevision {
    context: Uuid,
    revision: Uuid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentSettingsStatus<'a> {
    Dirty,
    Failed(&'a str),
    Saved,
    Saving,
    SessionOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentSettingsWrite {
    attachment: LogAttachmentRef,
    revision: AttachmentSettingsRevision,
    settings: StoredLogSettings,
}

impl AttachmentSettingsWrite {
    pub fn attachment(&self) -> &LogAttachmentRef {
        &self.attachment
    }

    pub fn revision(&self) -> AttachmentSettingsRevision {
        self.revision
    }

    pub fn settings(&self) -> &StoredLogSettings {
        &self.settings
    }

    pub fn complete(self, result: Result<(), String>) -> AttachmentSettingsCompletion {
        AttachmentSettingsCompletion {
            write: self,
            result,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AttachmentSettingsCompletion {
    write: AttachmentSettingsWrite,
    result: Result<(), String>,
}

impl AttachmentSettingsCompletion {
    pub fn attachment(&self) -> &LogAttachmentRef {
        self.write.attachment()
    }

    pub fn error(&self) -> Option<&str> {
        self.result.as_ref().err().map(String::as_str)
    }
}

#[derive(Debug)]
pub struct AttachmentSettingsState {
    attachment: LogAttachmentRef,
    desired: StoredLogSettings,
    context: Uuid,
    acknowledged: StoredLogSettings,
    write_state: SettingsWriteState,
}

impl AttachmentSettingsState {
    pub fn new(attachment: LogAttachmentRef, persisted: StoredLogSettings) -> Self {
        Self {
            attachment,
            desired: persisted.clone(),
            context: Uuid::new_v4(),
            acknowledged: persisted,
            write_state: SettingsWriteState::Idle,
        }
    }

    pub fn set_desired(&mut self, settings: StoredLogSettings) {
        self.desired = settings;
    }

    pub fn prepare_write(&mut self, access: WriteAccess) -> Option<AttachmentSettingsWrite> {
        if self.status(access) != AttachmentSettingsStatus::Dirty {
            return None;
        }
        let write = AttachmentSettingsWrite {
            attachment: self.attachment.clone(),
            revision: AttachmentSettingsRevision {
                context: self.context,
                revision: Uuid::new_v4(),
            },
            settings: self.desired.clone(),
        };
        self.write_state = SettingsWriteState::Saving(write.clone());
        Some(write)
    }

    pub fn acknowledge(&mut self, completion: AttachmentSettingsCompletion) -> bool {
        let SettingsWriteState::Saving(pending) = &self.write_state else {
            return false;
        };
        if pending != &completion.write {
            return false;
        }
        self.write_state = match completion.result {
            Ok(()) => {
                self.acknowledged = completion.write.settings;
                SettingsWriteState::Idle
            }
            Err(error) => SettingsWriteState::Failed(error),
        };
        true
    }

    pub fn retry(&mut self) -> bool {
        if !matches!(self.write_state, SettingsWriteState::Failed(_)) {
            return false;
        }
        self.write_state = SettingsWriteState::RetryRequested;
        true
    }

    pub fn attachment(&self) -> &LogAttachmentRef {
        &self.attachment
    }

    pub fn desired(&self) -> &StoredLogSettings {
        &self.desired
    }

    pub fn acknowledged(&self) -> &StoredLogSettings {
        &self.acknowledged
    }

    pub fn status(&self, access: WriteAccess) -> AttachmentSettingsStatus<'_> {
        if !access.allows_writing() {
            return AttachmentSettingsStatus::SessionOnly;
        }
        match &self.write_state {
            SettingsWriteState::Saving(_) => AttachmentSettingsStatus::Saving,
            SettingsWriteState::Failed(error) => AttachmentSettingsStatus::Failed(error),
            SettingsWriteState::Idle if self.desired == self.acknowledged => {
                AttachmentSettingsStatus::Saved
            }
            SettingsWriteState::RetryRequested | SettingsWriteState::Idle => {
                AttachmentSettingsStatus::Dirty
            }
        }
    }
}

#[derive(Debug)]
enum SettingsWriteState {
    Failed(String),
    Idle,
    RetryRequested,
    Saving(AttachmentSettingsWrite),
}

#[cfg(test)]
mod tests;
