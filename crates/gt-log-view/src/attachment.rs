//! What ties a loaded log to the log stored with a recording in history.

use gt_history_types::{DatabaseRef, LogAttachmentId, StoredLogFilter};

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
