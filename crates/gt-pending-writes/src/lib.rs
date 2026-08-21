//! The on-disk writes running in the background, registered while they run.
//!
//! A write registers itself through [`PendingWrites::try_begin`] and stays
//! registered until its [`PendingWriteGuard`] drops. Shutdown calls
//! [`PendingWrites::begin_shutdown`], which refuses every write that has not
//! started, and then waits for the ones that have.

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};

/// How many finished labels are kept for the shutdown window to list as done.
const RECENTLY_FINISHED_KEPT: usize = 8;

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
            Self::RecordingDatabase => {
                "The recording database needs repair before it opens again".to_owned()
            }
            Self::Settings => "The settings changed this session are lost".to_owned(),
        }
    }
}

/// One running write, as the shutdown window shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingWriteStatus {
    pub label: String,
    pub kind: WriteKind,
    /// How far the write has got, where it reports progress at all.
    pub progress: Option<f32>,
    /// Which step the write is on, where it names its steps at all.
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
    shutting_down: bool,
    running: BTreeMap<WriteId, RunningWrite>,
    recently_finished: Vec<String>,
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
    /// Register a write about to start, or [`None`] once shutdown has begun.
    ///
    /// The flag is read and the write registered under one lock, so a write
    /// that got a guard is always one [`Self::wait_until_idle`] waits for.
    pub fn try_begin(
        &self,
        label: impl Into<String>,
        kind: WriteKind,
    ) -> Option<PendingWriteGuard> {
        let mut state = self.0.state.lock();
        if state.shutting_down {
            return None;
        }
        let id = state.next_id;
        state.next_id += 1;
        state.running.insert(
            id,
            RunningWrite {
                label: label.into(),
                kind,
                progress: None,
                stage: None,
            },
        );
        Some(PendingWriteGuard {
            registry: Arc::clone(&self.0),
            id,
        })
    }

    /// Refuse every write that has not started yet.
    pub fn begin_shutdown(&self) {
        self.0.state.lock().shutting_down = true;
    }

    pub fn is_shutting_down(&self) -> bool {
        self.0.state.lock().shutting_down
    }

    /// Whether every registered write has finished.
    pub fn is_idle(&self) -> bool {
        self.0.state.lock().running.is_empty()
    }

    pub fn wait_until_idle(&self) {
        let mut state = self.0.state.lock();
        self.0
            .idle
            .wait_while(&mut state, |state| !state.running.is_empty());
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
            recently_finished: state.recently_finished.clone(),
        }
    }
}

/// Keeps one write registered for as long as it runs.
#[derive(Debug)]
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
            state.recently_finished.push(write.label);
            if state.recently_finished.len() > RECENTLY_FINISHED_KEPT {
                state.recently_finished.remove(0);
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

        assert!(writes.is_shutting_down());
        assert!(!writes.is_idle(), "the write that started is still running");
        drop(guard);
        assert!(writes.is_idle());
    }

    #[test]
    fn a_write_starting_after_shutdown_is_refused() {
        let writes = PendingWrites::default();
        writes.begin_shutdown();

        assert!(
            writes
                .try_begin("Compacting the TEC archive", TEC)
                .is_none()
        );
        assert!(writes.is_idle());
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
        writes.wait_until_idle();

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

        writes.wait_until_idle();

        assert!(writes.is_idle());
    }

    #[rstest]
    #[case(TEC, "Every archived ionospheric TEC day is discarded")]
    #[case(
        WriteKind::ArchiveDayInsert {
            archive: "aircraft interference"
        },
        "The aircraft interference day is downloaded again next run"
    )]
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
            writes.try_begin("Deleting hidden tracks", WriteKind::RecordingDatabase),
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
