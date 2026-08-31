//! The recording a log is anchored to, and what identifies that recording.

use gt_history_types::DatabaseRef;
use gt_loaded_files::{LoadedFileEntry, LoadedFileId, LoadedFilesView};

use crate::attachment::LogAttachmentState;

/// The recording a log takes its positions from. A log stored with a recording
/// in history is anchored to it: only an anchored log has an attachment.
#[derive(Debug)]
pub(crate) enum LogAnchor {
    None,
    Recording {
        key: RecordingKey,
        attachment: Option<LogAttachmentState>,
    },
}

/// Identifies the recording a log is anchored to.
///
/// A log anchored to a recording in the history database finds that recording
/// again across an unload and the next load: the [`DatabaseRef`] is the same
/// both times. A recording that is not in the database is identified by the
/// session identity it holds for as long as it stays loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingKey {
    Stored(DatabaseRef),
    Session(LoadedFileId),
}

impl RecordingKey {
    /// The key `recording` is anchored under: its database entry when it has
    /// one, and its session identity otherwise.
    pub fn of_loaded_recording(recording: LoadedFileEntry<'_>) -> Self {
        match recording.history().db_ref() {
            Some(db_ref) => Self::Stored(db_ref.clone()),
            None => Self::Session(recording.id()),
        }
    }

    /// The loaded recording this key resolves to, `None` while no loaded
    /// recording matches it.
    pub(crate) fn loaded_recording<'a>(
        &self,
        recordings: &LoadedFilesView<'a>,
    ) -> Option<LoadedFileEntry<'a>> {
        match self {
            Self::Stored(db_ref) => recordings
                .entries()
                .find(|entry| entry.history().db_ref() == Some(db_ref)),
            Self::Session(id) => recordings.entry_for_id(*id),
        }
    }
}
