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
//!
//! A run that finds another instance holding the data directory opens
//! nothing until it has the directory, as [`super::instance_wait`]
//! describes.

use std::mem;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use egui::Context;
use gt_instance_lock::{DataDirectoryLock, TakeOverRecord};
use gt_pending_writes::{PendingWrites, WriteAccess, WriteKind};
use gt_store::{
    ArchiveHandle, DayArchiveError, DbError, FlareStore, GeomagneticIndexArchive,
    HistoryDatabase as _, InterferenceArchive, IonexStore, JamStore, Recordings, RecordingsHandle,
    SolarFlareArchive, SolarStore, Store, StoredDayArchive, TecMapArchive,
};

use super::App;
use super::archive_recovery::{
    self, ArchiveOpenPlan, ArchiveRecovery, ArchiveUnavailable, InspectedArchives,
    InterruptedDeletePrompts, UnavailableArchives,
};
use super::background_thread;
#[cfg(test)]
use super::environment_storage;
use super::history_db::HistoryWorker;
use super::instance_wait::DataDirectoryWait;

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
    pub archive: Option<InterferenceArchive>,
    /// [`None`] disables geomagnetic index fetching and nothing else.
    pub geomagnetic_indices: Option<GeomagneticIndexArchive>,
    /// [`None`] disables TEC map fetching and nothing else.
    pub tec_maps: Option<TecMapArchive>,
    /// [`None`] disables solar flare fetching and nothing else.
    pub solar_flares: Option<SolarFlareArchive>,
    /// The archives this run opened nothing of on the user's choice, which
    /// the controls that need them explain themselves with.
    pub unavailable_archives: UnavailableArchives,
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
            unavailable_archives: UnavailableArchives::default(),
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

/// Why the databases are not open yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum DatabasesPending {
    /// This instance does not have the data directory, so the open has not
    /// started.
    WaitingForTheDataDirectory,
    /// The open is running.
    Opening,
    /// The open is waiting for the user's choice about an archive a delete
    /// was interrupted in.
    AwaitingAnInterruptedDeleteChoice,
}

/// How far the startup open has got.
pub(in crate::app) enum StorageOpen {
    /// Another instance holds the data directory, so nothing has been opened
    /// yet. [`App::wait_for_the_data_directory`] starts the open once this
    /// instance takes the directory.
    WaitingForTheDataDirectory {
        wait: DataDirectoryWait,
        queued_loads: Vec<QueuedLoad>,
    },
    /// The first step of an open after the user took write access: the
    /// archives are read for interrupted deletes, and nothing is written or
    /// opened.
    InspectingArchives {
        inspected: mpsc::Receiver<InspectedArchives>,
        queued_loads: Vec<QueuedLoad>,
    },
    /// Asking the user about each archive the inspection found an interrupted
    /// delete in. No write guard is held here: the open waits on a person.
    AskingAboutInterruptedDeletes {
        prompts: InterruptedDeletePrompts,
        queued_loads: Vec<QueuedLoad>,
    },
    Opening {
        opened: mpsc::Receiver<OpenStorage>,
        queued_loads: Vec<QueuedLoad>,
    },
    /// The open landed: its databases were adopted, or dropped because the
    /// app was already closing.
    Finished,
}

impl StorageOpen {
    /// The wait a run starts in when another instance holds the data
    /// directory.
    pub(in crate::app) fn waiting_for_the_data_directory(
        instance_lock: &DataDirectoryLock,
    ) -> Self {
        Self::WaitingForTheDataDirectory {
            wait: DataDirectoryWait::new(instance_lock),
            queued_loads: Vec::new(),
        }
    }

    /// Why the databases are not open yet, which grays the controls that
    /// need them and gives each one its hover text.
    pub(in crate::app) fn databases_pending(&self) -> Option<DatabasesPending> {
        match self {
            Self::WaitingForTheDataDirectory { .. } => {
                Some(DatabasesPending::WaitingForTheDataDirectory)
            }
            Self::AskingAboutInterruptedDeletes { .. } => {
                Some(DatabasesPending::AwaitingAnInterruptedDeleteChoice)
            }
            Self::InspectingArchives { .. } | Self::Opening { .. } => {
                Some(DatabasesPending::Opening)
            }
            Self::Finished => None,
        }
    }

