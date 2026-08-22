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
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

use fd_lock::{RwLock, RwLockWriteGuard};
use gt_pending_writes::{PendingWriteStatus, PendingWrites};
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
    /// The window is up, whether or not it has begun closing.
    Running,
    /// The window is gone and the writes that outlive it are finishing.
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

/// This process's mark on the data directory, and the status file it keeps
/// there while the mark stands.
///
/// A run that marked nothing - no data directory, or another instance holds
/// it - gets one whose reports write nothing.
#[derive(Debug)]
pub struct DataDirectoryLock {
    marked: Option<MarkedDataDirectory>,
}

#[derive(Debug)]
struct MarkedDataDirectory {
    /// Unlocked when this drops, and by the OS should the process die first.
    _lock: RwLockWriteGuard<'static, File>,
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
    /// A run that does not end up marking the directory - another instance
    /// holds it, or the lock file could not be opened - keeps that same open
    /// lock file for its own lifetime.
    ///
    /// A run with no data directory, one whose lock file cannot be opened,
    /// and one that finds another instance already holding the lock all get
    /// a lock that marks nothing and go ahead unchanged.
    ///
    /// The open lock file is kept for the rest of the process: releasing the
    /// lock is this lock's to do, and the descriptor it sits on outlives
    /// every borrow of it.
    pub fn acquire(data_directory: Option<&Path>) -> Self {
        let Some(directory) = data_directory else {
            return Self::marking_nothing();
        };
        let lock_file: &'static mut RwLock<File> = match open_lock_file(directory) {
            Ok(file) => Box::leak(Box::new(RwLock::new(file))),
            Err(error) => {
                log::warn!("Cannot mark {} as in use: {error}", directory.display());
                return Self::marking_nothing();
            }
        };
        match lock_file.try_write() {
            Ok(held) => Self::marking(held, directory),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                log::info!("Another GeoTrace instance is using {}", directory.display());
                Self::marking_nothing()
            }
            Err(error) => {
                log::warn!(
                    "Cannot lock {}: {error}",
                    directory.join(LOCK_FILE_NAME).display()
                );
                Self::marking_nothing()
            }
        }
    }

    /// The lock of a run that marks nothing, whose reports write nothing.
    pub fn marking_nothing() -> Self {
        Self { marked: None }
    }

    fn marking(held: RwLockWriteGuard<'static, File>, directory: &Path) -> Self {
        let mut lock = Self {
            marked: Some(MarkedDataDirectory {
                _lock: held,
                directory: directory.to_owned(),
                last_status_write: Instant::now(),
            }),
        };
        lock.write_status(InstanceState::Running, Vec::new());
        lock
    }

    /// Whether this process is the instance the data directory is marked as
    /// in use by.
    pub fn marks_the_data_directory(&self) -> bool {
        self.marked.is_some()
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
        let Some(marked) = &self.marked else {
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
        let Some(marked) = &mut self.marked else {
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
    /// No instance finds the directory free with a status still in it: the
    /// status file goes before the lock that vouches for it.
    fn drop(&mut self) {
        let Some(marked) = &self.marked else {
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
        second.mark_shutting_down(&PendingWrites::default());
        assert_eq!(
            InstanceStatus::read_from(directory.path()).map(|status| status.state),
            Some(InstanceState::Running),
            "the instance holding the directory owns the status file"
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
    }

    #[test]
    fn a_status_file_that_is_not_a_status_reads_as_nothing() {
        let directory = tempfile::tempdir().expect("temp dir");
        fs::write(directory.path().join(STATUS_FILE_NAME), b"{not json").expect("write");

        assert_eq!(InstanceStatus::read_from(directory.path()), None);
    }
}
