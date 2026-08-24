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

use parking_lot::Mutex;

pub use gt_flare_store::{ArchivedFlareDay, FlareStore, FlareStoreError, ReadOnlyFlareStore};
pub use gt_hdf5_archive::prune::{
    DeclinedRecovery, InterruptedDelete, InterruptedDeleteRecovery, PruneProgress,
    PruneProgressSink,
};
pub use gt_hdf5_archive::{ArchiveUsage, ArchivedDaySpan};
pub use gt_history::{
    ChannelSummary, DatabaseRef, DbError, HistoryDatabase, LOGS_DIRECTORY, LogAttachment,
    LogAttachmentEntry, LogAttachmentId, LogContentHash, PruneMode, ReadOnlyHistoryDatabase,
    RecordingEntry, RecordingMeta, StoredLogFilter, StoredLogFilterMode, StoredRecording,
    StoredSegmentation, TrackRange, extract_meta, format_count_suffix, identity_from_group_name,
    identity_group_name, make_group_name,
};
pub use gt_ionex_store::{ArchivedMapDay, IonexStore, IonexStoreError, ReadOnlyIonexStore};
pub use gt_jam_store::{JamStore, JamStoreError, ReadOnlyJamStore, StoredDay};
pub use gt_solar_store::{ArchivedIndexDay, ReadOnlySolarStore, SolarStore, SolarStoreError};

mod archive_handle;
mod day_archive;
pub mod log_attachments;
mod recordings_handle;
mod writable_archive;

pub use archive_handle::ArchiveHandle;
pub use day_archive::DayArchiveError;
pub use log_attachments::{
    AttachedLog, LogAttachmentError, LogAttachments, LogToAttach, ReadOnlyLogAttachments,
};
pub use recordings_handle::RecordingsHandle;
pub use writable_archive::WritableArchive;

/// The recording history database. Named for what it holds, since the store
/// fronts more than one.
pub type Recordings = gt_history::Database;

/// The recording history database as a read-only session opens it.
pub type ReadOnlyRecordings = gt_history::ReadOnlyDatabase;

/// The interference archive as this session opened it.
pub type InterferenceArchive = ArchiveHandle<JamStore, ReadOnlyJamStore>;

/// The geomagnetic index archive as this session opened it.
pub type GeomagneticIndexArchive = ArchiveHandle<SolarStore, ReadOnlySolarStore>;

/// The TEC map archive as this session opened it.
pub type TecMapArchive = ArchiveHandle<IonexStore, ReadOnlyIonexStore>;

/// The solar flare archive as this session opened it.
pub type SolarFlareArchive = ArchiveHandle<FlareStore, ReadOnlyFlareStore>;

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
    interference: SharedArchive<JamStore, ReadOnlyJamStore>,
    geomagnetic_indices: SharedArchive<SolarStore, ReadOnlySolarStore>,
    tec_maps: SharedArchive<IonexStore, ReadOnlyIonexStore>,
    solar_flares: SharedArchive<FlareStore, ReadOnlyFlareStore>,
}

impl Store {
    /// The store under the platform data directory.
    pub fn open_default() -> Result<Self, StoreError> {
        Ok(Self::open_in(Self::default_root()?))
    }