    /// The loads waiting on the databases, or [`None`] once they have landed
    /// and a load can go straight to the loader.
    pub(in crate::app) fn queued_loads_mut(&mut self) -> Option<&mut Vec<QueuedLoad>> {
        match self {
            Self::WaitingForTheDataDirectory { queued_loads, .. }
            | Self::InspectingArchives { queued_loads, .. }
            | Self::AskingAboutInterruptedDeletes { queued_loads, .. }
            | Self::Opening { queued_loads, .. } => Some(queued_loads),
            Self::Finished => None,
        }
    }

    /// Puts the app on the open a take-over runs, with what a test read off
    /// its own archives in place of the background step. What is already
    /// queued stays queued.
    #[cfg(test)]
    pub(in crate::app) fn inspect_archives_for_test(&mut self, inspected: InspectedArchives) {
        let (sender, receiver) = mpsc::channel();
        sender.send(inspected).ok();
        let queued_loads = self.queued_loads_mut().map(mem::take).unwrap_or_default();
        *self = Self::InspectingArchives {
            inspected: receiver,
            queued_loads,
        };
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
    /// Where this run's databases live, and so what it locks against a
    /// second instance. [`None`] for a run that stores nothing, and for one
    /// with no platform data directory - [`Self::root_to_open`] reports why.
    pub fn data_directory(self) -> Option<PathBuf> {
        match self {
            Self::Disabled => None,
            Self::DataDirectory => Store::default_root().ok(),
        }
    }

    /// The directory this run opens its databases under, or [`None`] where it
    /// opens none: a run that stores nothing, and one with no platform data
    /// directory.
    pub(in crate::app) fn root_to_open(self) -> Option<PathBuf> {
        match self {
            Self::Disabled => None,
            Self::DataDirectory => match Store::default_root() {
                Ok(root) => Some(root),
                Err(err) => {
                    log::error!("Failed to locate the data directory: {err}");
                    None
                }
            },
        }
    }

    /// Open every database on a thread of its own, recovering whatever
    /// interrupted delete an archive holds, and repainting when the result is
    /// ready for [`App::adopt_finished_storage_open`] to take. Whatever is in
    /// `queued_loads` runs when it lands.
    ///
    /// A run that opens nothing has its result in the channel before the
    /// first frame, and no thread is spawned.
    pub(in crate::app) fn open_in_background(
        self,
        ctx: &Context,
        pending_writes: PendingWrites,
        queued_loads: Vec<QueuedLoad>,
    ) -> StorageOpen {
        open_in_background_under(
            self.root_to_open(),
            ArchiveRecovery::Automatic,
            ctx,
            pending_writes,
            queued_loads,
        )
    }

    /// Read the archives for interrupted deletes on a thread of its own,
    /// after the user took write access from the instance holding the data
    /// directory.
    ///
    /// This step only reads: what it finds is put to the user, and the choices
    /// start the open itself.
    ///
    /// `previous_take_over` is the take-over recorded in the data directory
    /// before this one, which the prompts state.
    pub(in crate::app) fn inspect_archives_in_background(
        self,
        ctx: &Context,
        previous_take_over: Option<TakeOverRecord>,
        queued_loads: Vec<QueuedLoad>,
    ) -> StorageOpen {
        let (sender, inspected) = mpsc::channel();
        match self.root_to_open() {
            None => {
                sender.send(InspectedArchives::of_nothing()).ok();
            }
            Some(root) => {
                let ctx = ctx.clone();
                background_thread::spawn_or_panic("archive-inspect", move || {
                    sender
                        .send(archive_recovery::inspect_archives_under(
                            root,
                            previous_take_over,
                        ))
                        .ok();
                    ctx.request_repaint();
                });
            }
        }
        StorageOpen::InspectingArchives {
            inspected,
            queued_loads,
        }
    }

    pub fn open(self, ctx: &Context, pending_writes: PendingWrites) -> OpenStorage {
        open_under(
            self.root_to_open(),
            ArchiveRecovery::Automatic,
            ctx,
            pending_writes,
        )
    }
}

/// Open every database under `root` on a thread of its own. `recovery` says
/// what to do with the interrupted deletes it meets. A [`None`] root opens
/// nothing, and its result is in the channel before the first frame.
pub(in crate::app) fn open_in_background_under(
    root: Option<PathBuf>,
    recovery: ArchiveRecovery,
    ctx: &Context,
    pending_writes: PendingWrites,
    queued_loads: Vec<QueuedLoad>,
) -> StorageOpen {
    let (sender, opened) = mpsc::channel();
    match root {
        None => {
            sender.send(OpenStorage::disabled()).ok();
        }
        Some(root) => {
            let open_write = pending_writes
                .try_begin(OPENING_DATABASES, WriteKind::DatabaseOpen)
                .ok();
            let ctx = ctx.clone();
            background_thread::spawn_or_panic("storage-open", move || {
                let storage = open_under(Some(root), recovery, &ctx, pending_writes);
                sender.send(storage).ok();
                ctx.request_repaint();
                // Held until the result is in the channel: a close during
                // startup waits for the repair the open may be part-way
                // through.
                drop(open_write);
            });
        }
    }
    StorageOpen::Opening {
        opened,
        queued_loads,
    }
}

/// Open every database under `root`, or none at all where there is no root.
fn open_under(
    root: Option<PathBuf>,
    recovery: ArchiveRecovery,
    ctx: &Context,
    pending_writes: PendingWrites,
) -> OpenStorage {
    match root {
        None => OpenStorage::disabled(),
        Some(root) => open_in(&Store::open_in(root), ctx, pending_writes, recovery),
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

/// Open every database under `store`. `recovery` says what to do with the
/// interrupted deletes the archives hold.
///
/// Each archive is opened whatever the recordings database did: one being
/// unusable says nothing about the others.
pub(in crate::app) fn open_in(
    store: &Store,
    ctx: &Context,
    pending_writes: PendingWrites,
    recovery: ArchiveRecovery,
) -> OpenStorage {
    let write_access = pending_writes.write_access();
    let (history, history_failure) = match write_access {
        WriteAccess::Owner => match store.open_recordings() {
            Ok(db) => (
                HistoryWorker::spawn(RecordingsHandle::Owner(db), ctx.clone(), pending_writes),
                None,
            ),
            Err(err) => (
                HistoryWorker::disabled(),
                Some(classify_failure(&err, store.recordings_path())),
            ),
        },
        WriteAccess::ReadOnly => (open_recordings_read_only(store, ctx, pending_writes), None),
    };

    let mut unavailable_archives = UnavailableArchives::default();
    let archive =
        open_archive::<JamStore>(store, recovery, write_access, &mut unavailable_archives);
    let geomagnetic_indices =
        open_archive::<SolarStore>(store, recovery, write_access, &mut unavailable_archives);
    let tec_maps =
        open_archive::<IonexStore>(store, recovery, write_access, &mut unavailable_archives);
    let solar_flares =
        open_archive::<FlareStore>(store, recovery, write_access, &mut unavailable_archives);

    OpenStorage {
        history,
        history_failure,
        archive,
        geomagnetic_indices,
        tec_maps,
        solar_flares,
        unavailable_archives,
    }
}

/// The recording history a read-only session reads, or a disabled worker
/// where there is none to read.
///
/// No failure is reported for the user to choose about: every choice the
/// failure prompt offers writes to the database.
fn open_recordings_read_only(
    store: &Store,
    ctx: &Context,
    pending_writes: PendingWrites,
) -> HistoryWorker {
    match store.open_recordings_read_only() {
        Ok(Some(db)) => {
            HistoryWorker::spawn(RecordingsHandle::ReadOnly(db), ctx.clone(), pending_writes)
        }
        Ok(None) => {
            log::info!(
                "There is no recording history at {}, and this read-only session creates none",
                store.recordings_path().display()
            );
            HistoryWorker::disabled()
        }
        Err(err) => {
            log::warn!(
                "This session does not read the recording history at {}: {err}",
                store.recordings_path().display()
            );
            HistoryWorker::disabled()
        }
    }
}

/// Open one archive, recording why the app has no archive where it ends up
/// with none. A read-only session decides for itself, whatever `recovery`
/// plans: it opens only an archive that is already there, and never writes to
/// it.
///
/// An archive the user left as it is, and one another process holds, are both
/// reported to the controls that need them. Anything else is logged and the
/// app carries on without that archive, as it always has.
///
/// The plan and the report both come from `A::ARCHIVE`, so an open cannot be
/// planned as one archive and run against another.
fn open_archive<A: StoredDayArchive>(
    store: &Store,
    recovery: ArchiveRecovery,
    write_access: WriteAccess,
    unavailable_archives: &mut UnavailableArchives,
) -> Option<ArchiveHandle<A, A::ReadOnly>> {
    let archive = A::ARCHIVE;
    let plan = match write_access {
        WriteAccess::Owner => recovery.plan_for(archive),
        WriteAccess::ReadOnly => ArchiveOpenPlan::in_a_read_only_session(archive, store),
    };
    let opened = match plan {
        ArchiveOpenPlan::LeaveClosed(reason) => {
            unavailable_archives[archive] = Some(reason);
            return None;
        }
        ArchiveOpenPlan::Open(choice) => {
            store.open_or_create_archive_with_recovery_choice::<A>(choice)
        }
        ArchiveOpenPlan::OpenReadOnly => store.open_existing_archive_read_only::<A>(),
    };
    match opened {
        Ok(opened) => Some(opened),
        Err(err) => {
            if let Some(interrupted) = err.interrupted_delete_left_unrecovered() {
                log::warn!(
                    "The {} archive keeps the {} archived days its interrupted delete left, and \
                     is not opened this session",
                    archive.label_in_sentence(),
                    interrupted.archived_days
                );
                unavailable_archives[archive] =
                    Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered);
            } else if err.is_held_by_another_process() {
                log::warn!(
                    "The {} archive is open in another process, and is not opened this session",
                    archive.label_in_sentence()
                );
                unavailable_archives[archive] = Some(ArchiveUnavailable::HeldByTheOtherInstance);
            } else {
                log::error!(
                    "The {} archive at {} is unusable: {err}",
                    archive.label_in_sentence(),
                    archive.path_in(store).display()
                );
            }
            None
        }
    }
}

impl App {
    /// Install the databases a [`Storage::open`] produced.
    ///
    /// The schedulers read their archived days here: nothing may read what
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
            unavailable_archives,
        } = storage;

        self.install_history_worker(history);
        self.history_failure = history_failure;
        self.unavailable_archives = unavailable_archives;

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
        self.load_arriving_files(queued_loads);
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
    use strum::IntoEnumIterator as _;
    use tempfile::TempDir;

    use gt_jam_store::schema;
    use gt_store::{EnvironmentArchive, ReadOnlyDayArchive as _, ReadOnlyJamStore};
    use gt_test_utils::day_archive::{self, GroupPath};

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
        let open = Storage::Disabled.open_in_background(
            &Context::default(),
            PendingWrites::default(),
            Vec::new(),
        );

        let StorageOpen::Opening { opened, .. } = &open else {
            panic!("the open is what the app starts with");
        };
        opened
            .try_recv()
            .expect("the disabled result is already in the channel");
    }

