//! The on-disk writes running in the background, registered while they run.
//!
//! A write registers itself through [`PendingWrites::try_begin`] and stays
//! registered until its [`PendingWriteGuard`] drops. Shutdown calls
//! [`PendingWrites::begin_shutdown`], which rejects every write that has not
//! started, and then waits for the ones that have. A registry in
//! [`WriteAccess::ReadOnly`] rejects every write for the rest of the run.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

/// How many finished labels are kept for the shutdown window to list as done.
const RECENTLY_FINISHED_KEPT: usize = 8;

/// Whether this run writes anything the user keeps: the data directory and
/// the settings file.
///
/// A [`Self::ReadOnly`] session is one started beside the instance that owns
/// the data directory: it runs without the instance lock and never writes, so
/// both windows can be open at once.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WriteAccess {
    #[default]
    Owner,
    ReadOnly,
}

impl WriteAccess {
    pub const fn allows_writing(self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// Why the registry rejects a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRejection {
    ShuttingDown,
    ReadOnlySession,
}

impl fmt::Display for WriteRejection {
    /// The reason clause a log line states after its colon, as in
    /// `PruneReport::skipped_line`. The UI wording for a read-only session is
    /// in the `app::read_only_session` constants.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShuttingDown => f.write_str("shutting down"),
            Self::ReadOnlySession => f.write_str("this session is read-only"),
        }
    }
}

/// What stopping the process before a write finishes costs.
///
/// The archives are named as they read inside a sentence, e.g.
/// `"ionospheric TEC"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    /// Rewriting a whole day-keyed archive to drop days from it.
    ArchiveCompaction {
        archive: &'static str,
    },
    /// Adding one downloaded day to a day-keyed archive.
    ArchiveDayInsert {
        archive: &'static str,
    },
    /// Opening the databases at startup, which finishes any delete or repair a
    /// previous run left part-way through.
    DatabaseOpen,
    /// Recording in the data directory that this instance took write access
    /// from the instance holding it.
    TakeOverRecord,
    RecordingDatabase,
    Settings,
}

impl WriteKind {
    /// What an interrupted write of this kind costs, as the force-quit
    /// confirmation lists it.
    pub fn interruption_cost(self) -> String {
        match self {
            Self::ArchiveCompaction { archive } => {
                format!("Every archived {archive} day is discarded")
            }
            Self::ArchiveDayInsert { archive } => {
                format!("The {archive} day is downloaded again next run")
            }
            Self::DatabaseOpen => "The databases are repaired again next run".to_owned(),
            Self::TakeOverRecord => {
                "The data directory holds no record of this take-over".to_owned()
            }
            Self::RecordingDatabase => {
                "The recording database needs repair before it opens again".to_owned()
            }
            Self::Settings => "The settings changed this session are lost".to_owned(),
        }
    }
}

/// The label and kind one write registers under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRegistration {
    pub label: String,
    pub kind: WriteKind,
}

/// One running write, as the shutdown window shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingWriteStatus {
    pub label: String,
    pub kind: WriteKind,
    /// How far the write has got, where it reports progress at all.
    pub progress: Option<f32>,
    /// Which step the write is on, where it reports steps at all.
    pub stage: Option<String>,
}

/// What the registry held at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingWritesSnapshot {
    /// The running writes, oldest first.
    pub running: Vec<PendingWriteStatus>,
    /// The labels of the last few writes that finished, oldest first.
    pub recently_finished: Vec<String>,
}

impl PendingWritesSnapshot {
    /// What stopping the process now costs, one line per distinct
    /// consequence, in the order the writes started.
    pub fn interruption_costs(&self) -> Vec<String> {
        let mut costs: Vec<String> = Vec::new();
        for status in &self.running {
            let cost = status.kind.interruption_cost();
            if !costs.contains(&cost) {
                costs.push(cost);
            }
        }
        costs
    }
}

/// A shared handle to the registry of running writes.
#[derive(Debug, Clone, Default)]
pub struct PendingWrites(Arc<Registry>);

