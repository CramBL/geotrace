pub use gt_history_types::{
    ChannelSummary, DatabaseRef, DbError, HistoryDatabase, LOGS_DIRECTORY, LogAttachment,
    LogAttachmentEntry, LogAttachmentId, LogContentHash, NavPointTimeRange, PruneMode,
    ReadOnlyHistoryDatabase, RecordingEntry, RecordingMeta, RecordingUiState,
    StoredFixPlacementRule, StoredLogFilter, StoredLogFilterMode, StoredRecording,
    StoredSegmentation, StoredTrackSplitRule, TrackRange, TrackState, UiStateVersionReporter,
    UiStateVersionTooNew, format_count_suffix, identity_from_group_name, identity_group_name,
    listed_track_rows, log_attachment, logs_directory_for_database, make_group_name,
};

#[cfg(feature = "backend-pure")]
use gt_history_backend_pure::{PureDb, ReadOnlyPureDb};
#[cfg(feature = "backend-sys")]
use gt_history_backend_sys::{ReadOnlySysDb, SysDb};

/// Re-export `extract_meta` from the active backend so the default build does
/// not pull in the pure backend (and `hdf5-pure`) just for it.
#[cfg(feature = "backend-pure")]
pub use gt_history_backend_pure::extract_meta;

#[cfg(feature = "backend-sys")]
pub use gt_history_backend_sys::extract_meta;

// Pure-Rust backend
#[cfg(feature = "backend-pure")]
pub type ActiveDb = PureDb;
#[cfg(feature = "backend-pure")]
pub type ActiveReadOnlyDb = ReadOnlyPureDb;

// C-backed (libhdf5) backend
#[cfg(feature = "backend-sys")]
pub type ActiveDb = SysDb;
#[cfg(feature = "backend-sys")]
pub type ActiveReadOnlyDb = ReadOnlySysDb;

#[cfg(all(feature = "backend-sys", feature = "backend-pure"))]
compile_error!("Features 'backend-sys' and 'backend-pure' are mutually exclusive.");

#[cfg(not(any(feature = "backend-sys", feature = "backend-pure")))]
compile_error!("Either 'backend-sys' or 'backend-pure' must be enabled.");

pub type Database = ActiveDb;

/// The database as a read-only session opens it. [`Database`] derefs to this,
/// which has no write method.
pub type ReadOnlyDatabase = ActiveReadOnlyDb;

/// Name of the database file. Where it sits is `gt-store`'s decision.
pub const FILE_NAME: &str = "geotrace.h5";