    /// A run that stores nothing has no directory to lock. One that stores
    /// locks the very directory its databases are opened under.
    #[test]
    fn only_a_storing_run_has_a_data_directory_to_lock() {
        assert_eq!(Storage::Disabled.data_directory(), None);
        assert_eq!(
            Storage::DataDirectory.data_directory(),
            Store::default_root().ok()
        );
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
        let opened = open_in(
            &store,
            &Context::default(),
            PendingWrites::default(),
            ArchiveRecovery::Automatic,
        );

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

    /// An interference archive under `root` holding two days, with a delete
    /// marked part-way through it as an interrupted one leaves it.
    fn interrupt_a_delete_in_the_interference_archive(root: &Path) {
        let store = Store::open_in(root);
        let path = store.archive_path::<JamStore>();
        {
            let archive = store
                .open_or_create_archive::<JamStore>()
                .expect("interference archive")
                .writer(&PendingWrites::default())
                .expect("an owner session opens the archive writable");
            for offset in 0..2 {
                let day = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap_or_default()
                    + chrono::TimeDelta::days(offset);
                archive
                    .write(
                        EnvironmentArchive::AircraftInterference.day_insert_registration(day),
                        |interference| interference.insert_day(day, "host", Utc::now(), &[]),
                    )
                    .expect("the registry takes the write")
                    .expect("insert interference");
            }
        }
        drop(store);
        day_archive::mark_delete_in_flight(&path, GroupPath(schema::DAYS_GROUP))
            .expect("mark the delete");
    }

    /// A run that has the data directory to itself prompts for nothing: what
    /// an instance that is gone left interrupted is recovered as the archive
    /// opens, which discards the days it holds.
    #[test]
    fn the_normal_open_recovers_an_interrupted_delete_without_asking() {
        let dir = tempfile::tempdir().expect("temp dir");
        interrupt_a_delete_in_the_interference_archive(dir.path());
        let store = Store::open_in(dir.path());

        let opened = open_in(
            &store,
            &Context::default(),
            PendingWrites::default(),
            ArchiveRecovery::Automatic,
        );

        let archive = opened.archive.expect("the recovered archive is open");
        assert_eq!(archive.read().days().expect("read the archive index"), []);
        assert_eq!(
            ReadOnlyJamStore::interrupted_delete_at(&store.archive_path::<JamStore>())
                .expect("read the archive"),
            None,
            "the open left the delete interrupted"
        );
        assert_eq!(
            opened.unavailable_archives[EnvironmentArchive::AircraftInterference],
            None
        );
    }

    /// A read-only session leaves a data directory it finds nothing in
    /// exactly as it found it, and each control that needs an archive states
    /// why it has none.
    #[test]
    fn a_read_only_open_creates_no_database_and_no_archive() {
        let (dir, store) = store();

        let opened = open_in(
            &store,
            &Context::default(),
            PendingWrites::new(WriteAccess::ReadOnly),
            ArchiveRecovery::Automatic,
        );

        assert!(opened.history.path().is_none());
        assert_eq!(opened.history_failure, None);
        assert!(opened.archive.is_none());
        assert!(opened.geomagnetic_indices.is_none());
        assert!(opened.tec_maps.is_none());
        assert!(opened.solar_flares.is_none());
        for archive in EnvironmentArchive::iter() {
            assert_eq!(
                opened.unavailable_archives[archive],
                Some(ArchiveUnavailable::MissingInAReadOnlySession),
                "{archive:?} was reported as unavailable for the wrong reason"
            );
        }
        assert_eq!(
            std::fs::read_dir(dir.path())
                .expect("read the data directory")
                .count(),
            0,
            "the read-only session put a file in the data directory"
        );
    }

    /// Recovering an interrupted delete rewrites the archive, which is the one
    /// thing a read-only session may not do: it leaves the file as it is and
    /// goes without that archive.
    #[test]
    fn a_read_only_open_leaves_an_interrupted_delete_unrecovered() {
        let dir = tempfile::tempdir().expect("temp dir");
        interrupt_a_delete_in_the_interference_archive(dir.path());
        let store = Store::open_in(dir.path());

        let opened = open_in(
            &store,
            &Context::default(),
            PendingWrites::new(WriteAccess::ReadOnly),
            ArchiveRecovery::Automatic,
        );

        assert!(opened.archive.is_none());
        assert_eq!(
            opened.unavailable_archives[EnvironmentArchive::AircraftInterference],
            Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
        );
        assert_eq!(
            ReadOnlyJamStore::interrupted_delete_at(&store.archive_path::<JamStore>())
                .expect("read the archive")
                .map(|interrupted| interrupted.archived_days),
            Some(2),
            "the read-only session recovered the delete"
        );
    }

    /// The archives a read-only session finds are read: only writing to them
    /// is off.
    ///
    /// The session opens through a [`Store`] of its own, as it does beside
    /// the instance that created the archive.
    #[test]
    fn a_read_only_open_reads_the_archives_that_are_already_there() {
        let (_dir, store) = store();
        store
            .open_or_create_archive::<JamStore>()
            .expect("create the archive");

        let opened = open_in(
            &Store::open_in(store.root()),
            &Context::default(),
            PendingWrites::new(WriteAccess::ReadOnly),
            ArchiveRecovery::Automatic,
        );

        let archive = opened.archive.expect("the archive is open for reading");
        assert!(
            archive.writer(&PendingWrites::default()).is_none(),
            "a read-only session holds the archive without its mutators"
        );
        assert_eq!(archive.read().days().expect("read the day index"), []);
        assert_eq!(
            opened.unavailable_archives[EnvironmentArchive::AircraftInterference],
            None
        );
    }

    /// Archive `day` in each of the four day archives `opened` holds, through
    /// the handles it already has open.
    fn archive_one_day_in_each(opened: &OpenStorage, day: NaiveDate) {
        let fetched_at = Utc::now();
        environment_storage::archive_one_day(
            opened.archive.as_ref().expect("interference archive"),
            EnvironmentArchive::AircraftInterference.day_insert_registration(day),
            |interference| interference.insert_day(day, "host", fetched_at, &[]),
        );
        environment_storage::archive_one_day(
            opened
                .geomagnetic_indices
                .as_ref()
                .expect("geomagnetic index archive"),
            EnvironmentArchive::GeomagneticIndices.day_insert_registration(day),
            |indices| {
                indices.insert_or_replace_kp_day(
                    day,
                    "host",
                    fetched_at,
                    &KpSeries {
                        samples: Vec::new(),
                    },
                )
            },
        );
        environment_storage::archive_one_day(
            opened.tec_maps.as_ref().expect("TEC map archive"),
            EnvironmentArchive::IonosphericTec.day_insert_registration(day),
            |maps| {
                maps.insert_or_replace_day(
                    day,
                    "host",
                    fetched_at,
                    IonexProduct::Final,
                    &ionex_fixtures::uniform_maps(day, &[(0, 10.0)]),
                )
            },
        );
        environment_storage::archive_one_day(
            opened.solar_flares.as_ref().expect("solar flare archive"),
            EnvironmentArchive::SolarFlares.day_insert_registration(day),
            |flares| flares.insert_or_replace_day(day, "host", fetched_at, &[]),
        );
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
        let opened = open_in(
            &store,
            &Context::default(),
            PendingWrites::default(),
            ArchiveRecovery::Automatic,
        );
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

        let opened = open_in(
            &store,
            &Context::default(),
            PendingWrites::default(),
            ArchiveRecovery::Automatic,
        );

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

    /// A read-only session offers no remedy for an unreadable database - every
    /// one of them writes - so it keeps the failure out of the prompts and
    /// simply runs without the recordings.
    #[test]
    fn a_broken_recordings_database_raises_no_prompt_in_a_read_only_session() {
        let (_dir, store) = store();
        std::fs::write(store.recordings_path(), b"not a database").expect("write");

        let opened = open_in(
            &store,
            &Context::default(),
            PendingWrites::new(WriteAccess::ReadOnly),
            ArchiveRecovery::Automatic,
        );

        assert_eq!(
            opened.history_failure, None,
            "a read-only session raised a prompt whose every choice would write"
        );
        assert!(opened.history.path().is_none());
    }
}
