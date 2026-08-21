//! Opening the app's on-disk databases.
//!
//! Which databases a run gets is the application's choice, made once at
//! startup and carried in [`super::StartupOptions`]:
//! [`Storage::DataDirectory`] for the user's own databases,
//! [`Storage::Disabled`] for a run that stores nothing. Tests pick the
//! second, so no test reaches the user's recordings or interference
//! archive.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::Context;
use gt_pending_writes::PendingWrites;
use gt_store::{
    DbError, FlareStore, HistoryDatabase as _, IonexStore, JamStore, Recordings, SolarStore, Store,
};

use super::history_db::HistoryWorker;

/// Where a run's databases live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// The databases under the user's data directory, resolved at open
    /// time.
    DataDirectory,
    /// No databases. Nothing is read or written.
    Disabled,
}

/// Why the recordings database is unavailable, and so which prompt the user
/// gets. Each carries the database's path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryFailure {
    /// Another process holds the file. Nothing to repair.
    Busy(PathBuf),
    /// Marked open for write by a writer that did not shut down cleanly. The
    /// user can clear the flag.
    Locked(PathBuf),
    /// The file cannot be read. The user can recreate it.
    Unreadable(PathBuf),
}

impl HistoryFailure {
    /// The database this failure concerns.
    pub fn path(&self) -> &Path {
        match self {
            Self::Busy(path) | Self::Locked(path) | Self::Unreadable(path) => path,
        }
    }
}

/// The databases a run holds, and what went wrong opening them.
pub struct OpenStorage {
    pub history: HistoryWorker,
    /// Set when the recordings database could not be opened.
    pub history_failure: Option<HistoryFailure>,
    /// [`None`] disables interference fetching and nothing else.
    pub archive: Option<Arc<JamStore>>,
    /// [`None`] disables geomagnetic index fetching and nothing else.
    pub geomagnetic_indices: Option<Arc<SolarStore>>,
    /// [`None`] disables TEC map fetching and nothing else.
    pub tec_maps: Option<Arc<IonexStore>>,
    /// [`None`] disables solar flare fetching and nothing else.
    pub solar_flares: Option<Arc<FlareStore>>,
}

impl OpenStorage {
    /// Every database absent, with nothing to report.
    fn disabled() -> Self {
        Self {
            history: HistoryWorker::disabled(),
            history_failure: None,
            archive: None,
            geomagnetic_indices: None,
            tec_maps: None,
            solar_flares: None,
        }
    }
}

impl Storage {
    pub fn open(self, ctx: &Context, pending_writes: PendingWrites) -> OpenStorage {
        match self {
            Self::Disabled => OpenStorage::disabled(),
            Self::DataDirectory => match Store::open_default() {
                Ok(store) => open_in(&store, ctx, pending_writes),
                Err(err) => {
                    log::error!("Failed to locate the data directory: {err}");
                    OpenStorage::disabled()
                }
            },
        }
    }
}

/// Which prompt a failed recordings open turns into.
///
/// [`DbError::Busy`] and [`DbError::WriteLocked`] both need conditions that
/// are awkward to reach from a test, so the routing is a function of its own.
fn classify_failure(err: &DbError, path: PathBuf) -> HistoryFailure {
    match err {
        DbError::Busy => {
            log::warn!(
                "History database at {} is open in another process",
                path.display()
            );
            HistoryFailure::Busy(path)
        }
        DbError::WriteLocked => {
            log::warn!(
                "History database at {} is locked (marked open for write)",
                path.display()
            );
            HistoryFailure::Locked(path)
        }
        _ => {
            log::error!("History database at {} is unusable: {err}", path.display());
            HistoryFailure::Unreadable(path)
        }
    }
}

/// Open the recordings database again, classifying a failure the same way
/// the startup open does.
pub(crate) fn reopen_recordings(path: &Path) -> Result<Recordings, HistoryFailure> {
    Recordings::open_or_create(path).map_err(|err| classify_failure(&err, path.to_owned()))
}