#[derive(Debug, Default)]
struct Registry {
    state: Mutex<RegistryState>,
    /// Notified whenever the last running write finishes.
    idle: Condvar,
}

#[derive(Debug, Default)]
struct RegistryState {
    write_access: WriteAccess,
    shutting_down: bool,
    running: BTreeMap<WriteId, RunningWrite>,
    recently_finished: VecDeque<String>,
    next_id: WriteId,
}

/// Registration order, which is also the order the shutdown window lists the
/// running writes in.
type WriteId = u64;

#[derive(Debug)]
struct RunningWrite {
    label: String,
    kind: WriteKind,
    progress: Option<f32>,
    stage: Option<String>,
}

impl PendingWrites {
    pub fn new(write_access: WriteAccess) -> Self {
        Self(Arc::new(Registry {
            state: Mutex::new(RegistryState {
                write_access,
                ..Default::default()
            }),
            ..Default::default()
        }))
    }

    pub fn write_access(&self) -> WriteAccess {
        self.0.state.lock().write_access
    }

    /// Rejects every write from here to the end of the run.
    ///
    /// There is no way back, which is what the user chose: a session started
    /// read-only beside the instance that owns the data directory stays
    /// read-only until GeoTrace is restarted. Turning read-only part-way
    /// through the run costs nothing where the user makes that choice - a
    /// session still waiting for the data directory has opened no database
    /// and written nothing.
    pub fn become_read_only_for_the_rest_of_the_run(&self) {
        self.0.state.lock().write_access = WriteAccess::ReadOnly;
    }

    /// Why a write starting now is rejected, or [`None`] while the registry
    /// takes them.
    ///
    /// A read-only session names itself even once it is shutting down: the
    /// write was never going to run in it.
    pub fn rejection(&self) -> Option<WriteRejection> {
        let state = self.0.state.lock();
        match state.write_access {
            WriteAccess::ReadOnly => Some(WriteRejection::ReadOnlySession),
            WriteAccess::Owner => state.shutting_down.then_some(WriteRejection::ShuttingDown),
        }
    }

    /// Register a write about to start, or report why the registry rejects it.
    ///
    /// The flag is read and the write registered under one lock, so a write
    /// that got a guard is always one [`Self::wait_until_idle_for`] waits
    /// for.
    pub fn try_begin(
        &self,
        label: impl Into<String>,
        kind: WriteKind,
    ) -> Result<PendingWriteGuard, WriteRejection> {
        let mut state = self.0.state.lock();
        if state.write_access == WriteAccess::ReadOnly {
            return Err(WriteRejection::ReadOnlySession);
        }
        if state.shutting_down {
            return Err(WriteRejection::ShuttingDown);
        }
        Ok(self.register(&mut state, label.into(), kind))
    }

    /// Register a write shutdown itself performs, which [`Self::try_begin`]
    /// rejects by design, or [`None`] in a read-only session, whose shutdown
    /// writes nothing either.
    pub fn try_begin_shutdown_write(
        &self,
        label: impl Into<String>,
        kind: WriteKind,
    ) -> Option<PendingWriteGuard> {
        let mut state = self.0.state.lock();
        let write_access = state.write_access;
        match write_access {
            WriteAccess::ReadOnly => None,
            WriteAccess::Owner => Some(self.register(&mut state, label.into(), kind)),
        }
    }

    fn register(
        &self,
        state: &mut RegistryState,
        label: String,
        kind: WriteKind,
    ) -> PendingWriteGuard {
        let id = state.next_id;
        state.next_id += 1;
        state.running.insert(
            id,
            RunningWrite {
                label,
                kind,
                progress: None,
                stage: None,
            },
        );
        PendingWriteGuard {
            registry: Arc::clone(&self.0),
            id,
        }
    }

    /// Reject every write that has not started yet.
    pub fn begin_shutdown(&self) {
        self.0.state.lock().shutting_down = true;
    }

