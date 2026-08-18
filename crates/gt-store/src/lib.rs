//! The interface to everything GeoTrace keeps on disk.
//!
//! Five databases live under one directory: the recording history
//! ([`gt_history`]), the interference archive ([`gt_jam_store`]), the
//! geomagnetic index archive ([`gt_solar_store`]), the TEC map archive
//! ([`gt_ionex_store`]) and the solar flare archive ([`gt_flare_store`]).
//! [`Store`] owns where they are and what they are called. The types for
//! working with them are re-exported here so call sites have one import.
//!
//! Settings are not part of this - they are a config file, not a database.
//!
//! Each database is opened on its own, so one failing to open says nothing
//! about the other.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

pub use gt_flare_store::{ArchivedFlareDay, FlareStore, FlareStoreError};
pub use gt_history::{
    ChannelSummary, DatabaseRef, DbError, HistoryDatabase, PruneMode, RecordingEntry,
    RecordingMeta, StoredRecording, StoredSegmentation, TrackRange, extract_meta,
    format_count_suffix, identity_from_group_name, identity_group_name, make_group_name,
};
pub use gt_ionex_store::{ArchivedMapDay, IonexStore, IonexStoreError};
pub use gt_jam_store::{JamStore, JamStoreError, StoredDay};
pub use gt_solar_store::{ArchivedIndexDay, SolarStore, SolarStoreError};

/// The recording history database. Named for what it holds, since the store
/// fronts more than one.
pub type Recordings = gt_history::Database;

/// Directory holding every database, under the platform data directory.
const DIRECTORY: &str = "geotrace";

/// Everything [`Store`] itself can fail at. Opening a database yields that
/// database's own error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("no platform data directory available")]
    NoDataDir,
}

/// Where GeoTrace's databases live.
///
/// Every caller going through one store works through the same lock over each
/// archive file: each archive is opened once and shared from there. Not
/// [`Clone`], which would open a second archive per copy.
#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    interference: SharedArchive<JamStore>,
    geomagnetic_indices: SharedArchive<SolarStore>,
    tec_maps: SharedArchive<IonexStore>,
    solar_flares: SharedArchive<FlareStore>,
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
        Self {
            root: root.into(),
            interference: SharedArchive::empty(),
            geomagnetic_indices: SharedArchive::empty(),
            tec_maps: SharedArchive::empty(),
            solar_flares: SharedArchive::empty(),
        }
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

    /// Path of the geomagnetic index archive.
    pub fn geomagnetic_indices_path(&self) -> PathBuf {
        self.root.join(gt_solar_store::FILE_NAME)
    }

    /// Path of the TEC map archive.
    pub fn tec_maps_path(&self) -> PathBuf {
        self.root.join(gt_ionex_store::FILE_NAME)
    }

    /// Path of the solar flare archive.
    pub fn solar_flares_path(&self) -> PathBuf {
        self.root.join(gt_flare_store::FILE_NAME)
    }

    /// Open the recording history, creating it if it does not exist.
    ///
    /// Returns [`DbError`] itself so callers can match
    /// [`DbError::WriteLocked`], which the user can clear, or
    /// [`DbError::Busy`] while another process holds it.
    pub fn open_recordings(&self) -> Result<Recordings, DbError> {
        Recordings::open_or_create(&self.recordings_path())
    }

    /// The interference archive, creating it if it does not exist.
    ///
    /// Opened on the first call that succeeds and shared from then on.
    pub fn open_interference(&self) -> Result<Arc<JamStore>, JamStoreError> {
        self.interference
            .get_or_open(|| JamStore::open_or_create(&self.interference_path()))
    }

    /// The geomagnetic index archive, creating it if it does not exist.
    ///
    /// Shared the same way as [`Self::open_interference`].
    pub fn open_geomagnetic_indices(&self) -> Result<Arc<SolarStore>, SolarStoreError> {
        self.geomagnetic_indices
            .get_or_open(|| SolarStore::open_or_create(&self.geomagnetic_indices_path()))
    }

    /// The TEC map archive, creating it if it does not exist.
    ///
    /// Shared the same way as [`Self::open_interference`].
    pub fn open_tec_maps(&self) -> Result<Arc<IonexStore>, IonexStoreError> {
        self.tec_maps
            .get_or_open(|| IonexStore::open_or_create(&self.tec_maps_path()))
    }

    /// The solar flare archive, creating it if it does not exist.
    ///
    /// Shared the same way as [`Self::open_interference`].
    pub fn open_solar_flares(&self) -> Result<Arc<FlareStore>, FlareStoreError> {
        self.solar_flares
            .get_or_open(|| FlareStore::open_or_create(&self.solar_flares_path()))
    }
}

/// An archive opened on first use and shared from then on.
///
/// Only a successful open is kept: a failed one leaves the slot empty, so the
/// next caller opens again.
#[derive(Debug)]
struct SharedArchive<T>(Mutex<Option<Arc<T>>>);

