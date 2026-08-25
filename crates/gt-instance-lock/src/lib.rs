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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fd_lock::RwLock;
use gt_pending_writes::{PendingWriteStatus, PendingWrites};
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

/// The file the last take-over of the data directory is written to.
pub const TAKE_OVER_FILE_NAME: &str = "take-over.json";

/// A reader never finds half a take-over record: each one is written here and
/// renamed over [`TAKE_OVER_FILE_NAME`].
const TAKE_OVER_FILE_BEING_WRITTEN_NAME: &str = "take-over.json.new";

/// Shortest time between two status writes, well above the interval the
/// shutdown wait reads the registry at.
pub const MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES: Duration = Duration::from_millis(500);

/// A status this old counts as [`StatusFreshness::Stale`]: ten status writes
/// at [`MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES`], five seconds.
pub const AGE_AT_WHICH_A_STATUS_COUNTS_AS_STALE: Duration =
    MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES.saturating_mul(10);

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
    /// Seconds since the Unix epoch on the writing instance's clock.
    /// [`None`] in a status file from a GeoTrace before this field, and
    /// where the writing clock reads before the epoch.
    pub written_at: Option<u64>,
}

/// The age of an [`InstanceStatus`], as the reading clock measures it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFreshness {
    /// The status is younger than [`AGE_AT_WHICH_A_STATUS_COUNTS_AS_STALE`].
    Current,
    /// The status is `age` old, at or past
    /// [`AGE_AT_WHICH_A_STATUS_COUNTS_AS_STALE`].
    Stale { age: Duration },
    /// The status has no `written_at`.
    UnknownAge,
    /// An [`InstanceState::Running`] status, written once when this instance
    /// takes the mark, and not rewritten until the shutdown begins.
    NotRefreshedWhileRunning,
}

impl InstanceStatus {
    /// [`Self::freshness_at`] against [`SystemTime::now`].
    pub fn freshness(&self) -> StatusFreshness {
        self.freshness_at(SystemTime::now())
    }

    /// How old this status is against `now`.
    ///
    /// Only an [`InstanceState::ShuttingDown`] status is measured:
    /// [`DataDirectoryLock::report_shutdown_progress`] rewrites it every
    /// [`MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES`], so its age is how long
    /// the instance has gone without reporting.
    ///
    /// Clock skew never makes a status stale: a `written_at` past `now`
    /// gives an age of zero.
    pub fn freshness_at(&self, now: SystemTime) -> StatusFreshness {
        if self.state == InstanceState::Running {
            return StatusFreshness::NotRefreshedWhileRunning;
        }
        let Some(written_at) = self.written_at else {
            return StatusFreshness::UnknownAge;
        };
        let now = now
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        let age = Duration::from_secs(now.saturating_sub(written_at));
        if age >= AGE_AT_WHICH_A_STATUS_COUNTS_AS_STALE {
            StatusFreshness::Stale { age }
        } else {
            StatusFreshness::Current
        }
    }
}

/// What a read of [`STATUS_FILE_NAME`] in a data directory found.
#[derive(Debug, Clone, PartialEq)]
pub enum InstanceStatusRead {
    Status(InstanceStatus),
    Absent,
    /// `fs::read` failed with something other than
    /// [`io::ErrorKind::NotFound`], with the error it reported.
    Unreadable(String),
    /// `serde_json` rejected the file, with the error it reported.
    Malformed(String),
}

impl InstanceStatusRead {
    /// Reads [`STATUS_FILE_NAME`] in `data_directory`.
    pub fn read_from(data_directory: &Path) -> Self {
        let path = data_directory.join(STATUS_FILE_NAME);
        let json = match fs::read(&path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::Absent,
            Err(error) => {
                log::warn!(
                    "Cannot read the instance status file {}: {error}",
                    path.display()
                );
                return Self::Unreadable(error.to_string());
            }
        };
        match serde_json::from_slice(&json) {
            Ok(status) => Self::Status(status),
            Err(error) => {
                log::warn!(
                    "The instance status file {} is not a status: {error}",
                    path.display()
                );
                Self::Malformed(error.to_string())
            }
        }
    }