    /// Whether every registered write has finished.
    pub fn is_idle(&self) -> bool {
        self.0.state.lock().running.is_empty()
    }

    /// Waits for every registered write to finish, reporting whether they
    /// all did within `timeout`. The caller decides what to do about the
    /// ones that did not.
    pub fn wait_until_idle_for(&self, timeout: Duration) -> bool {
        let mut state = self.0.state.lock();
        self.0
            .idle
            .wait_while_for(&mut state, |state| !state.running.is_empty(), timeout);
        state.running.is_empty()
    }

    pub fn snapshot(&self) -> PendingWritesSnapshot {
        let state = self.0.state.lock();
        PendingWritesSnapshot {
            running: state
                .running
                .values()
                .map(|write| PendingWriteStatus {
                    label: write.label.clone(),
                    kind: write.kind,
                    progress: write.progress,
                    stage: write.stage.clone(),
                })
                .collect(),
            recently_finished: state.recently_finished.iter().cloned().collect(),
        }
    }
}

/// Keeps one write registered for as long as it runs.
#[derive(Debug)]
#[must_use]
pub struct PendingWriteGuard {
    registry: Arc<Registry>,
    id: WriteId,
}

impl PendingWriteGuard {
    /// Report how far the write has got, as a fraction of the whole.
    pub fn set_progress(&self, fraction: f32) {
        self.edit_running_write(|write| write.progress = Some(fraction.clamp(0.0, 1.0)));
    }

    /// Report which step the write is on.
    pub fn set_stage(&self, stage: impl Into<String>) {
        self.edit_running_write(|write| write.stage = Some(stage.into()));
    }

    /// A live guard always finds its own write: dropping the guard is the only
    /// thing that takes one out of the registry.
    fn edit_running_write(&self, edit: impl FnOnce(&mut RunningWrite)) {
        let mut state = self.registry.state.lock();
        let write = state.running.get_mut(&self.id);
        debug_assert!(
            write.is_some(),
            "a write stays registered until its guard drops"
        );
        if let Some(write) = write {
            edit(write);
        }
    }
}

