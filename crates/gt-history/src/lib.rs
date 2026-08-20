pub use gt_history_types::{
    ChannelSummary, DatabaseRef, DbError, HistoryDatabase, LOGS_DIRECTORY, LogAttachment,
    LogAttachmentEntry, LogAttachmentId, LogContentHash, PruneMode, RecordingEntry, RecordingMeta,
    StoredLogFilter, StoredLogFilterMode, StoredRecording, StoredSegmentation, TrackRange,
    format_count_suffix, identity_from_group_name, identity_group_name, log_attachment,
    logs_directory_for_database, make_group_name,
};

// Pure-Rust backend
#[cfg(feature = "backend-pure")]
pub mod pure_impl {
    pub use gt_history_backend_pure::{PureDb, extract_meta};
}
#[cfg(feature = "backend-pure")]
pub type ActiveDb = pure_impl::PureDb;

// C-backed (libhdf5) backend
#[cfg(feature = "backend-sys")]
pub mod sys_impl {
    pub use gt_history_backend_sys::SysDb;
}
#[cfg(feature = "backend-sys")]
pub type ActiveDb = sys_impl::SysDb;

#[cfg(all(feature = "backend-sys", feature = "backend-pure"))]
compile_error!("Features 'backend-sys' and 'backend-pure' are mutually exclusive.");

#[cfg(not(any(feature = "backend-sys", feature = "backend-pure")))]
compile_error!("Either 'backend-sys' or 'backend-pure' must be enabled.");

pub type Database = ActiveDb;

/// Name of the database file. Where it sits is `gt-store`'s decision.
pub const FILE_NAME: &str = "geotrace.h5";

/// Re-export `extract_meta` from the active backend so the default build does
/// not pull in the pure backend (and `hdf5-pure`) just for it.
#[cfg(feature = "backend-pure")]
pub use gt_history_backend_pure::extract_meta;

#[cfg(feature = "backend-sys")]
pub use gt_history_backend_sys::extract_meta;
