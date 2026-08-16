//! The interface to everything GeoTrace keeps on disk.
//!
//! Two databases live under one directory: the recording history
//! ([`gt_history`]) and the interference archive ([`gt_jam_store`]).
//! [`Store`] owns where they are and what they are called. The types for
//! working with them are re-exported here so call sites have one import.
//!
//! Settings are not part of this - they are a config file, not a database.
//!
//! Each database is opened on its own and returned owned, so one failing to
//! open says nothing about the other.

use std::path::{Path, PathBuf};

pub use gt_history::{
    ChannelSummary, DatabaseRef, DbError, HistoryDatabase, PruneMode, RecordingEntry,
    RecordingMeta, StoredRecording, StoredSegmentation, TrackRange, extract_meta,
    format_count_suffix, identity_from_group_name, identity_group_name, make_group_name,
};
pub use gt_jam_store::{JamStore, JamStoreError, StoredDay};

/// The recording history database. Named for what it holds, since the store
/// fronts more than one.
pub type Recordings = gt_history::Database;

/// Directory holding both databases, under the platform data directory.
const DIRECTORY: &str = "geotrace";

/// Everything [`Store`] itself can fail at. Opening a database yields that
/// database's own error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("no platform data directory available")]
    NoDataDir,
}

/// Where GeoTrace's databases live.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store under the platform data directory.
    pub fn open_default() -> Result<Self, StoreError> {
        let root = dirs::data_dir()
            .ok_or(StoreError::NoDataDir)?
            .join(DIRECTORY);
        Ok(Self::open_in(root))
    }

    /// The store under `root`, for tests and for a user-chosen location.
    pub fn open_in(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path of the recording history database.
    pub fn recordings_path(&self) -> PathBuf {
        self.root.join(gt_history::FILE_NAME)
    }

    /// Path of the interference archive.
    pub fn interference_path(&self) -> PathBuf {
        self.root.join(gt_jam_store::FILE_NAME)
    }

    /// Open the recording history, creating it if it does not exist.
    ///
    /// Returns [`DbError`] itself so callers can match
    /// [`DbError::WriteLocked`], which the user can clear, or
    /// [`DbError::Busy`] while another process holds it.
    pub fn open_recordings(&self) -> Result<Recordings, DbError> {
        Recordings::open_or_create(&self.recordings_path())
    }

    /// Open the interference archive, creating it if it does not exist.
    pub fn open_interference(&self) -> Result<JamStore, JamStoreError> {
        JamStore::open_or_create(&self.interference_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("temp dir: {err}"));
        let store = Store::open_in(dir.path());
        (dir, store)
    }

    #[test]
    fn both_databases_sit_under_one_root() {
        let (dir, store) = store();
        assert_eq!(store.root(), dir.path());
        assert_eq!(store.recordings_path().parent(), Some(dir.path()));
        assert_eq!(store.interference_path().parent(), Some(dir.path()));
        assert_ne!(store.recordings_path(), store.interference_path());
    }

    #[test]
    fn the_default_store_names_the_geotrace_directory() {
        let store = Store::open_default();
        if let Ok(store) = store {
            assert_eq!(store.root().file_name(), Some(DIRECTORY.as_ref()));
        }
    }

    #[test]
    fn opening_creates_each_database_under_the_root() {
        let (_dir, store) = store();
        store.open_recordings().expect("recordings");
        store.open_interference().expect("interference");
        assert!(store.recordings_path().exists());
        assert!(store.interference_path().exists());
    }

    #[test]
    fn the_archive_opens_without_the_recording_history() {
        let (_dir, store) = store();
        store.open_interference().expect("interference");
        assert!(store.interference_path().exists());
        assert!(!store.recordings_path().exists());
    }

    #[test]
    fn the_recording_history_opens_without_the_archive() {
        let (_dir, store) = store();
        store.open_recordings().expect("recordings");
        assert!(store.recordings_path().exists());
        assert!(!store.interference_path().exists());
    }

    /// The database's own error reaches the caller undisguised, which is what
    /// lets the app route [`DbError::Busy`] and [`DbError::WriteLocked`] to
    /// their own prompts and everything else to the corruption prompt. A
    /// genuine write lock needs a valid superblock checksum to reproduce, so
    /// this corrupts the file instead and checks the error is not wrapped or
    /// replaced.
    #[cfg(feature = "backend-sys")]
    #[test]
    fn a_broken_history_reports_the_databases_own_error() {
        use std::io::{Seek as _, SeekFrom, Write as _};

        let (_dir, store) = store();
        store.open_recordings().expect("create");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(store.recordings_path())
            .expect("raw open");
        file.seek(SeekFrom::Start(11)).expect("seek");
        file.write_all(&[0x01]).expect("corrupt the superblock");
        drop(file);

        // `Recordings` is not Debug, so this cannot use `expect_err`.
        match store.open_recordings() {
            Err(DbError::Backend(_) | DbError::WriteLocked) => {}
            Err(other) => panic!("gt-store altered the error: {other}"),
            Ok(_) => panic!("the corrupt database opened"),
        }
    }
}
