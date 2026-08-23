//! Keeps two GeoTrace instances off the same databases by marking the data
//! directory as in use.
//!
//! The instance that gets there first holds an exclusive advisory lock on
//! `instance.lock` in the data directory for as long as it runs, and writes
//! what it is doing to `instance-status.json` beside it. A crashed instance
//! leaves no lock behind: the lock lives on an open file descriptor, which
//! the OS releases however the process ends. The status file it leaves is
//! read only while the lock is held - a status without a lock behind it says
//! nothing about a live instance.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fd_lock::RwLock;
use gt_pending_writes::{PendingWriteStatus, PendingWrites, WriteAccess};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// The file the lock is taken on. Its contents are never read: only whether
/// a process holds it.
pub const LOCK_FILE_NAME: &str = "instance.lock";

/// The file naming what the instance holding the lock is doing.
pub const STATUS_FILE_NAME: &str = "instance-status.json";

/// A reader never finds half a status: each one is written here and renamed
/// over [`STATUS_FILE_NAME`].
const STATUS_FILE_BEING_WRITTEN_NAME: &str = "instance-status.json.new";

/// Shortest time between two status writes, well above the interval the
/// shutdown wait reads the registry at.
pub const MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES: Duration = Duration::from_millis(500);

/// What the instance holding the data directory is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    /// The instance is running and has not begun shutting down.
    Running,
    /// The shutdown has begun and the writes it is waiting for are listed.
    /// Its window stays up until "Run in background" or the last of those
    /// writes closes it.
    ShuttingDown,
}

/// One running write, as the status file reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingWriteReport {
    pub label: String,
    /// How far the write has got, where it reports progress at all.
    pub progress: Option<f32>,
    /// Which step the write is on, where it names its steps at all.
    pub stage: Option<String>,
}

impl PendingWriteReport {
    /// The writes the registry has running, oldest first.
    fn of_running_writes(pending_writes: &PendingWrites) -> Vec<Self> {
        pending_writes
            .snapshot()
            .running
            .iter()
            .map(Self::from)
            .collect()
    }
}

impl From<&PendingWriteStatus> for PendingWriteReport {
    fn from(status: &PendingWriteStatus) -> Self {
        Self {
            label: status.label.clone(),
            progress: status.progress,
            stage: status.stage.clone(),
        }
    }
}

/// What the instance holding the data directory last wrote about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceStatus {
    pub process_id: u32,
    pub state: InstanceState,
    /// The writes still running, listed once the instance is shutting down.
    pub pending_writes: Vec<PendingWriteReport>,
}

impl InstanceStatus {
    /// The status the instance holding `data_directory` last wrote, or
    /// [`None`] when there is none to read.
    pub fn read_from(data_directory: &Path) -> Option<Self> {
        let path = data_directory.join(STATUS_FILE_NAME);
        let json = match fs::read(&path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) => {
                log::warn!(
                    "Cannot read the instance status file {}: {error}",
                    path.display()
                );
                return None;
            }
        };
        serde_json::from_slice(&json)
            .inspect_err(|error| {
                log::warn!(
                    "The instance status file {} is unreadable: {error}",
                    path.display()
                );
            })
            .ok()
    }
}

/// What this run found when it went to mark the data directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirectoryOwnership {
    /// This process holds the lock and keeps the status file.
    MarkedByThisInstance,
    /// Another instance holds the lock. Nothing here may open the databases
    /// under it, and its status file says what it is doing.
    HeldByAnotherInstance,
    /// The lock file could not be opened or locked, so nothing marks the
    /// directory.
    LockFileUnavailable,
    /// The run has no data directory to mark.
    NoDataDirectory,
}

/// This process's mark on the data directory, and the status file it keeps
/// there while the mark stands.
#[derive(Debug)]
pub struct DataDirectoryLock {
    mark: DataDirectoryMark,
}