/// Open every database under `store`.
///
/// Each archive is opened whatever the recordings database did: one being
/// unusable says nothing about the others.
fn open_in(store: &Store, ctx: &Context, pending_writes: PendingWrites) -> OpenStorage {
    let (history, history_failure) = match store.open_recordings() {
        Ok(db) => (HistoryWorker::spawn(db, ctx.clone(), pending_writes), None),
        Err(err) => (
            HistoryWorker::disabled(),
            Some(classify_failure(&err, store.recordings_path())),
        ),
    };

    let archive = store
        .open_interference()
        .inspect_err(|err| {
            log::error!(
                "Interference archive at {} is unusable: {err}",
                store.interference_path().display()
            );
        })
        .ok();

    let geomagnetic_indices = store
        .open_geomagnetic_indices()
        .inspect_err(|err| {
            log::error!(
                "Geomagnetic index archive at {} is unusable: {err}",
                store.geomagnetic_indices_path().display()
            );
        })
        .ok();

    let tec_maps = store
        .open_tec_maps()
        .inspect_err(|err| {
            log::error!(
                "TEC map archive at {} is unusable: {err}",
                store.tec_maps_path().display()
            );
        })
        .ok();

    let solar_flares = store
        .open_solar_flares()
        .inspect_err(|err| {
            log::error!(
                "Solar flare archive at {} is unusable: {err}",
                store.solar_flares_path().display()
            );
        })
        .ok();

    OpenStorage {
        history,
        history_failure,
        archive,
        geomagnetic_indices,
        tec_maps,
        solar_flares,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;

    fn db_path() -> PathBuf {
        PathBuf::from("/tmp/recordings.h5")
    }

    /// A store under a throwaway root, so the user's data directory is never
    /// the one being opened.
    fn store() -> (TempDir, Store) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Store::open_in(dir.path());
        (dir, store)
    }

    /// Nothing is opened, so no test can reach the user's data directory
    /// through the app's own startup path.
    #[test]
    fn disabled_storage_opens_nothing() {
        let opened = Storage::Disabled.open(&Context::default(), PendingWrites::default());
        assert!(opened.history.path().is_none());
        assert!(opened.history_failure.is_none());
        assert!(opened.archive.is_none());
        assert!(opened.geomagnetic_indices.is_none());
        assert!(opened.tec_maps.is_none());
        assert!(opened.solar_flares.is_none());
    }

    #[test]
    fn a_usable_store_opens_every_database() {
        let (_dir, store) = store();
        let opened = open_in(&store, &Context::default(), PendingWrites::default());

        assert!(opened.history.path().is_some(), "the worker has a database");
        assert!(opened.archive.is_some());
        assert!(opened.geomagnetic_indices.is_some());
        assert!(opened.tec_maps.is_some());
        assert!(opened.solar_flares.is_some());
        assert!(opened.history_failure.is_none());
    }

    #[rstest]
    #[case::busy(DbError::Busy, HistoryFailure::Busy(db_path()))]
    #[case::locked(DbError::WriteLocked, HistoryFailure::Locked(db_path()))]
    #[case::unreadable(
        DbError::Backend("cannot open file".to_owned()),
        HistoryFailure::Unreadable(db_path())
    )]
    #[case::schema_too_new(
        DbError::SchemaTooNew { found: 9, supported: 2 },
        HistoryFailure::Unreadable(db_path())
    )]
    fn each_open_failure_reaches_its_own_prompt(
        #[case] err: DbError,
        #[case] expected: HistoryFailure,
    ) {
        let failure = classify_failure(&err, db_path());

        assert_eq!(failure, expected);
        assert_eq!(failure.path(), db_path(), "the path is carried through");
    }

    /// One database being unusable says nothing about the other, so the
    /// archive still opens and the recordings path reaches the UI.
    #[test]
    fn a_broken_recordings_database_leaves_the_archive_open() {
        let (_dir, store) = store();
        std::fs::write(store.recordings_path(), b"not a database").expect("write");

        let opened = open_in(&store, &Context::default(), PendingWrites::default());

        assert_eq!(
            opened.history_failure,
            Some(HistoryFailure::Unreadable(store.recordings_path())),
            "the unreadable database is reported for the UI"
        );
        assert!(opened.history.path().is_none());
        assert!(opened.archive.is_some(), "the archives are unaffected");
        assert!(opened.geomagnetic_indices.is_some());
        assert!(opened.tec_maps.is_some());
        assert!(opened.solar_flares.is_some());
    }
}
