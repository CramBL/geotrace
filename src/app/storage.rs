//! Opening the app's on-disk databases.
//!
//! Which databases a run gets is the application's choice, made once at
//! startup and carried in [`super::StartupOptions`]:
//! [`Storage::DataDirectory`] for the user's own databases,
//! [`Storage::Disabled`] for a run that stores nothing. Tests pick the
//! second, so no test reaches the user's recordings or interference
//! archive.
//!
//! The open runs on a thread of its own, reporting over an mpsc channel and
//! repainting when it lands, as [`super::jamming`] describes. The window is
//! painted from the first frame, and [`App::adopt_finished_storage_open`]
//! installs the databases in whichever frame they arrive.

use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;

use egui::Context;
use gt_pending_writes::{PendingWrites, WriteKind};
use gt_store::{
    DbError, FlareStore, HistoryDatabase as _, IonexStore, JamStore, Recordings, SolarStore, Store,
};

use super::App;
use super::history_db::HistoryWorker;

/// What the open is called wherever it is shown: the loading overlay while it
/// runs, and the shutdown window when a close waits for it.
pub(in crate::app) const OPENING_DATABASES: &str = "Opening the databases";

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

/// A file load the app took before the databases landed, held until they do.
///
/// A recording loaded without them would not be stored, and its tracks would
/// resolve against no archive and cache that they have no environment data.
pub(in crate::app) enum QueuedLoad {
    Path(PathBuf),
    /// A drop or a paste, which carries the bytes themselves.
    Bytes {
        bytes: Arc<[u8]>,
        name: String,
    },
    /// Log text pasted into the window.
    PastedText(String),
}

/// How far the startup open has got.
pub(in crate::app) enum StorageOpen {
    Opening {
        opened: mpsc::Receiver<OpenStorage>,
        queued_loads: Vec<QueuedLoad>,
    },
    /// The open landed: its databases were adopted, or dropped because the
    /// app was already closing.
    Finished,
}

impl StorageOpen {
    /// Whether the databases are still opening, which grays the controls that
    /// need them.
    pub(in crate::app) fn is_opening(&self) -> bool {
        matches!(self, Self::Opening { .. })
    }

    /// The loads waiting on the databases, or [`None`] once they have landed
    /// and a load can go straight to the loader.
    pub(in crate::app) fn queued_loads_mut(&mut self) -> Option<&mut Vec<QueuedLoad>> {
        match self {
            Self::Opening { queued_loads, .. } => Some(queued_loads),
            Self::Finished => None,
        }
    }

    fn opening(opened: mpsc::Receiver<OpenStorage>) -> Self {
        Self::Opening {
            opened,
            queued_loads: Vec::new(),
        }
    }

    /// Hands the open to a test, which lands the databases itself through the
    /// sender it gets back. What is already queued stays queued.
    #[cfg(test)]
    pub(in crate::app) fn take_over_for_test(&mut self) -> mpsc::Sender<OpenStorage> {
        let (sender, opened) = mpsc::channel();
        let queued_loads = self.queued_loads_mut().map(mem::take).unwrap_or_default();
        *self = Self::Opening {
            opened,
            queued_loads,
        };
        sender
    }
}

impl Storage {
    /// Open every database on a thread of its own, repainting when the result
    /// is ready for [`App::adopt_finished_storage_open`] to take.
    ///
    /// [`Self::Disabled`] opens nothing, so its result is in the channel
    /// before the first frame and no thread is spawned.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub(in crate::app) fn open_in_background(
        self,
        ctx: &Context,
        pending_writes: PendingWrites,
    ) -> StorageOpen {
        let (sender, opened) = mpsc::channel();
        match self {
            Self::Disabled => {
                sender.send(OpenStorage::disabled()).ok();
            }
            Self::DataDirectory => {
                let open_write =
                    pending_writes.try_begin(OPENING_DATABASES, WriteKind::DatabaseOpen);
                let ctx = ctx.clone();
                thread::Builder::new()
                    .name("storage-open".to_owned())
                    .spawn(move || {
                        let storage = Self::DataDirectory.open(&ctx, pending_writes);
                        sender.send(storage).ok();
                        ctx.request_repaint();
                        // Held until the result is in the channel: a close
                        // during startup waits for the repair the open may be
                        // part-way through.
                        drop(open_write);
                    })
                    .expect("failed to spawn the storage-open thread");
            }
        }
        StorageOpen::opening(opened)
    }

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

impl App {
    /// Install the databases a [`Storage::open`] produced.
    ///
    /// The schedulers read their archived days here: nothing may ask them what
    /// they hold before this runs.
    pub(super) fn adopt_open_storage(&mut self, storage: OpenStorage) {
        debug_assert!(
            !self.shutdown.has_begun(),
            "a history worker is never installed once shutdown has ended one"
        );
        let OpenStorage {
            history,
            history_failure,
            archive,
            geomagnetic_indices,
            tec_maps,
            solar_flares,
        } = storage;

        self.install_history_worker(history);
        self.history_failure = history_failure;

        self.jamming.adopt_store(archive);
        self.geomagnetic_indices.adopt_store(geomagnetic_indices);
        self.tec_maps.adopt_store(tec_maps);
        self.solar_flares.adopt_store(solar_flares);
    }

