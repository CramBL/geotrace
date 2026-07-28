pub use gt_history_types::{
    ChannelSummary, DatabaseRef, DbError, HistoryDatabase, PruneMode, RecordingEntry,
    RecordingMeta, StoredRecording, StoredSegmentation, TrackRange, format_count_suffix,
    identity_from_group_name, identity_group_name, make_group_name,
};
use std::path::PathBuf;

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

/// Re-export ActiveDb as Database to minimize changes to the app.
pub type Database = ActiveDb;

/// Returns the platform-specific default path for the database file:
/// `<data_dir>/geotrace/geotrace.h5`.
pub fn default_path() -> Result<PathBuf, DbError> {
    dirs::data_dir()
        .map(|d| d.join("geotrace").join("geotrace.h5"))
        .ok_or(DbError::NoDataDir)
}

/// Re-export `extract_meta` from the active backend so the default build does
/// not pull in the pure backend (and `hdf5-pure`) just for it.
#[cfg(feature = "backend-pure")]
pub use gt_history_backend_pure::extract_meta;

#[cfg(feature = "backend-sys")]
pub use gt_history_backend_sys::extract_meta;