/// The mark, with what each outcome leaves this run holding.
#[derive(Debug)]
enum DataDirectoryMark {
    MarkedByThisInstance(MarkedDataDirectory),
    /// The directory is kept: the wait retries the mark on it and reads the
    /// status of the instance holding it.
    HeldByAnotherInstance {
        directory: PathBuf,
    },
    /// A wait can try this directory again once whatever `cause` names has
    /// passed, so it is kept here too.
    LockFileUnavailable {
        directory: PathBuf,
        cause: String,
    },
    NoDataDirectory,
}

impl DataDirectoryMark {
    /// Marks `directory` as in use by this process, or reports what stopped
    /// it. A retry every quarter second repeats no log line: this is silent,
    /// and [`DataDirectoryLock::acquire`] logs what the first attempt found.
    fn of(directory: &Path) -> Self {
        let file = match open_lock_file(directory) {
            Ok(file) => file,
            Err(error) => {
                return Self::LockFileUnavailable {
                    directory: directory.to_owned(),
                    cause: error.to_string(),
                };
            }
        };
        let mut lock_file = RwLock::new(file);
        match take_the_lock(&mut lock_file) {
            Ok(()) => Self::MarkedByThisInstance(MarkedDataDirectory {
                _lock_file: lock_file,
                directory: directory.to_owned(),
                last_status_write: Instant::now(),
            }),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                Self::HeldByAnotherInstance {
                    directory: directory.to_owned(),
                }
            }
            Err(error) => Self::LockFileUnavailable {
                directory: directory.to_owned(),
                cause: format!("{} cannot be locked: {error}", LOCK_FILE_NAME),
            },
        }
    }

    /// The directory to retry on the next attempt, kept by every outcome
    /// except a run with no data directory at all.
    fn directory_to_retry(&self) -> Option<PathBuf> {
        match self {
            Self::HeldByAnotherInstance { directory }
            | Self::LockFileUnavailable { directory, .. } => Some(directory.clone()),
            Self::MarkedByThisInstance(_) | Self::NoDataDirectory => None,
        }
    }

    fn ownership(&self) -> DataDirectoryOwnership {
        match self {
            Self::MarkedByThisInstance(_) => DataDirectoryOwnership::MarkedByThisInstance,
            Self::HeldByAnotherInstance { .. } => DataDirectoryOwnership::HeldByAnotherInstance,
            Self::LockFileUnavailable { .. } => DataDirectoryOwnership::LockFileUnavailable,
            Self::NoDataDirectory => DataDirectoryOwnership::NoDataDirectory,
        }
    }
}

#[derive(Debug)]
struct MarkedDataDirectory {
    /// The lock stands while this file is open, and the OS closes it however
    /// the process ends.
    _lock_file: RwLock<File>,
    directory: PathBuf,
    last_status_write: Instant,
}

impl MarkedDataDirectory {
    fn replace_status_file(&self, status: &InstanceStatus) -> io::Result<()> {
        let being_written = self.directory.join(STATUS_FILE_BEING_WRITTEN_NAME);
        fs::write(&being_written, serde_json::to_vec(status)?)?;
        fs::rename(being_written, self.directory.join(STATUS_FILE_NAME))
    }
}

impl DataDirectoryLock {
    /// Marks `data_directory` as in use by this process for as long as the
    /// lock lives, and reports this instance as running.
    ///
    /// A run that ends up marking nothing - no data directory, a lock file
    /// that would not open, or another instance already holding it - says so
    /// through [`Self::ownership`]. The last of those is what the app opens
    /// a window and waits on, through
    /// [`Self::retry_marking_the_data_directory`], which retries a lock file
    /// that would not open too.
    pub fn acquire(data_directory: Option<&Path>) -> Self {
        let Some(directory) = data_directory else {
            return Self::marking_nothing();
        };
        let mark = DataDirectoryMark::of(directory);
        let mut lock = Self { mark };
        match &lock.mark {
            DataDirectoryMark::MarkedByThisInstance(_) => {
                lock.write_status(InstanceState::Running, Vec::new());
            }
            DataDirectoryMark::HeldByAnotherInstance { .. } => {
                log::info!("Another GeoTrace instance is using {}", directory.display());
            }
            DataDirectoryMark::LockFileUnavailable { cause, .. } => {
                log::warn!("Cannot mark {} as in use: {cause}", directory.display());
            }
            DataDirectoryMark::NoDataDirectory => {}
        }
        lock
    }