    /// The status the read parsed, and [`None`] for the three failures.
    pub fn status(&self) -> Option<&InstanceStatus> {
        match self {
            Self::Status(status) => Some(status),
            Self::Absent | Self::Unreadable(_) | Self::Malformed(_) => None,
        }
    }
}

/// What a take-over left in the data directory: one instance opened the
/// databases while another instance held the lock.
///
/// [`DataDirectoryLock::record_take_over`] writes the file and the next
/// take-over replaces it. It outlives the session that took write access:
/// nothing deletes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeOverRecord {
    /// The process that took write access.
    pub taken_by_process_id: u32,
    /// The `process_id` read from [`STATUS_FILE_NAME`] at the take-over, and
    /// [`None`] where that file could not be read.
    pub taken_from_process_id: Option<u32>,
    /// Seconds since the Unix epoch on the taking instance's clock. [`None`]
    /// in a take-over file from a GeoTrace before this field, and where that
    /// clock reads before the epoch.
    pub written_at: Option<u64>,
}

impl TakeOverRecord {
    /// Reads [`TAKE_OVER_FILE_NAME`] in `data_directory`, and reports
    /// [`None`] where that file is absent, unreadable or not a record.
    pub fn read_from(data_directory: &Path) -> Option<Self> {
        let path = data_directory.join(TAKE_OVER_FILE_NAME);
        let json = match fs::read(&path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) => {
                log::warn!("Cannot read the take-over file {}: {error}", path.display());
                return None;
            }
        };
        match serde_json::from_slice(&json) {
            Ok(record) => Some(record),
            Err(error) => {
                log::warn!(
                    "The take-over file {} is not a take-over record: {error}",
                    path.display()
                );
                None
            }
        }
    }

    fn replace_in(&self, directory: &Path) -> io::Result<()> {
        let being_written = directory.join(TAKE_OVER_FILE_BEING_WRITTEN_NAME);
        fs::write(&being_written, serde_json::to_vec(self)?)?;
        fs::rename(being_written, directory.join(TAKE_OVER_FILE_NAME))
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

    /// The data directory this run marked or has yet to take, and [`None`]
    /// for a run with none.
    fn directory(&self) -> Option<&Path> {
        match self {
            Self::MarkedByThisInstance(marked) => Some(&marked.directory),
            Self::HeldByAnotherInstance { directory }
            | Self::LockFileUnavailable { directory, .. } => Some(directory),
            Self::NoDataDirectory => None,
        }
    }

    /// The directory to retry on the next attempt, kept by every outcome
    /// except a run with no data directory at all.
    fn directory_to_retry(&self) -> Option<PathBuf> {
        match self {
            Self::HeldByAnotherInstance { .. } | Self::LockFileUnavailable { .. } => {
                self.directory().map(Path::to_owned)
            }
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

    /// Gives up on marking the data directory for the rest of the run,
    /// leaving it to the instance that owns it.
    ///
    /// A read-only session gives it up as it starts: with no directory left
    /// to retry, nothing promotes that session to the owner later.
    pub fn give_up_marking_the_data_directory(&mut self) {
        debug_assert!(
            self.ownership() != DataDirectoryOwnership::MarkedByThisInstance,
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

    /// A read of the status file of the instance holding the data directory,
    /// and [`None`] unless another instance holds it.
    pub fn status_of_the_holding_instance(&self) -> Option<InstanceStatusRead> {
        let DataDirectoryMark::HeldByAnotherInstance { directory } = &self.mark else {
            return None;
        };
        Some(InstanceStatusRead::read_from(directory))
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

    /// Records in the data directory that this process took write access
    /// from the instance holding it, and reports the take-over the record
    /// replaces.
    ///
    /// A run that marks no data directory - a read-only session, and a run
    /// without one at all - writes nothing here.
    pub fn record_take_over(&self, taken_from_process_id: Option<u32>) -> Option<TakeOverRecord> {
        let directory = self.mark.directory()?;
        let replaced = TakeOverRecord::read_from(directory);
        let record = TakeOverRecord {
            taken_by_process_id: process::id(),
            taken_from_process_id,
            written_at: seconds_since_the_epoch(SystemTime::now()),
        };
        if let Err(error) = record.replace_in(directory) {
            log::warn!(
                "Cannot write the take-over file {}: {error}",
                directory.join(TAKE_OVER_FILE_NAME).display()
            );
        }
        replaced
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
            written_at: seconds_since_the_epoch(SystemTime::now()),
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
    pub fn status_of_the_holding_instance(&self) -> Option<InstanceStatusRead> {
        self.0.lock().status_of_the_holding_instance()
    }

    /// See [`DataDirectoryLock::lock_file_failure`].
    pub fn lock_file_failure(&self) -> Option<String> {
        self.0.lock().lock_file_failure()
    }

    /// See [`DataDirectoryLock::record_take_over`].
    pub fn record_take_over(&self, taken_from_process_id: Option<u32>) -> Option<TakeOverRecord> {
        self.0.lock().record_take_over(taken_from_process_id)
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

/// `time` in the unit [`InstanceStatus::written_at`] and
/// [`TakeOverRecord::written_at`] hold, and [`None`] for a reading before the
/// Unix epoch.
pub fn seconds_since_the_epoch(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|since_the_epoch| since_the_epoch.as_secs())
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
    /// Removes the status file, so a clean exit leaves nothing behind.
    ///
    /// `process::exit` skips this, so a force quit leaves the status file in
    /// the directory. `DataDirectoryMark::of` overwrites it when the next
    /// instance takes the lock, and `status_of_the_holding_instance` only
    /// reads it while another instance holds the lock, so a leftover one is
    /// never read.
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
    use rstest::rstest;

    use super::*;

    const TEC_COMPACTION: WriteKind = WriteKind::ArchiveCompaction {
        archive: "ionospheric TEC",
    };

    /// The clock the freshness cases read the status against.
    const READING_CLOCK_SECONDS_SINCE_THE_EPOCH: u64 = 1_700_000_000;

    /// The status the instance holding `directory` wrote, where the read
    /// found and parsed one.
    fn status_file_in(directory: &Path) -> Option<InstanceStatus> {
        InstanceStatusRead::read_from(directory).status().cloned()
    }

    /// A status file as an instance writes it, with the state and
    /// `written_at` left to the case.
    fn status_written_at(state: InstanceState, written_at: Option<u64>) -> InstanceStatus {
        InstanceStatus {
            process_id: 4321,
            state,
            pending_writes: Vec::new(),
            written_at,
        }
    }

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

        assert_eq!(
            lock.ownership(),
            DataDirectoryOwnership::MarkedByThisInstance
        );
        let status = status_file_in(directory.path()).expect("the status file");
        assert_eq!(status.process_id, process::id());
        assert_eq!(status.state, InstanceState::Running);
        assert_eq!(status.pending_writes, Vec::new());
        assert_eq!(
            status.freshness(),
            StatusFreshness::NotRefreshedWhileRunning
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
            written_at: Some(READING_CLOCK_SECONDS_SINCE_THE_EPOCH),
        };

        assert_eq!(
            serde_json::to_string(&status).expect("serialize the status"),
            r#"{"process_id":4321,"state":"shutting_down","pending_writes":[{"label":"Compacting the TEC archive","progress":0.5,"stage":null}],"written_at":1700000000}"#
        );
        assert_eq!(
            serde_json::to_string(&InstanceState::Running).expect("serialize the state"),
            r#""running""#
        );
    }

    /// A force quit skips `Drop` and leaves the status file behind, and a
    /// process that ends between the write and the rename in
    /// `replace_status_file` leaves the temporary too. Taking the lock
    /// replaces both.
    #[test]
    fn marking_the_directory_replaces_the_status_files_the_previous_instance_left() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(directory.path().join(STATUS_FILE_NAME), b"{}").expect("write a stale status");
        fs::write(directory.path().join(STATUS_FILE_BEING_WRITTEN_NAME), b"{}")
            .expect("write a status that never got renamed");

        let lock = DataDirectoryLock::acquire(Some(directory.path()));

        assert_eq!(
            lock.ownership(),
            DataDirectoryOwnership::MarkedByThisInstance
        );
        assert_eq!(
            file_names_in(directory.path()),
            vec![STATUS_FILE_NAME.to_owned(), LOCK_FILE_NAME.to_owned()]
        );
        assert_eq!(
            status_file_in(directory.path()).map(|status| status.process_id),
            Some(process::id())
        );
    }

    /// The first run of a fresh install has no data directory yet.
    #[test]
    fn a_data_directory_that_does_not_exist_yet_is_created_to_be_marked() {
        let parent = tempfile::tempdir().expect("temp dir");
        let directory = parent.path().join("geotrace");

        let lock = DataDirectoryLock::acquire(Some(&directory));

        assert_eq!(
            lock.ownership(),
            DataDirectoryOwnership::MarkedByThisInstance
        );
        assert!(directory.join(LOCK_FILE_NAME).exists());
    }

    /// The lock is on the open file, not on the process: a second attempt
    /// from this very process is refused just as another process's is.
    #[test]
    fn a_directory_already_marked_is_left_to_the_instance_holding_it() {
        let directory = tempfile::tempdir().expect("temp dir");
        let _holder = DataDirectoryLock::acquire(Some(directory.path()));

        let mut second = DataDirectoryLock::acquire(Some(directory.path()));

        assert_eq!(
            second.ownership(),
            DataDirectoryOwnership::HeldByAnotherInstance
        );
        second.mark_shutting_down(&PendingWrites::default());
        assert_eq!(
            status_file_in(directory.path()).map(|status| status.state),
            Some(InstanceState::Running),
            "the instance holding the directory owns the status file"
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
        assert_eq!(
            InstanceStatusRead::read_from(directory.path()),
            InstanceStatusRead::Absent,
            "the run that gave up the mark wrote a status file"
        );
        assert_eq!(
            DataDirectoryLock::acquire(Some(directory.path())).ownership(),
            DataDirectoryOwnership::MarkedByThisInstance,
            "the run that gave up the mark holds the lock the next instance needs"
        );
    }

    #[test]
    fn the_status_file_goes_when_the_lock_does_and_the_directory_can_be_marked_again() {
        let directory = tempfile::tempdir().expect("temp dir");

        drop(DataDirectoryLock::acquire(Some(directory.path())));

        assert_eq!(
            InstanceStatusRead::read_from(directory.path()),
            InstanceStatusRead::Absent
        );
        assert_eq!(file_names_in(directory.path()), vec![LOCK_FILE_NAME]);
        assert_eq!(
            DataDirectoryLock::acquire(Some(directory.path())).ownership(),
            DataDirectoryOwnership::MarkedByThisInstance,
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

        let status = status_file_in(directory.path()).expect("the status file");
        assert_eq!(status.state, InstanceState::ShuttingDown);
        assert_eq!(
            status.pending_writes,
            vec![PendingWriteReport {
                label: "Compacting the TEC archive".to_owned(),
                progress: Some(0.25),
                stage: Some("Rewriting maps".to_owned()),
            }]
        );
        assert_eq!(status.freshness(), StatusFreshness::Current);
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
            status_file_in(directory.path())
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
            status_file_in(directory.path())
                .expect("the status file")
                .pending_writes
                .len(),
            1,
            "a report inside the minimum interval leaves the file alone"
        );
        thread::sleep(MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES);

        lock.report_shutdown_progress(&pending_writes);

        assert_eq!(
            status_file_in(directory.path())
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
            status_file_in(&directory).map(|status| status.state),
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
            status_file_in(directory.path()).map(|status| status.state),
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
                .and_then(|read| read.status().map(|status| status.state)),
            Some(InstanceState::Running)
        );
        holder.mark_shutting_down(&pending_writes);

        let status = waiting
            .status_of_the_holding_instance()
            .and_then(|read| read.status().cloned())
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
            status_file_in(directory.path()).map(|status| status.state),
            Some(InstanceState::ShuttingDown)
        );
        assert_eq!(
            held_by_main.ownership(),
            DataDirectoryOwnership::MarkedByThisInstance
        );
    }

    #[test]
    fn a_directory_without_a_status_file_reads_as_absent() {
        let directory = tempfile::tempdir().expect("temp dir");

        assert_eq!(
            InstanceStatusRead::read_from(directory.path()),
            InstanceStatusRead::Absent
        );
    }

    /// A directory in place of the status file fails `fs::read` with an error
    /// other than `NotFound`.
    #[test]
    fn a_status_file_that_cannot_be_read_reads_as_unreadable() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::create_dir(directory.path().join(STATUS_FILE_NAME))
            .expect("create a directory where the status file goes");

        assert!(matches!(
            InstanceStatusRead::read_from(directory.path()),
            InstanceStatusRead::Unreadable(_)
        ));
    }

    #[test]
    fn a_status_file_serde_json_rejects_reads_as_malformed() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(directory.path().join(STATUS_FILE_NAME), b"{not json").expect("write");

        assert!(matches!(
            InstanceStatusRead::read_from(directory.path()),
            InstanceStatusRead::Malformed(_)
        ));
    }

    /// The instance the take-over in these cases took write access from.
    const TAKEN_FROM_PROCESS_ID: u32 = 4321;

    /// A wait that found `directory` held by another instance, which is the
    /// state a take-over starts from.
    fn lock_waiting_for(directory: &Path) -> DataDirectoryLock {
        let lock = DataDirectoryLock::acquire(Some(directory));
        assert_eq!(
            lock.ownership(),
            DataDirectoryOwnership::HeldByAnotherInstance
        );
        lock
    }

    #[test]
    fn a_take_over_records_this_process_and_the_one_it_took_write_access_from() {
        let directory = tempfile::tempdir().expect("temp dir");
        let _holder = DataDirectoryLock::acquire(Some(directory.path()));
        let taking_over = lock_waiting_for(directory.path());
        let clock_before_the_take_over = seconds_since_the_epoch(SystemTime::now())
            .expect("a clock reading after the Unix epoch");

        let replaced = taking_over.record_take_over(Some(TAKEN_FROM_PROCESS_ID));

        assert_eq!(
            replaced, None,
            "a directory with no take-over file replaced one"
        );
        let record = TakeOverRecord::read_from(directory.path()).expect("the take-over file");
        assert_eq!(record.taken_by_process_id, process::id());
        assert_eq!(record.taken_from_process_id, Some(TAKEN_FROM_PROCESS_ID));
        assert!(
            record
                .written_at
                .is_some_and(|written_at| written_at >= clock_before_the_take_over),
            "the record is stamped {:?}, before the take-over",
            record.written_at
        );
    }

    /// Each take-over replaces the file the one before it wrote, and the
    /// instance recording this one reads what it replaced.
    #[test]
    fn a_take_over_reports_the_take_over_recorded_before_it() {
        let directory = tempfile::tempdir().expect("temp dir");
        let _holder = DataDirectoryLock::acquire(Some(directory.path()));
        let taking_over = lock_waiting_for(directory.path());
        taking_over.record_take_over(Some(TAKEN_FROM_PROCESS_ID));

        let replaced = taking_over.record_take_over(Some(8765));

        assert_eq!(
            replaced.and_then(|record| record.taken_from_process_id),
            Some(TAKEN_FROM_PROCESS_ID)
        );
        assert_eq!(
            TakeOverRecord::read_from(directory.path())
                .and_then(|record| record.taken_from_process_id),
            Some(8765),
            "the second take-over left the first one's record in place"
        );
    }

    /// `record_take_over` writes into the directory the mark is on, and
    /// `give_up_marking_the_data_directory` leaves the mark on none.
    #[test]
    fn a_run_that_marks_no_directory_records_no_take_over() {
        let directory = tempfile::tempdir().expect("temp dir");
        let _holder = DataDirectoryLock::acquire(Some(directory.path()));
        let mut read_only = lock_waiting_for(directory.path());
        read_only.give_up_marking_the_data_directory();

        assert_eq!(
            read_only.record_take_over(Some(TAKEN_FROM_PROCESS_ID)),
            None
        );
        assert_eq!(TakeOverRecord::read_from(directory.path()), None);
    }

    /// The take-over file's field names are part of the interface, not an
    /// implementation detail of [`TakeOverRecord`]: it is read across
    /// processes and across versions.
    #[test]
    fn the_take_over_file_holds_the_names_a_reader_looks_for() {
        let record = TakeOverRecord {
            taken_by_process_id: 1234,
            taken_from_process_id: Some(TAKEN_FROM_PROCESS_ID),
            written_at: Some(READING_CLOCK_SECONDS_SINCE_THE_EPOCH),
        };

        assert_eq!(
            serde_json::to_string(&record).expect("serialize the record"),
            r#"{"taken_by_process_id":1234,"taken_from_process_id":4321,"written_at":1700000000}"#
        );
    }

    /// A take-over file from a GeoTrace before the `written_at` field has
    /// none: serde reads the missing field as [`None`].
    #[test]
    fn a_take_over_file_without_a_written_at_parses_without_one() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(
            directory.path().join(TAKE_OVER_FILE_NAME),
            br#"{"taken_by_process_id":1234,"taken_from_process_id":4321}"#,
        )
        .expect("write");

        assert_eq!(
            TakeOverRecord::read_from(directory.path()).map(|record| record.written_at),
            Some(None)
        );
    }

    #[test]
    fn a_take_over_file_serde_json_rejects_reads_as_no_take_over() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(directory.path().join(TAKE_OVER_FILE_NAME), b"{not json").expect("write");

        assert_eq!(TakeOverRecord::read_from(directory.path()), None);
    }

    /// A status file from a GeoTrace before the `written_at` field existed
    /// has none: serde reads the missing field as [`None`], so the file
    /// parses and reads as [`StatusFreshness::UnknownAge`].
    #[test]
    fn a_status_file_without_a_written_at_parses_and_has_an_unknown_age() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(
            directory.path().join(STATUS_FILE_NAME),
            br#"{"process_id":4321,"state":"shutting_down","pending_writes":[]}"#,
        )
        .expect("write");

        assert_eq!(
            status_file_in(directory.path()).map(|status| status.freshness()),
            Some(StatusFreshness::UnknownAge)
        );
    }

    /// The reading clock may be ahead of or behind the writing one, a status
    /// file from a GeoTrace before `written_at` has none to measure, and a
    /// running instance rewrites its status only once its shutdown begins.
    #[rstest]
    #[case::a_second_under_the_stale_age(
        InstanceState::ShuttingDown,
        Some(READING_CLOCK_SECONDS_SINCE_THE_EPOCH - AGE_AT_WHICH_A_STATUS_COUNTS_AS_STALE.as_secs() + 1),
        StatusFreshness::Current
    )]
    #[case::at_the_stale_age(
        InstanceState::ShuttingDown,
        Some(READING_CLOCK_SECONDS_SINCE_THE_EPOCH - AGE_AT_WHICH_A_STATUS_COUNTS_AS_STALE.as_secs()),
        StatusFreshness::Stale { age: AGE_AT_WHICH_A_STATUS_COUNTS_AS_STALE }
    )]
    #[case::stamped_an_hour_in_the_future(
        InstanceState::ShuttingDown,
        Some(READING_CLOCK_SECONDS_SINCE_THE_EPOCH + 3600),
        StatusFreshness::Current
    )]
    #[case::without_a_written_at(InstanceState::ShuttingDown, None, StatusFreshness::UnknownAge)]
    #[case::running_since_before_the_stale_age(
        InstanceState::Running,
        Some(READING_CLOCK_SECONDS_SINCE_THE_EPOCH - 86_400),
        StatusFreshness::NotRefreshedWhileRunning
    )]
    fn a_status_is_current_until_it_reaches_the_stale_age(
        #[case] state: InstanceState,
        #[case] written_at: Option<u64>,
        #[case] expected: StatusFreshness,
    ) {
        let now = UNIX_EPOCH + Duration::from_secs(READING_CLOCK_SECONDS_SINCE_THE_EPOCH);

        assert_eq!(
            status_written_at(state, written_at).freshness_at(now),
            expected
        );
    }
}
