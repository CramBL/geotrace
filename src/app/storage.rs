//! Opening the app's on-disk databases.
//!
//! Which databases a run gets is the application's choice, made once at
//! startup and carried in [`super::StartupOptions`]: [`Storage::Default`]
//! for the user's data directory, [`Storage::Disabled`] for a run that
//! stores nothing. Tests pick the second, so no test reaches the user's
//! recordings or interference archive.

use std::path::PathBuf;

use egui::Context;
use gt_store::{DbError, JamStore, Store};

use super::history_db::HistoryWorker;

/// Where a run's databases live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    /// The user's data directory, resolved at open time.
    Default,
    /// No databases. Nothing is read or written.
    Disabled,
}

/// The databases a run holds, and what went wrong opening them.
pub struct OpenStorage {
    pub history: HistoryWorker,
    /// The recordings database is marked open for write. The UI offers to
    /// clear the lock.
    pub pending_history_unlock: Option<PathBuf>,
    /// The recordings database could not be read.
    pub pending_db_corruption: Option<PathBuf>,
    /// [`None`] disables interference fetching and nothing else.
    pub archive: Option<JamStore>,
}

impl OpenStorage {
    /// Every database absent, with nothing to report.
    fn disabled() -> Self {
        Self {
            history: HistoryWorker::disabled(),
            pending_history_unlock: None,
            pending_db_corruption: None,
            archive: None,
        }
    }
}

impl Storage {
    pub fn open(self, ctx: &Context) -> OpenStorage {
        match self {
            Self::Disabled => OpenStorage::disabled(),
            Self::Default => match Store::open_default() {
                Ok(store) => open_in(&store, ctx),
                Err(err) => {
                    log::error!("Failed to locate the data directory: {err}");
                    OpenStorage::disabled()
                }
            },
        }
    }
}

/// Which prompt a failed recordings open turns into: a lock the user can
/// clear, or a database that cannot be read at all.
///
/// [`DbError::WriteLocked`] needs HDF5's post-crash consistency flags to
/// reach, so the routing is a function of its own to keep it testable.
fn history_failure(err: &DbError, path: PathBuf) -> (Option<PathBuf>, Option<PathBuf>) {
    match err {
        DbError::WriteLocked => {
            log::warn!(
                "History database at {} is locked (marked open for write)",
                path.display()
            );
            (Some(path), None)
        }
        _ => {
            log::error!("History database at {} is unusable: {err}", path.display());
            (None, Some(path))
        }
    }
}

/// Open both databases under `store`.
///
/// The archive is opened whatever the recordings database did: one being
/// unusable says nothing about the other.
fn open_in(store: &Store, ctx: &Context) -> OpenStorage {
    let (history, pending_history_unlock, pending_db_corruption) = match store.open_recordings() {
        Ok(db) => (HistoryWorker::spawn(db, ctx.clone()), None, None),
        Err(err) => {
            let (unlock, corruption) = history_failure(&err, store.recordings_path());
            (HistoryWorker::disabled(), unlock, corruption)
        }
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

    OpenStorage {
        history,
        pending_history_unlock,
        pending_db_corruption,
        archive,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;

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
        let opened = Storage::Disabled.open(&Context::default());
        assert!(opened.history.path().is_none());
        assert!(opened.pending_history_unlock.is_none());
        assert!(opened.pending_db_corruption.is_none());
        assert!(opened.archive.is_none());
    }

    #[test]
    fn a_usable_store_opens_both_databases() {
        let (_dir, store) = store();
        let opened = open_in(&store, &Context::default());

        assert!(opened.history.path().is_some(), "the worker has a database");
        assert!(opened.archive.is_some());
        assert!(opened.pending_history_unlock.is_none());
        assert!(opened.pending_db_corruption.is_none());
    }

    #[rstest]
    #[case::locked(DbError::WriteLocked, true)]
    #[case::unreadable(DbError::Backend("cannot open file".to_owned()), false)]
    #[case::schema_too_new(DbError::SchemaTooNew { found: 9, supported: 2 }, false)]
    fn a_failed_open_reports_a_lock_or_a_broken_database(
        #[case] err: DbError,
        #[case] recoverable: bool,
    ) {
        let path = PathBuf::from("/tmp/recordings.h5");
        let (unlock, corruption) = history_failure(&err, path.clone());

        assert_eq!(unlock.is_some(), recoverable);
        assert_eq!(corruption.is_some(), !recoverable);
        assert_eq!(unlock.or(corruption), Some(path), "the path is reported");
    }

    /// One database being unusable says nothing about the other, so the
    /// archive still opens and the recordings path reaches the UI.
    #[test]
    fn a_broken_recordings_database_leaves_the_archive_open() {
        let (_dir, store) = store();
        std::fs::write(store.recordings_path(), b"not a database").expect("write");

        let opened = open_in(&store, &Context::default());

        assert_eq!(
            opened.pending_db_corruption,
            Some(store.recordings_path()),
            "the unreadable database is reported for the UI"
        );
        assert!(opened.history.path().is_none());
        assert!(opened.pending_history_unlock.is_none());
        assert!(opened.archive.is_some(), "the archive is unaffected");
    }
}