    /// Marks `data_directory` as in use where this run owns it, and marks
    /// nothing in a read-only session, which leaves the directory to the
    /// instance holding it and writes nothing under it.
    pub fn acquire_if_owner(write_access: WriteAccess, data_directory: Option<&Path>) -> Self {
        match write_access {
            WriteAccess::Owner => Self::acquire(data_directory),
            WriteAccess::ReadOnly => Self::marking_nothing(),
        }
    }

    /// The lock of a run that marks nothing, whose reports write nothing.
    pub fn marking_nothing() -> Self {
        Self {
            mark: DataDirectoryMark::NoDataDirectory,
        }
    }

    /// What this run owns of the data directory.
    pub fn ownership(&self) -> DataDirectoryOwnership {
        self.mark.ownership()
    }

    /// Whether this process is the instance the data directory is marked as
    /// in use by.
    pub fn marks_the_data_directory(&self) -> bool {
        self.ownership() == DataDirectoryOwnership::MarkedByThisInstance
    }

    /// Gives up on marking the data directory for the rest of the run,
    /// leaving it to the instance that owns it.
    ///
    /// A read-only session gives it up as it starts: with no directory left
    /// to retry, nothing promotes that session to the owner later.
    pub fn give_up_marking_the_data_directory(&mut self) {
        debug_assert!(
            !self.marks_the_data_directory(),
            "the mark is given up before it is taken: dropping it here would leave the status \
             file behind the lock it vouches for"
        );
        self.mark = DataDirectoryMark::NoDataDirectory;
    }

    /// Tries the mark again on the directory this run has yet to take, and
    /// reports what it owns afterwards.
    ///
    /// A lock file that would not open is worth trying again: whatever
    /// stopped it may have passed, and until it succeeds this run owns
    /// nothing. Only a run with no data directory at all has nothing to
    /// retry.
    pub fn retry_marking_the_data_directory(&mut self) -> DataDirectoryOwnership {
        let Some(directory) = self.mark.directory_to_retry() else {
            return self.ownership();
        };
        let mark = DataDirectoryMark::of(&directory);
        let took_it = matches!(mark, DataDirectoryMark::MarkedByThisInstance(_));
        self.mark = mark;
        if took_it {
            self.write_status(InstanceState::Running, Vec::new());
        }
        self.ownership()
    }

    /// What the instance holding the data directory last wrote about itself,
    /// and [`None`] unless another instance holds it.
    pub fn status_of_the_holding_instance(&self) -> Option<InstanceStatus> {
        let DataDirectoryMark::HeldByAnotherInstance { directory } = &self.mark else {
            return None;
        };
        InstanceStatus::read_from(directory)
    }

    /// Why the lock file could not be opened or locked, and [`None`] unless
    /// that is what stopped this run.
    pub fn lock_file_failure(&self) -> Option<String> {
        match &self.mark {
            DataDirectoryMark::LockFileUnavailable { cause, .. } => Some(cause.clone()),
            DataDirectoryMark::MarkedByThisInstance(_)
            | DataDirectoryMark::HeldByAnotherInstance { .. }
            | DataDirectoryMark::NoDataDirectory => None,
        }
    }

    /// Report that this instance has begun shutting down, and what it is
    /// still writing.
    pub fn mark_shutting_down(&mut self, pending_writes: &PendingWrites) {
        self.write_status(
            InstanceState::ShuttingDown,
            PendingWriteReport::of_running_writes(pending_writes),
        );
    }