impl<T> SharedArchive<T> {
    fn empty() -> Self {
        Self(Mutex::new(None))
    }

    fn get_or_open<E>(&self, open: impl FnOnce() -> Result<T, E>) -> Result<Arc<T>, E> {
        let mut shared = self.0.lock();
        if let Some(archive) = shared.as_ref() {
            return Ok(Arc::clone(archive));
        }
        Ok(Arc::clone(shared.insert(Arc::new(open()?))))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("temp dir: {err}"));
        let store = Store::open_in(dir.path());
        (dir, store)
    }

    #[test]
    fn every_database_sits_under_one_root_with_its_own_name() {
        let (dir, store) = store();
        assert_eq!(store.root(), dir.path());
        let paths = [
            store.recordings_path(),
            store.interference_path(),
            store.geomagnetic_indices_path(),
            store.tec_maps_path(),
            store.solar_flares_path(),
        ];
        for path in &paths {
            assert_eq!(path.parent(), Some(dir.path()));
        }
        let named: BTreeSet<&PathBuf> = paths.iter().collect();
        assert_eq!(named.len(), paths.len());
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
        store
            .open_geomagnetic_indices()
            .expect("geomagnetic indices");
        store.open_tec_maps().expect("tec maps");
        store.open_solar_flares().expect("solar flares");
        assert!(store.recordings_path().exists());
        assert!(store.interference_path().exists());
        assert!(store.geomagnetic_indices_path().exists());
        assert!(store.tec_maps_path().exists());
        assert!(store.solar_flares_path().exists());
    }

    /// One instance per store: two callers share the archive, and so share
    /// the lock that serializes writes to its file.
    #[test]
    fn each_archive_is_opened_once_and_shared() {
        let (_dir, store) = store();

        let interference = store.open_interference().expect("interference");
        let indices = store
            .open_geomagnetic_indices()
            .expect("geomagnetic indices");

        assert!(Arc::ptr_eq(
            &interference,
            &store.open_interference().expect("interference again")
        ));
        assert!(Arc::ptr_eq(
            &indices,
            &store
                .open_geomagnetic_indices()
                .expect("geomagnetic indices again")
        ));

        let maps = store.open_tec_maps().expect("tec maps");
        assert!(Arc::ptr_eq(
            &maps,
            &store.open_tec_maps().expect("tec maps again")
        ));

        let flares = store.open_solar_flares().expect("solar flares");
        assert!(Arc::ptr_eq(
            &flares,
            &store.open_solar_flares().expect("solar flares again")
        ));
    }

    /// A failure that has since been repaired must not keep the archive shut
    /// for the rest of the process: opening is idempotent.
    #[test]
    fn a_failed_open_is_retried_on_the_next_call() {
        let (_dir, store) = store();
        std::fs::write(store.interference_path(), b"not an archive").expect("write");

        store.open_interference().expect_err("garbage");
        std::fs::remove_file(store.interference_path()).expect("remove");

        store.open_interference().expect("retried after the repair");
        assert!(store.interference_path().exists());
    }

    #[test]
    fn the_interference_archive_opens_without_the_recording_history() {
        let (_dir, store) = store();
        store.open_interference().expect("interference");
        assert!(store.interference_path().exists());
        assert!(!store.recordings_path().exists());
        assert!(!store.geomagnetic_indices_path().exists());
        assert!(!store.tec_maps_path().exists());
        assert!(!store.solar_flares_path().exists());
    }

    #[test]
    fn the_geomagnetic_index_archive_opens_without_the_recording_history() {
        let (_dir, store) = store();
        store
            .open_geomagnetic_indices()
            .expect("geomagnetic indices");
        assert!(store.geomagnetic_indices_path().exists());
        assert!(!store.recordings_path().exists());
        assert!(!store.interference_path().exists());
        assert!(!store.tec_maps_path().exists());
        assert!(!store.solar_flares_path().exists());
    }

    #[test]
    fn the_tec_map_archive_opens_without_the_recording_history() {
        let (_dir, store) = store();
        store.open_tec_maps().expect("tec maps");
        assert!(store.tec_maps_path().exists());
        assert!(!store.recordings_path().exists());
        assert!(!store.interference_path().exists());
        assert!(!store.geomagnetic_indices_path().exists());
        assert!(!store.solar_flares_path().exists());
    }

    #[test]
    fn the_recording_history_opens_without_the_archives() {
        let (_dir, store) = store();
        store.open_recordings().expect("recordings");
        assert!(store.recordings_path().exists());
        assert!(!store.interference_path().exists());
        assert!(!store.geomagnetic_indices_path().exists());
        assert!(!store.tec_maps_path().exists());
        assert!(!store.solar_flares_path().exists());
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
