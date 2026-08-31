//! What ties a loaded log to the log stored with a recording in history, and
//! the attachments this session lists for the loaded recordings.

use gt_history_types::{DatabaseRef, LogAttachmentEntry, LogAttachmentId, StoredLogFilter};

/// Names the attachment a log was stored as: the recording carrying it, and
/// the attachment's own id on that recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogAttachmentRef {
    pub recording: DatabaseRef,
    pub id: LogAttachmentId,
}

/// A log's attachment, with the filter stack the database holds for it.
#[derive(Debug)]
pub(crate) struct LogAttachmentState {
    pub(crate) reference: LogAttachmentRef,

    /// The chips as they were last written. A stack that no longer serializes
    /// to this is one the next write-back stores.
    pub(crate) stored_filters: Vec<StoredLogFilter>,
}

/// Every attachment of the loaded recordings, as the history database last
/// listed them.
///
/// A recording is listed here from the read its load runs until it leaves the
/// session, whether or not any of its attachments is loaded as a log.
#[derive(Debug, Default)]
pub struct SessionLogAttachments {
    by_recording: Vec<RecordingAttachments>,
}

#[derive(Debug)]
struct RecordingAttachments {
    recording: DatabaseRef,
    entries: Vec<LogAttachmentEntry>,
}

impl RecordingAttachments {
    /// Puts the entries in the order the viewer lists them: by name, and by id
    /// for two attachments stored under one name.
    fn sort_by_name_then_id(&mut self) {
        self.entries.sort_by(|left, right| {
            left.attachment
                .name
                .cmp(&right.attachment.name)
                .then(left.id.cmp(&right.id))
        });
    }
}

impl SessionLogAttachments {
    /// Files `entries` under `recording`, replacing what this session listed
    /// for it before.
    pub fn set_attachments_of(&mut self, recording: DatabaseRef, entries: Vec<LogAttachmentEntry>) {
        match self.attachments_of_mut(&recording) {
            Some(listed) => {
                listed.entries = entries;
                listed.sort_by_name_then_id();
            }
            None => {
                let mut listed = RecordingAttachments { recording, entries };
                listed.sort_by_name_then_id();
                self.by_recording.push(listed);
            }
        }
    }

    /// Files one more attachment under `recording`, replacing the entry listed
    /// under the same id.
    pub fn record_attachment(&mut self, recording: DatabaseRef, entry: LogAttachmentEntry) {
        match self.attachments_of_mut(&recording) {
            Some(listed) => {
                listed.entries.retain(|listed| listed.id != entry.id);
                listed.entries.push(entry);
                listed.sort_by_name_then_id();
            }
            None => self.by_recording.push(RecordingAttachments {
                recording,
                entries: vec![entry],
            }),
        }
    }

    /// Drops one attachment, mirroring its removal from the database.
    pub fn remove_attachment(&mut self, attachment: &LogAttachmentRef) {
        if let Some(listed) = self.attachments_of_mut(&attachment.recording) {
            listed.entries.retain(|entry| entry.id != attachment.id);
        }
    }

    /// Drops everything listed for `recording`, mirroring that recording
    /// leaving the session.
    pub fn forget_recording(&mut self, recording: &DatabaseRef) {
        self.by_recording
            .retain(|listed| &listed.recording != recording);
    }

    /// The attachments of `recording`, by name and then by id.
    pub fn of_recording(&self, recording: &DatabaseRef) -> &[LogAttachmentEntry] {
        self.by_recording
            .iter()
            .find(|listed| &listed.recording == recording)
            .map_or(&[], |listed| listed.entries.as_slice())
    }

    fn attachments_of_mut(&mut self, recording: &DatabaseRef) -> Option<&mut RecordingAttachments> {
        self.by_recording
            .iter_mut()
            .find(|listed| &listed.recording == recording)
    }
}

#[cfg(test)]
mod tests {
    use gt_history_types::{LogAttachment, LogContentHash};

    use crate::test_fixtures::{recording_ref_of_group, stored_recording_ref};

    use super::*;

    fn entry(name: &str) -> LogAttachmentEntry {
        LogAttachmentEntry {
            id: LogAttachmentId::new_random(),
            attachment: LogAttachment::new(
                name.to_owned(),
                LogContentHash::of_log_bytes(name.as_bytes()),
                Vec::new(),
            ),
        }
    }

    fn names(attachments: &SessionLogAttachments, recording: &DatabaseRef) -> Vec<String> {
        attachments
            .of_recording(recording)
            .iter()
            .map(|entry| entry.attachment.name.clone())
            .collect()
    }

    /// The list a recording load fills, and the attachment an attach adds to
    /// it.
    #[test]
    fn a_recordings_attachments_are_listed_by_name() {
        let recording = stored_recording_ref();
        let mut attachments = SessionLogAttachments::default();

        attachments.set_attachments_of(
            recording.clone(),
            vec![entry("navsyncd.log"), entry("hal-powerd.log")],
        );
        attachments.record_attachment(recording.clone(), entry("kernel.log"));

        assert_eq!(
            names(&attachments, &recording),
            ["hal-powerd.log", "kernel.log", "navsyncd.log"]
        );
    }

    /// A detach takes one attachment out, and a recording leaving the session
    /// takes every attachment of that recording with it.
    #[test]
    fn a_removed_attachment_and_a_forgotten_recording_leave_the_other_recording_listed() {
        let removed_from = stored_recording_ref();
        let forgotten = recording_ref_of_group("2026-01-02T09-15-00");
        let mut attachments = SessionLogAttachments::default();
        let detached = entry("hal-powerd.log");
        attachments.set_attachments_of(
            removed_from.clone(),
            vec![detached.clone(), entry("navsyncd.log")],
        );
        attachments.set_attachments_of(forgotten.clone(), vec![entry("kernel.log")]);

        attachments.remove_attachment(&LogAttachmentRef {
            recording: removed_from.clone(),
            id: detached.id,
        });
        attachments.forget_recording(&forgotten);

        assert_eq!(names(&attachments, &removed_from), ["navsyncd.log"]);
        assert_eq!(names(&attachments, &forgotten), Vec::<String>::new());
    }
}