    /// Report what the shutdown is still writing, no more often than
    /// [`MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES`].
    pub fn report_shutdown_progress(&mut self, pending_writes: &PendingWrites) {
        let DataDirectoryMark::MarkedByThisInstance(marked) = &self.mark else {
            return;
        };
        if marked.last_status_write.elapsed() < MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES {
            return;
        }
        self.write_status(
            InstanceState::ShuttingDown,
            PendingWriteReport::of_running_writes(pending_writes),
        );
    }

    fn write_status(&mut self, state: InstanceState, pending_writes: Vec<PendingWriteReport>) {
        let DataDirectoryMark::MarkedByThisInstance(marked) = &mut self.mark else {
            return;
        };
        marked.last_status_write = Instant::now();
        let status = InstanceStatus {
            process_id: process::id(),
            state,
            pending_writes,
        };
        if let Err(error) = marked.replace_status_file(&status) {
            log::warn!(
                "Cannot write the instance status file {}: {error}",
                marked.directory.join(STATUS_FILE_NAME).display()
            );
        }
    }
}

/// A [`DataDirectoryLock`] the window and the wait for the last writes share.
///
/// The app retries the mark through it while another instance holds the
/// directory, and `main` reports the shutdown through the same lock.
#[derive(Debug, Clone)]
pub struct SharedDataDirectoryLock(Arc<Mutex<DataDirectoryLock>>);

impl SharedDataDirectoryLock {
    pub fn new(lock: DataDirectoryLock) -> Self {
        Self(Arc::new(Mutex::new(lock)))
    }

    /// [`DataDirectoryLock::acquire`], shared from the start.
    pub fn acquire(data_directory: Option<&Path>) -> Self {
        Self::new(DataDirectoryLock::acquire(data_directory))
    }

    /// See [`DataDirectoryLock::acquire_if_owner`].
    pub fn acquire_if_owner(write_access: WriteAccess, data_directory: Option<&Path>) -> Self {
        Self::new(DataDirectoryLock::acquire_if_owner(
            write_access,
            data_directory,
        ))
    }

    /// The lock of a run that marks nothing, whose reports write nothing.
    pub fn marking_nothing() -> Self {
        Self::new(DataDirectoryLock::marking_nothing())
    }

    /// What this run owns of the data directory.
    pub fn ownership(&self) -> DataDirectoryOwnership {
        self.0.lock().ownership()
    }

    /// See [`DataDirectoryLock::give_up_marking_the_data_directory`].
    pub fn give_up_marking_the_data_directory(&self) {
        self.0.lock().give_up_marking_the_data_directory();
    }

    /// See [`DataDirectoryLock::retry_marking_the_data_directory`].
    pub fn retry_marking_the_data_directory(&self) -> DataDirectoryOwnership {
        self.0.lock().retry_marking_the_data_directory()
    }

    /// See [`DataDirectoryLock::status_of_the_holding_instance`].
    pub fn status_of_the_holding_instance(&self) -> Option<InstanceStatus> {
        self.0.lock().status_of_the_holding_instance()
    }

    /// See [`DataDirectoryLock::lock_file_failure`].
    pub fn lock_file_failure(&self) -> Option<String> {
        self.0.lock().lock_file_failure()
    }

    /// See [`DataDirectoryLock::mark_shutting_down`].
    pub fn mark_shutting_down(&self, pending_writes: &PendingWrites) {
        self.0.lock().mark_shutting_down(pending_writes);
    }

    /// See [`DataDirectoryLock::report_shutdown_progress`].
    pub fn report_shutdown_progress(&self, pending_writes: &PendingWrites) {
        self.0.lock().report_shutdown_progress(pending_writes);
    }
}

/// Takes the exclusive lock on `lock_file`, leaving it taken for as long as
/// that file stays open.
///
/// fd-lock releases the lock when its guard drops, and that guard borrows the
/// file it was taken on - which a lock kept beside its own file cannot live
/// with. The guard is forgotten instead, leaving the lock on the open file.
fn take_the_lock(lock_file: &mut RwLock<File>) -> io::Result<()> {
    lock_file.try_write().map(mem::forget)
}