    /// Where [`Self::open_default`] puts the databases, without opening any
    /// of them.
    pub fn default_root() -> Result<PathBuf, StoreError> {
        Ok(dirs::data_dir()
            .ok_or(StoreError::NoDataDir)?
            .join(DIRECTORY))
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

    /// Directory of the logs attached to recordings in the history database,
    /// created when the first log is attached.
    ///
    /// Deleting a recording deletes the logs attached to it.
    pub fn logs_path(&self) -> PathBuf {
        self.root.join(LOGS_DIRECTORY)
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

    /// Open the recording history without writing to it, or [`None`] where
    /// there is none to open: a read-only session creates no database and
    /// repairs none.
    pub fn open_recordings_read_only(&self) -> Result<Option<ReadOnlyRecordings>, DbError> {
        let path = self.recordings_path();
        if !path.exists() {
            return Ok(None);
        }
        ReadOnlyRecordings::open_existing_read_only(&path).map(Some)
    }

    /// The interference archive, creating it if it does not exist.
    ///
    /// Opened on the first call that succeeds and shared from then on.
    pub fn open_interference(&self) -> Result<InterferenceArchive, JamStoreError> {
        self.open_interference_with_recovery_choice(InterruptedDeleteRecovery::Recover)
    }

    /// The interference archive, recovering an interrupted delete only when
    /// `recovery` asks for it.
    pub fn open_interference_with_recovery_choice(
        &self,
        recovery: InterruptedDeleteRecovery,
    ) -> Result<InterferenceArchive, JamStoreError> {
        self.interference.get_or_open(|| {
            JamStore::open_or_create_with_recovery_choice(&self.interference_path(), recovery)
                .map(ArchiveHandle::owner)
        })
    }

    /// The interference archive as a read-only session opens it, without
    /// writing to it.
    pub fn open_interference_read_only(&self) -> Result<InterferenceArchive, JamStoreError> {
        self.interference.get_or_open(|| {
            ReadOnlyJamStore::open_existing_read_only(&self.interference_path())
                .map(ArchiveHandle::read_only)
        })
    }

    /// The geomagnetic index archive, creating it if it does not exist.
    ///
    /// Shared the same way as [`Self::open_interference`].
    pub fn open_geomagnetic_indices(&self) -> Result<GeomagneticIndexArchive, SolarStoreError> {
        self.open_geomagnetic_indices_with_recovery_choice(InterruptedDeleteRecovery::Recover)
    }

    /// The geomagnetic index archive, recovering an interrupted delete only
    /// when `recovery` asks for it.
    pub fn open_geomagnetic_indices_with_recovery_choice(
        &self,
        recovery: InterruptedDeleteRecovery,
    ) -> Result<GeomagneticIndexArchive, SolarStoreError> {
        self.geomagnetic_indices.get_or_open(|| {
            SolarStore::open_or_create_with_recovery_choice(
                &self.geomagnetic_indices_path(),
                recovery,
            )
            .map(ArchiveHandle::owner)
        })
    }

    /// The geomagnetic index archive as a read-only session opens it, without
    /// writing to it.
    pub fn open_geomagnetic_indices_read_only(
        &self,
    ) -> Result<GeomagneticIndexArchive, SolarStoreError> {
        self.geomagnetic_indices.get_or_open(|| {
            ReadOnlySolarStore::open_existing_read_only(&self.geomagnetic_indices_path())
                .map(ArchiveHandle::read_only)
        })
    }

    /// The TEC map archive, creating it if it does not exist.
    ///
    /// Shared the same way as [`Self::open_interference`].
    pub fn open_tec_maps(&self) -> Result<TecMapArchive, IonexStoreError> {
        self.open_tec_maps_with_recovery_choice(InterruptedDeleteRecovery::Recover)
    }

    /// The TEC map archive, recovering an interrupted delete only when
    /// `recovery` asks for it.
    pub fn open_tec_maps_with_recovery_choice(
        &self,
        recovery: InterruptedDeleteRecovery,
    ) -> Result<TecMapArchive, IonexStoreError> {
        self.tec_maps.get_or_open(|| {
            IonexStore::open_or_create_with_recovery_choice(&self.tec_maps_path(), recovery)
                .map(ArchiveHandle::owner)
        })
    }

    /// The TEC map archive as a read-only session opens it, without writing to
    /// it.
    pub fn open_tec_maps_read_only(&self) -> Result<TecMapArchive, IonexStoreError> {
        self.tec_maps.get_or_open(|| {
            ReadOnlyIonexStore::open_existing_read_only(&self.tec_maps_path())
                .map(ArchiveHandle::read_only)
        })
    }

    /// The solar flare archive, creating it if it does not exist.
    ///
    /// Shared the same way as [`Self::open_interference`].
    pub fn open_solar_flares(&self) -> Result<SolarFlareArchive, FlareStoreError> {
        self.open_solar_flares_with_recovery_choice(InterruptedDeleteRecovery::Recover)
    }

    /// The solar flare archive, recovering an interrupted delete only when
    /// `recovery` asks for it.
    pub fn open_solar_flares_with_recovery_choice(
        &self,
        recovery: InterruptedDeleteRecovery,
    ) -> Result<SolarFlareArchive, FlareStoreError> {
        self.solar_flares.get_or_open(|| {
            FlareStore::open_or_create_with_recovery_choice(&self.solar_flares_path(), recovery)
                .map(ArchiveHandle::owner)
        })
    }

    /// The solar flare archive as a read-only session opens it, without
    /// writing to it.
    pub fn open_solar_flares_read_only(&self) -> Result<SolarFlareArchive, FlareStoreError> {
        self.solar_flares.get_or_open(|| {
            ReadOnlyFlareStore::open_existing_read_only(&self.solar_flares_path())
                .map(ArchiveHandle::read_only)
        })
    }
}

/// An archive opened on first use and shared from then on: a later open
/// returns the handle the first one produced, writable or read-only.
///
/// Only a successful open is kept: a failed one leaves the slot empty, so the
/// next caller opens again.
#[derive(Debug)]
struct SharedArchive<W, R>(Mutex<Option<ArchiveHandle<W, R>>>);

impl<W, R> SharedArchive<W, R> {
    fn empty() -> Self {
        Self(Mutex::new(None))
    }

    fn get_or_open<E>(
        &self,
        open: impl FnOnce() -> Result<ArchiveHandle<W, R>, E>,
    ) -> Result<ArchiveHandle<W, R>, E> {
        let mut shared = self.0.lock();
        if let Some(archive) = shared.as_ref() {
            return Ok(archive.clone());
        }
        Ok(shared.insert(open()?).clone())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::ptr;

    use gt_pending_writes::PendingWrites;

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

    /// The store's logs directory and the history database's own derivation
    /// of it name the same directory. The database deletes an attachment's
    /// log when its recording goes.
    #[test]
    fn attached_logs_sit_in_one_directory_beside_the_recording_history() {
        let (dir, store) = store();

        assert_eq!(store.logs_path().parent(), Some(dir.path()));
        assert_eq!(
            store.logs_path(),
            gt_history::logs_directory_for_database(&store.recordings_path())
        );
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

        assert!(ptr::eq(
            interference.read(),
            store
                .open_interference()
                .expect("interference again")
                .read()
        ));
        assert!(ptr::eq(
            indices.read(),
            store
                .open_geomagnetic_indices()
                .expect("geomagnetic indices again")
                .read()
        ));

        let maps = store.open_tec_maps().expect("tec maps");
        assert!(ptr::eq(
            maps.read(),
            store.open_tec_maps().expect("tec maps again").read()
        ));

        let flares = store.open_solar_flares().expect("solar flares");
        assert!(ptr::eq(
            flares.read(),
            store
                .open_solar_flares()
                .expect("solar flares again")
                .read()
        ));
    }

    /// [`ArchiveHandle::writer`] is [`None`] for every archive a read-only
    /// session opened, and [`Some`] for every archive its owner opened.
    #[test]
    fn a_read_only_open_hands_out_no_writer() {
        let (_dir, store) = store();
        store.open_interference().expect("interference");
        store
            .open_geomagnetic_indices()
            .expect("geomagnetic indices");
        store.open_tec_maps().expect("tec maps");
        store.open_solar_flares().expect("solar flares");
        let read_only = Store::open_in(store.root());

        assert!(
            read_only
                .open_interference_read_only()
                .expect("interference")
                .writer(&PendingWrites::default())
                .is_none()
        );
        assert!(
            read_only
                .open_geomagnetic_indices_read_only()
                .expect("geomagnetic indices")
                .writer(&PendingWrites::default())
                .is_none()
        );
        assert!(
            read_only
                .open_tec_maps_read_only()
                .expect("tec maps")
                .writer(&PendingWrites::default())
                .is_none()
        );
        assert!(
            read_only
                .open_solar_flares_read_only()
                .expect("solar flares")
                .writer(&PendingWrites::default())
                .is_none()
        );
        assert!(
            store
                .open_interference()
                .expect("interference")
                .writer(&PendingWrites::default())
                .is_some()
        );
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

    /// A read-only session leaves a data directory without a recording
    /// history exactly as it found it.
    #[test]
    fn a_read_only_open_creates_no_recording_history() {
        let (_dir, store) = store();

        let opened = store
            .open_recordings_read_only()
            .expect("a missing database is not a failure");

        assert!(opened.is_none());
        assert!(!store.recordings_path().exists());
    }

    /// A read-only session lists what the owning instance stored and leaves
    /// the database file byte for byte as it found it.
    #[test]
    fn a_read_only_open_reads_an_existing_recording_history_without_writing_to_it() {
        let (_dir, store) = store();
        store.open_recordings().expect("create the database");
        let before = std::fs::read(store.recordings_path()).expect("read the database");

        let opened = store
            .open_recordings_read_only()
            .expect("open the database")
            .expect("the database is there");

        assert_eq!(opened.list_recordings().expect("list").len(), 0);
        assert_eq!(
            std::fs::read(store.recordings_path()).expect("read the database"),
            before,
            "the read-only open changed the recording history"
        );
    }

    /// A read-only session opens every archive without writing to any of
    /// them.
    ///
    /// The file a rebuild fills stands in for the writes an open would make:
    /// an open that creates, rebuilds or repairs removes it before rebuilding
    /// again, and a read-only one leaves it where it is.
    #[test]
    fn a_read_only_open_writes_to_no_archive() {
        let (_dir, store) = store();
        store.open_interference().expect("interference");
        store
            .open_geomagnetic_indices()
            .expect("geomagnetic indices");
        store.open_tec_maps().expect("tec maps");
        store.open_solar_flares().expect("solar flares");
        let archives = [
            store.interference_path(),
            store.geomagnetic_indices_path(),
            store.tec_maps_path(),
            store.solar_flares_path(),
        ];
        let rebuilding: Vec<PathBuf> = archives
            .iter()
            .map(|path| {
                let rebuilding = gt_hdf5_archive::ArchiveFile::new(path).rebuilding_path();
                std::fs::write(&rebuilding, b"an interrupted rebuild").expect("write");
                rebuilding
            })
            .collect();
        let before: Vec<Vec<u8>> = archives
            .iter()
            .map(|path| std::fs::read(path).expect("read the archive"))
            .collect();
        let read_only = Store::open_in(store.root());

        read_only
            .open_interference_read_only()
            .expect("interference");
        read_only
            .open_geomagnetic_indices_read_only()
            .expect("geomagnetic indices");
        read_only.open_tec_maps_read_only().expect("tec maps");
        read_only
            .open_solar_flares_read_only()
            .expect("solar flares");

        for (path, before) in archives.iter().zip(before) {
            assert_eq!(
                std::fs::read(path).expect("read the archive"),
                before,
                "the read-only open changed {}",
                path.display()
            );
        }
        for path in rebuilding {
            assert!(
                path.exists(),
                "the read-only open removed {}",
                path.display()
            );
        }
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