impl Drop for PendingWriteGuard {
    fn drop(&mut self) {
        let mut state = self.registry.state.lock();
        if let Some(write) = state.running.remove(&self.id) {
            state.recently_finished.push_back(write.label);
            if state.recently_finished.len() > RECENTLY_FINISHED_KEPT {
                state.recently_finished.pop_front();
            }
        }
        let idle = state.running.is_empty();
        drop(state);
        if idle {
            self.registry.idle.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use rstest::rstest;

    use super::*;

    const TEC: WriteKind = WriteKind::ArchiveCompaction {
        archive: "ionospheric TEC",
    };

    #[test]
    fn a_write_that_started_before_shutdown_keeps_the_registry_busy() {
        let writes = PendingWrites::default();
        let guard = writes.try_begin("Compacting the TEC archive", TEC);

        writes.begin_shutdown();

        assert_eq!(writes.rejection(), Some(WriteRejection::ShuttingDown));
        assert!(!writes.is_idle(), "the write that started is still running");
        drop(guard);
        assert!(writes.is_idle());
    }

    #[test]
    fn a_write_starting_after_shutdown_is_rejected() {
        let writes = PendingWrites::default();
        writes.begin_shutdown();

        assert_eq!(
            writes.try_begin("Compacting the TEC archive", TEC).err(),
            Some(WriteRejection::ShuttingDown)
        );
        assert!(writes.is_idle());
    }

    #[test]
    fn a_write_shutdown_performs_itself_registers_after_shutdown_began() {
        let writes = PendingWrites::default();
        writes.begin_shutdown();

        let guard = writes.try_begin_shutdown_write("Saving settings", WriteKind::Settings);

        assert!(!writes.is_idle());
        drop(guard);
        assert!(writes.is_idle());
    }

    #[rstest]
    #[case(TEC)]
    #[case(WriteKind::ArchiveDayInsert {
        archive: "aircraft interference"
    })]
    #[case(WriteKind::DatabaseOpen)]
    #[case(WriteKind::RecordingDatabase)]
    #[case(WriteKind::Settings)]
    fn a_read_only_session_rejects_every_kind_of_write(#[case] kind: WriteKind) {
        let writes = PendingWrites::new(WriteAccess::ReadOnly);

        assert_eq!(
            writes.try_begin("A write of some kind", kind).err(),
            Some(WriteRejection::ReadOnlySession)
        );
        assert!(
            writes
                .try_begin_shutdown_write("A write of some kind", kind)
                .is_none(),
            "the shutdown of a read-only session writes nothing either"
        );
        assert!(writes.is_idle());
    }

    /// The reason a read-only session gives never becomes "shutting down":
    /// the write it rejected was never going to run.
    #[test]
    fn a_read_only_session_that_begins_shutting_down_still_names_itself() {
        let writes = PendingWrites::new(WriteAccess::ReadOnly);

        writes.begin_shutdown();

        assert_eq!(writes.rejection(), Some(WriteRejection::ReadOnlySession));
        assert_eq!(
            writes
                .try_begin("Saving settings", WriteKind::Settings)
                .err(),
            Some(WriteRejection::ReadOnlySession)
        );
    }

    #[test]
    fn a_session_that_turned_read_only_rejects_the_writes_it_took_before() {
        let writes = PendingWrites::new(WriteAccess::Owner);
        let started_as_owner =
            writes.try_begin("Storing a recording", WriteKind::RecordingDatabase);

        writes.become_read_only_for_the_rest_of_the_run();

        assert_eq!(writes.write_access(), WriteAccess::ReadOnly);
        assert_eq!(writes.rejection(), Some(WriteRejection::ReadOnlySession));
        assert_eq!(
            writes
                .try_begin("Storing a recording", WriteKind::RecordingDatabase)
                .err(),
            Some(WriteRejection::ReadOnlySession)
        );
        assert!(
            !writes.is_idle(),
            "the write that started before the session turned read-only is still running"
        );
        drop(started_as_owner);
        assert!(writes.is_idle());
    }

    #[test]
    fn a_session_that_owns_the_data_directory_takes_writes() {
        let writes = PendingWrites::new(WriteAccess::Owner);

        assert_eq!(writes.rejection(), None);
        assert_eq!(
            writes
                .try_begin("Saving settings", WriteKind::Settings)
                .err(),
            None
        );
    }

    #[rstest]
    #[case(WriteRejection::ShuttingDown, "shutting down")]
    #[case(WriteRejection::ReadOnlySession, "this session is read-only")]
    fn each_rejection_reads_as_a_reason_clause(
        #[case] rejection: WriteRejection,
        #[case] expected: &str,
    ) {
        assert_eq!(rejection.to_string(), expected);
    }

    #[test]
    fn a_finished_write_is_listed_as_recently_finished() {
        let writes = PendingWrites::default();

        drop(writes.try_begin("Saving settings", WriteKind::Settings));

        let snapshot = writes.snapshot();
        assert!(snapshot.running.is_empty());
        assert_eq!(snapshot.recently_finished, vec!["Saving settings"]);
    }

    #[test]
    fn only_the_last_finished_labels_are_kept() {
        let writes = PendingWrites::default();
        for index in 0..RECENTLY_FINISHED_KEPT + 2 {
            drop(writes.try_begin(format!("Write {index}"), WriteKind::Settings));
        }

        let finished = writes.snapshot().recently_finished;

        assert_eq!(finished.len(), RECENTLY_FINISHED_KEPT);
        assert_eq!(finished.first().map(String::as_str), Some("Write 2"));
        assert_eq!(finished.last().map(String::as_str), Some("Write 9"));
    }

    #[test]
    fn the_snapshot_reports_progress_and_stage_in_the_order_writes_started() {
        let writes = PendingWrites::default();
        let compaction = writes
            .try_begin("Compacting the TEC archive", TEC)
            .expect("the registry is running");
        let settings = writes
            .try_begin("Saving settings", WriteKind::Settings)
            .expect("the registry is running");
        compaction.set_progress(0.25);
        compaction.set_stage("Rewriting maps");

        let running = writes.snapshot().running;

        assert_eq!(
            running,
            vec![
                PendingWriteStatus {
                    label: "Compacting the TEC archive".to_owned(),
                    kind: TEC,
                    progress: Some(0.25),
                    stage: Some("Rewriting maps".to_owned()),
                },
                PendingWriteStatus {
                    label: "Saving settings".to_owned(),
                    kind: WriteKind::Settings,
                    progress: None,
                    stage: None,
                },
            ]
        );
        drop(settings);
    }

    #[rstest]
    #[case(-0.5, 0.0)]
    #[case(1.5, 1.0)]
    fn reported_progress_is_clamped_to_a_fraction(#[case] reported: f32, #[case] stored: f32) {
        let writes = PendingWrites::default();
        let guard = writes
            .try_begin("Compacting the TEC archive", TEC)
            .expect("the registry is running");

        guard.set_progress(reported);

        assert_eq!(
            writes.snapshot().running.first().and_then(|w| w.progress),
            Some(stored)
        );
    }

    #[test]
    fn waiting_returns_once_the_last_write_finishes_on_another_thread() {
        let writes = PendingWrites::default();
        let guard = writes
            .try_begin("Compacting the TEC archive", TEC)
            .expect("the registry is running");
        let held_for = Duration::from_millis(80);
        let holder = thread::Builder::new()
            .name("pending-write-holder".to_owned())
            .spawn(move || {
                thread::sleep(held_for);
                drop(guard);
            })
            .expect("spawn the holding thread");

        let started_at = Instant::now();
        assert!(writes.wait_until_idle_for(held_for * 100));

        assert!(
            started_at.elapsed() >= held_for,
            "waiting returned before the write finished"
        );
        assert!(writes.is_idle());
        holder.join().expect("the holding thread finished");
    }

    #[test]
    fn waiting_returns_at_once_when_nothing_is_running() {
        let writes = PendingWrites::default();

        assert!(writes.wait_until_idle_for(Duration::ZERO));
    }

    #[test]
    fn waiting_gives_up_on_a_write_that_is_still_running() {
        let writes = PendingWrites::default();
        let _guard = writes
            .try_begin("Compacting the TEC archive", TEC)
            .expect("the registry is running");

        assert!(!writes.wait_until_idle_for(Duration::from_millis(10)));
    }

    #[rstest]
    #[case(TEC, "Every archived ionospheric TEC day is discarded")]
    #[case(
        WriteKind::ArchiveDayInsert {
            archive: "aircraft interference"
        },
        "The aircraft interference day is downloaded again next run"
    )]
    #[case(WriteKind::DatabaseOpen, "The databases are repaired again next run")]
    #[case(
        WriteKind::RecordingDatabase,
        "The recording database needs repair before it opens again"
    )]
    #[case(WriteKind::Settings, "The settings changed this session are lost")]
    fn each_kind_states_what_interrupting_it_costs(
        #[case] kind: WriteKind,
        #[case] expected: &str,
    ) {
        assert_eq!(kind.interruption_cost(), expected);
    }

    #[test]
    fn two_writes_of_one_kind_cost_one_line_between_them() {
        let writes = PendingWrites::default();
        let guards = [
            writes.try_begin("Storing a recording", WriteKind::RecordingDatabase),
            writes.try_begin("Deleting shelved tracks", WriteKind::RecordingDatabase),
            writes.try_begin("Compacting the TEC archive", TEC),
        ];

        let costs = writes.snapshot().interruption_costs();

        assert_eq!(
            costs,
            vec![
                WriteKind::RecordingDatabase.interruption_cost(),
                TEC.interruption_cost(),
            ],
            "one line per kind, in the order the writes started"
        );
        drop(guards);
    }
}