fn open_lock_file(directory: &Path) -> io::Result<File> {
    fs::create_dir_all(directory)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(LOCK_FILE_NAME))
}

impl Drop for DataDirectoryLock {
    /// Takes the status file with the lock that vouches for it, so a clean
    /// exit leaves nothing behind. A force quit leaves its status in the
    /// directory: the next instance to mark it replaces that status as it
    /// writes its own, and nothing reads a status while the lock behind it
    /// is free.
    fn drop(&mut self) {
        let DataDirectoryMark::MarkedByThisInstance(marked) = &self.mark else {
            return;
        };
        let path = marked.directory.join(STATUS_FILE_NAME);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => log::warn!(
                "Cannot remove the instance status file {}: {error}",
                path.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use gt_pending_writes::WriteKind;

    use super::*;

    const TEC_COMPACTION: WriteKind = WriteKind::ArchiveCompaction {
        archive: "ionospheric TEC",
    };

    /// The file names in `directory`, sorted.
    fn file_names_in(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .expect("read the data directory")
            .map(|entry| {
                entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn marking_a_free_directory_reports_this_process_as_running() {
        let directory = tempfile::tempdir().expect("temp dir");

        let lock = DataDirectoryLock::acquire(Some(directory.path()));

        assert!(lock.marks_the_data_directory());
        assert_eq!(
            InstanceStatus::read_from(directory.path()),
            Some(InstanceStatus {
                process_id: process::id(),
                state: InstanceState::Running,
                pending_writes: Vec::new(),
            })
        );
        assert_eq!(
            file_names_in(directory.path()),
            vec![STATUS_FILE_NAME.to_owned(), LOCK_FILE_NAME.to_owned()]
        );
    }

    /// The status file's field and state names are part of the interface,
    /// not an implementation detail of these types: it is read across
    /// processes and across versions.
    #[test]
    fn the_status_file_holds_the_names_a_reader_looks_for() {
        let status = InstanceStatus {
            process_id: 4321,
            state: InstanceState::ShuttingDown,
            pending_writes: vec![PendingWriteReport {
                label: "Compacting the TEC archive".to_owned(),
                progress: Some(0.5),
                stage: None,
            }],
        };

        assert_eq!(
            serde_json::to_string(&status).expect("serialize the status"),
            r#"{"process_id":4321,"state":"shutting_down","pending_writes":[{"label":"Compacting the TEC archive","progress":0.5,"stage":null}]}"#
        );
        assert_eq!(
            serde_json::to_string(&InstanceState::Running).expect("serialize the state"),
            r#""running""#
        );
    }

    /// A process that ends without unwinding - a force quit - leaves its
    /// status file behind, and one that ends between a status write and its
    /// rename leaves the temporary too. The instance that marks the directory
    /// next replaces both, so neither outlives the lock they came with.
    #[test]
    fn marking_the_directory_replaces_the_status_files_the_previous_instance_left() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(directory.path().join(STATUS_FILE_NAME), b"{}").expect("write a stale status");
        fs::write(directory.path().join(STATUS_FILE_BEING_WRITTEN_NAME), b"{}")
            .expect("write a status that never got renamed");

        let lock = DataDirectoryLock::acquire(Some(directory.path()));

        assert!(lock.marks_the_data_directory());
        assert_eq!(
            file_names_in(directory.path()),
            vec![STATUS_FILE_NAME.to_owned(), LOCK_FILE_NAME.to_owned()]
        );
        assert_eq!(
            InstanceStatus::read_from(directory.path()).map(|status| status.process_id),
            Some(process::id())
        );
    }

    /// The first run of a fresh install has no data directory yet.
    #[test]
    fn a_data_directory_that_does_not_exist_yet_is_created_to_be_marked() {
        let parent = tempfile::tempdir().expect("temp dir");
        let directory = parent.path().join("geotrace");

        let lock = DataDirectoryLock::acquire(Some(&directory));

        assert!(lock.marks_the_data_directory());
        assert!(directory.join(LOCK_FILE_NAME).exists());
    }

    /// The lock is on the open file, not on the process: a second attempt
    /// from this very process is refused just as another process's is.
    #[test]
    fn a_directory_already_marked_is_left_to_the_instance_holding_it() {
        let directory = tempfile::tempdir().expect("temp dir");
        let _holder = DataDirectoryLock::acquire(Some(directory.path()));

        let mut second = DataDirectoryLock::acquire(Some(directory.path()));

        assert!(!second.marks_the_data_directory());
        assert_eq!(
            second.ownership(),
            DataDirectoryOwnership::HeldByAnotherInstance
        );
        second.mark_shutting_down(&PendingWrites::default());
        assert_eq!(
            InstanceStatus::read_from(directory.path()).map(|status| status.state),
            Some(InstanceState::Running),
            "the instance holding the directory owns the status file"
        );
    }

    /// A second GeoTrace started read-only leaves the directory to the
    /// instance that owns it: it marks nothing, and the mark it did not take
    /// is still free.
    #[test]
    fn a_read_only_session_marks_nothing_and_leaves_the_directory_free() {
        let directory = tempfile::tempdir().expect("temp dir");

        let read_only =
            DataDirectoryLock::acquire_if_owner(WriteAccess::ReadOnly, Some(directory.path()));

        assert!(!read_only.marks_the_data_directory());
        assert_eq!(file_names_in(directory.path()), Vec::<String>::new());
        assert!(
            DataDirectoryLock::acquire_if_owner(WriteAccess::Owner, Some(directory.path()))
                .marks_the_data_directory(),
            "the instance that owns the directory can still mark it"
        );
    }

    /// A session that gave up on the mark never takes it: the retry the wait
    /// ran has nothing left to try, whoever lets go of the directory.
    #[test]
    fn a_run_that_gave_up_the_mark_never_takes_the_directory_again() {
        let directory = tempfile::tempdir().expect("temp dir");
        let holder = DataDirectoryLock::acquire(Some(directory.path()));
        let mut waiting = DataDirectoryLock::acquire(Some(directory.path()));
        assert_eq!(
            waiting.ownership(),
            DataDirectoryOwnership::HeldByAnotherInstance
        );

        waiting.give_up_marking_the_data_directory();
        drop(holder);

        assert_eq!(
            waiting.retry_marking_the_data_directory(),
            DataDirectoryOwnership::NoDataDirectory
        );
        assert!(!waiting.marks_the_data_directory());
        assert_eq!(
            InstanceStatus::read_from(directory.path()),
            None,
            "the run that gave up the mark wrote a status file"
        );
    }

    #[test]
    fn the_status_file_goes_when_the_lock_does_and_the_directory_can_be_marked_again() {
        let directory = tempfile::tempdir().expect("temp dir");

        drop(DataDirectoryLock::acquire(Some(directory.path())));

        assert_eq!(InstanceStatus::read_from(directory.path()), None);
        assert_eq!(file_names_in(directory.path()), vec![LOCK_FILE_NAME]);
        assert!(
            DataDirectoryLock::acquire(Some(directory.path())).marks_the_data_directory(),
            "the released lock is free for the next run"
        );
    }

    #[test]
    fn a_shutdown_names_the_writes_it_is_still_waiting_for() {
        let directory = tempfile::tempdir().expect("temp dir");
        let pending_writes = PendingWrites::default();
        let compaction = pending_writes
            .try_begin("Compacting the TEC archive", TEC_COMPACTION)
            .expect("the registry is running");
        compaction.set_progress(0.25);
        compaction.set_stage("Rewriting maps");
        let mut lock = DataDirectoryLock::acquire(Some(directory.path()));

        lock.mark_shutting_down(&pending_writes);

        assert_eq!(
            InstanceStatus::read_from(directory.path()),
            Some(InstanceStatus {
                process_id: process::id(),
                state: InstanceState::ShuttingDown,
                pending_writes: vec![PendingWriteReport {
                    label: "Compacting the TEC archive".to_owned(),
                    progress: Some(0.25),
                    stage: Some("Rewriting maps".to_owned()),
                }],
            })
        );
    }

    /// JSON has no number for a NaN, which `f32::clamp` passes through to a
    /// write's reported progress. The write still reaches the status file,
    /// and reads back with no progress.
    #[test]
    fn a_write_reporting_a_progress_json_cannot_hold_is_still_named() {
        let directory = tempfile::tempdir().expect("temp dir");
        let pending_writes = PendingWrites::default();
        let compaction = pending_writes
            .try_begin("Compacting the TEC archive", TEC_COMPACTION)
            .expect("the registry is running");
        compaction.set_progress(f32::NAN);
        let mut lock = DataDirectoryLock::acquire(Some(directory.path()));

        lock.mark_shutting_down(&pending_writes);

        assert_eq!(
            InstanceStatus::read_from(directory.path())
                .expect("the status file")
                .pending_writes,
            vec![PendingWriteReport {
                label: "Compacting the TEC archive".to_owned(),
                progress: None,
                stage: None,
            }]
        );
    }

    #[test]
    fn a_report_is_written_once_the_minimum_interval_has_passed() {
        let directory = tempfile::tempdir().expect("temp dir");
        let pending_writes = PendingWrites::default();
        let compaction = pending_writes
            .try_begin("Compacting the TEC archive", TEC_COMPACTION)
            .expect("the registry is running");
        let mut lock = DataDirectoryLock::acquire(Some(directory.path()));
        lock.mark_shutting_down(&pending_writes);
        drop(compaction);

        lock.report_shutdown_progress(&pending_writes);

        assert_eq!(
            InstanceStatus::read_from(directory.path())
                .expect("the status file")
                .pending_writes
                .len(),
            1,
            "a report inside the minimum interval leaves the file alone"
        );
        thread::sleep(MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES);

        lock.report_shutdown_progress(&pending_writes);

        assert_eq!(
            InstanceStatus::read_from(directory.path())
                .expect("the status file")
                .pending_writes,
            Vec::new(),
            "the write that finished is gone from the status"
        );
    }

    #[test]
    fn a_run_without_a_data_directory_marks_nothing() {
        let mut lock = DataDirectoryLock::acquire(None);

        lock.mark_shutting_down(&PendingWrites::default());

        assert!(!lock.marks_the_data_directory());
        assert_eq!(lock.ownership(), DataDirectoryOwnership::NoDataDirectory);
    }

    /// A directory that cannot be created, and so a lock file that cannot be
    /// opened, is not another instance holding it: there is nothing to show
    /// the user about a second instance, and a cause to report instead.
    #[test]
    fn a_data_directory_that_cannot_be_created_leaves_the_lock_file_unavailable() {
        let parent = tempfile::tempdir().expect("temp dir");
        let file_in_the_way = parent.path().join("geotrace");
        fs::write(&file_in_the_way, b"not a directory").expect("write");

        let lock = DataDirectoryLock::acquire(Some(&file_in_the_way.join("data")));

        assert_eq!(
            lock.ownership(),
            DataDirectoryOwnership::LockFileUnavailable
        );
        assert!(
            lock.lock_file_failure().is_some(),
            "the run reports why it marked nothing"
        );
        assert_eq!(lock.status_of_the_holding_instance(), None);
    }

    /// A lock file that would not open says nothing about who has the
    /// directory, so the wait keeps trying and takes it once the way is
    /// clear.
    #[test]
    fn a_retry_takes_the_directory_once_the_lock_file_can_be_opened() {
        let parent = tempfile::tempdir().expect("temp dir");
        let file_in_the_way = parent.path().join("geotrace");
        fs::write(&file_in_the_way, b"not a directory").expect("write");
        let directory = file_in_the_way.join("data");
        let mut lock = DataDirectoryLock::acquire(Some(&directory));

        assert_eq!(
            lock.retry_marking_the_data_directory(),
            DataDirectoryOwnership::LockFileUnavailable,
            "the file is still in the way"
        );
        fs::remove_file(&file_in_the_way).expect("clear the way");

        assert_eq!(
            lock.retry_marking_the_data_directory(),
            DataDirectoryOwnership::MarkedByThisInstance
        );
        assert_eq!(lock.lock_file_failure(), None);
        assert_eq!(
            InstanceStatus::read_from(&directory).map(|status| status.state),
            Some(InstanceState::Running),
            "the instance that took it reports itself as running"
        );
    }

    /// The wait: a second instance keeps trying, and takes the directory in
    /// the attempt after the one holding it lets go.
    #[test]
    fn a_retry_takes_the_directory_the_instance_holding_it_let_go() {
        let directory = tempfile::tempdir().expect("temp dir");
        let holder = DataDirectoryLock::acquire(Some(directory.path()));
        let mut waiting = DataDirectoryLock::acquire(Some(directory.path()));

        assert_eq!(
            waiting.retry_marking_the_data_directory(),
            DataDirectoryOwnership::HeldByAnotherInstance,
            "the instance holding it has not let go"
        );
        drop(holder);

        assert_eq!(
            waiting.retry_marking_the_data_directory(),
            DataDirectoryOwnership::MarkedByThisInstance
        );
        assert_eq!(
            InstanceStatus::read_from(directory.path()).map(|status| status.state),
            Some(InstanceState::Running),
            "the instance that took it reports itself as running"
        );
    }

    /// A run that marks nothing for want of a data directory has nothing to
    /// retry: every attempt would open a lock file of its own.
    #[test]
    fn a_retry_without_a_data_directory_marks_nothing() {
        let mut lock = DataDirectoryLock::marking_nothing();

        assert_eq!(
            lock.retry_marking_the_data_directory(),
            DataDirectoryOwnership::NoDataDirectory
        );
    }

    /// What the waiting instance shows the user comes from the status file
    /// the instance holding the directory writes.
    #[test]
    fn the_waiting_instance_reads_the_status_of_the_one_holding_the_directory() {
        let directory = tempfile::tempdir().expect("temp dir");
        let pending_writes = PendingWrites::default();
        let _compaction = pending_writes
            .try_begin("Compacting the TEC archive", TEC_COMPACTION)
            .expect("the registry is running");
        let mut holder = DataDirectoryLock::acquire(Some(directory.path()));
        let waiting = DataDirectoryLock::acquire(Some(directory.path()));

        assert_eq!(
            waiting
                .status_of_the_holding_instance()
                .map(|status| status.state),
            Some(InstanceState::Running)
        );
        holder.mark_shutting_down(&pending_writes);

        let status = waiting
            .status_of_the_holding_instance()
            .expect("the status file");

        assert_eq!(status.state, InstanceState::ShuttingDown);
        assert_eq!(
            status
                .pending_writes
                .iter()
                .map(|write| write.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Compacting the TEC archive"]
        );
        assert_eq!(
            holder.status_of_the_holding_instance(),
            None,
            "the instance holding the directory reads no one else's status"
        );
    }

    /// Both ends of the shutdown wait hold the same lock: `main` reports
    /// through its clone while the window's clone is gone.
    #[test]
    fn a_shared_lock_reports_through_every_clone() {
        let directory = tempfile::tempdir().expect("temp dir");
        let lock = SharedDataDirectoryLock::acquire(Some(directory.path()));
        let held_by_main = lock.clone();
        drop(lock);

        held_by_main.mark_shutting_down(&PendingWrites::default());

        assert_eq!(
            InstanceStatus::read_from(directory.path()).map(|status| status.state),
            Some(InstanceState::ShuttingDown)
        );
        assert_eq!(
            held_by_main.ownership(),
            DataDirectoryOwnership::MarkedByThisInstance
        );
    }

    #[test]
    fn a_status_file_that_is_not_a_status_reads_as_nothing() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(directory.path().join(STATUS_FILE_NAME), b"{not json").expect("write");

        assert_eq!(InstanceStatus::read_from(directory.path()), None);
    }
}