    /// Install the databases in the frame the open lands them, and start what
    /// waited on them.
    ///
    /// A storage that lands after the app began closing is dropped instead:
    /// its recordings database is closed on a thread of its own, so the app
    /// closes with nothing left open.
    pub(in crate::app) fn adopt_finished_storage_open(&mut self) {
        let StorageOpen::Opening {
            opened,
            queued_loads,
        } = &mut self.storage_open
        else {
            return;
        };
        let storage = match opened.try_recv() {
            Ok(storage) => storage,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                log::error!(
                    "The storage-open thread ended without reporting: this run stores nothing"
                );
                self.toasts
                    .error("The databases could not be opened: nothing is stored this run");
                OpenStorage::disabled()
            }
        };
        let queued_loads = mem::take(queued_loads);
        self.storage_open = StorageOpen::Finished;

        if self.shutdown.has_begun() {
            self.end_history_worker_off_the_gui_thread(storage.history);
            return;
        }

        debug_assert!(
            self.loader.loading_jobs.is_empty(),
            "a file is loaded only once the databases have been adopted"
        );
        self.adopt_open_storage(storage);
        self.auto_prune_environment_days();
        for load in queued_loads {
            match load {
                QueuedLoad::Path(path) => self.spawn_load_path(path),
                QueuedLoad::Bytes { bytes, name } => self.handle_dropped_bytes(bytes, &name),
                QueuedLoad::PastedText(text) => self.loader.spawn_pasted_log_text(text),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Utc};
    use gt_fetch::TransportSource;
    use gt_ionex::IonexProduct;
    use gt_solar::series::KpSeries;
    use gt_test_utils::ionex_fixtures;
    use rstest::rstest;
    use tempfile::TempDir;

    use crate::app::environment_storage::PrunedDays;
    use crate::app::{flares, jamming, solar, tec};

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

    /// Opening nothing needs no thread: the result is in the channel before
    /// the first frame polls for it.
    #[test]
    fn a_disabled_storage_lands_before_the_first_poll() {
        let open =
            Storage::Disabled.open_in_background(&Context::default(), PendingWrites::default());

        let StorageOpen::Opening { opened, .. } = &open else {
            panic!("the open is what the app starts with");
        };
        opened
            .try_recv()
            .expect("the disabled result is already in the channel");
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

    /// Archive `day` in each of the four day archives `opened` holds, through
    /// the handles it already has open.
    fn archive_one_day_in_each(opened: &OpenStorage, day: NaiveDate) {
        let fetched_at = Utc::now();
        opened
            .archive
            .as_ref()
            .expect("interference archive")
            .insert_day(day, "host", fetched_at, &[])
            .expect("insert interference");
        opened
            .geomagnetic_indices
            .as_ref()
            .expect("geomagnetic index archive")
            .insert_or_replace_kp_day(
                day,
                "host",
                fetched_at,
                &KpSeries {
                    samples: Vec::new(),
                },
            )
            .expect("insert geomagnetic indices");
        opened
            .tec_maps
            .as_ref()
            .expect("TEC map archive")
            .insert_or_replace_day(
                day,
                "host",
                fetched_at,
                IonexProduct::Final,
                &ionex_fixtures::uniform_maps(day, &[(0, 10.0)]),
            )
            .expect("insert TEC maps");
        opened
            .solar_flares
            .as_ref()
            .expect("solar flare archive")
            .insert_or_replace_day(day, "host", fetched_at, &[])
            .expect("insert solar flares");
    }

    /// Each scheduler derives the days it holds from its archive's index, and
    /// a scheduler that adopts an archive after construction has to read that
    /// index too - otherwise it would re-download days it already has.
    ///
    /// Every constructor routes through its own `adopt_store`, so this covers
    /// both ways a scheduler takes an archive.
    #[test]
    fn a_scheduler_adopting_an_archive_reports_the_days_it_holds() {
        let (_dir, store) = store();
        let archived = NaiveDate::from_ymd_opt(2026, 7, 20).expect("valid date");
        let opened = open_in(&store, &Context::default(), PendingWrites::default());
        archive_one_day_in_each(&opened, archived);

        let mut jamming = jamming::JammingScheduler::new(
            Context::default(),
            None,
            gt_jam::DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
            PendingWrites::default(),
        );
        jamming.adopt_store(opened.archive.clone());

        let mut geomagnetic_indices = solar::GeomagneticIndexScheduler::new(
            Context::default(),
            None,
            gt_solar::DEFAULT_BASE_URL.to_owned(),
            TransportSource::Offline,
            PendingWrites::default(),
        );
        geomagnetic_indices.adopt_store(opened.geomagnetic_indices.clone());

        let mut tec_maps = tec::TecMapScheduler::new(
            Context::default(),
            None,
            crate::settings::TecSettings::default().mirrors,
            None,
            TransportSource::Offline,
            PendingWrites::default(),
        );
        tec_maps.adopt_store(opened.tec_maps.clone());

        let mut solar_flares = flares::SolarFlareScheduler::new(
            Context::default(),
            None,
            gt_flare::DEFAULT_BASE_URL.to_owned(),
            None,
            TransportSource::Offline,
            PendingWrites::default(),
        );
        solar_flares.adopt_store(opened.solar_flares.clone());

        assert_eq!(
            jamming.archived_days_covered(PrunedDays::All),
            1,
            "interference"
        );
        assert_eq!(
            geomagnetic_indices.archived_days_covered(PrunedDays::All),
            1,
            "geomagnetic indices"
        );
        assert_eq!(
            tec_maps.archived_days_covered(PrunedDays::All),
            1,
            "TEC maps"
        );
        assert_eq!(
            solar_flares.archived_days_covered(PrunedDays::All),
            1,
            "solar flares"
        );
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
