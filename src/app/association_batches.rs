use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gt_loaded_files::LoadedFileId;
use gt_ui_types::LoadedLogId;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct AssociationBatchId(u64);

#[derive(Debug, Default)]
struct BatchProgress {
    recordings: AtomicUsize,
    logs: AtomicUsize,
    submitted: AtomicBool,
}

#[derive(Debug)]
pub(super) struct AssociationSubmission {
    id: AssociationBatchId,
    progress: Arc<BatchProgress>,
    next_recording_ordinal: Cell<usize>,
}

impl AssociationSubmission {
    pub(super) fn recording(&self) -> RecordingArrival {
        self.progress.recordings.fetch_add(1, Ordering::Relaxed);
        let ordinal = self.next_recording_ordinal.get();
        self.next_recording_ordinal.set(ordinal + 1);
        RecordingArrival {
            batch: self.id,
            ordinal: RecordingArrivalOrdinal(ordinal),
            progress: Arc::clone(&self.progress),
        }
    }

    pub(super) fn log(&self) -> LogArrival {
        self.progress.logs.fetch_add(1, Ordering::Relaxed);
        LogArrival {
            batch: self.id,
            progress: Arc::clone(&self.progress),
        }
    }
}

impl Drop for AssociationSubmission {
    fn drop(&mut self) {
        self.progress.submitted.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RecordingArrivalOrdinal(usize);

struct ArrivalCandidate {
    recording: LoadedFileId,
    ordinal: RecordingArrivalOrdinal,
}

struct BatchState {
    initial_candidates: Vec<LoadedFileId>,
    arrival_candidates: Vec<ArrivalCandidate>,
    logs: Vec<LoadedLogId>,
    progress: Arc<BatchProgress>,
}

impl BatchState {
    fn ordered_candidates(&self) -> Vec<LoadedFileId> {
        let mut arrivals: Vec<_> = self.arrival_candidates.iter().collect();
        arrivals.sort_by_key(|candidate| candidate.ordinal);
        let mut candidates = self.initial_candidates.clone();
        for candidate in arrivals {
            if !candidates.contains(&candidate.recording) {
                candidates.push(candidate.recording);
            }
        }
        candidates
    }
}

#[derive(Default)]
struct BatchRegistry {
    batches: BTreeMap<AssociationBatchId, BatchState>,
    next_id: u64,
}

#[derive(Default)]
pub(super) struct AssociationBatches(BatchRegistry);

#[derive(Debug)]
pub enum RecordingOperationOrigin {
    Arrival(RecordingArrival),
    Independent,
}

/// The app transfers each recording operation through screening, prompts and loading.
/// Dropping an operation completes it on success, failure or cancellation.
#[derive(Debug)]
pub struct RecordingArrival {
    batch: AssociationBatchId,
    ordinal: RecordingArrivalOrdinal,
    progress: Arc<BatchProgress>,
}

impl Drop for RecordingArrival {
    fn drop(&mut self) {
        let previous = self.progress.recordings.fetch_sub(1, Ordering::Release);
        debug_assert!(previous > 0);
    }
}

#[derive(Debug)]
pub(super) struct LogArrival {
    batch: AssociationBatchId,
    progress: Arc<BatchProgress>,
}

impl Drop for LogArrival {
    fn drop(&mut self) {
        let previous = self.progress.logs.fetch_sub(1, Ordering::Release);
        debug_assert!(previous > 0);
    }
}

impl AssociationBatches {
    pub(super) fn begin_submission(
        &mut self,
        candidates: impl Iterator<Item = LoadedFileId>,
    ) -> AssociationSubmission {
        let id = AssociationBatchId(self.0.next_id);
        self.0.next_id += 1;
        let progress = Arc::new(BatchProgress::default());
        self.0.batches.insert(
            id,
            BatchState {
                initial_candidates: candidates.collect(),
                arrival_candidates: Vec::new(),
                logs: Vec::new(),
                progress: Arc::clone(&progress),
            },
        );
        AssociationSubmission {
            id,
            progress,
            next_recording_ordinal: Cell::new(0),
        }
    }

    pub(super) fn register_log(&mut self, arrival: &LogArrival, log: LoadedLogId) {
        let batch = self.batch_for_live_arrival_mut(arrival.batch);
        if !batch.logs.contains(&log) {
            batch.logs.push(log);
        }
    }

    pub(super) fn register_recording(&mut self, arrival: &RecordingArrival, id: LoadedFileId) {
        self.batch_for_live_arrival_mut(arrival.batch)
            .arrival_candidates
            .push(ArrivalCandidate {
                recording: id,
                ordinal: arrival.ordinal,
            });
    }

    pub(super) fn ready_logs(
        &mut self,
        pending: &[LoadedLogId],
    ) -> Vec<(LoadedLogId, Vec<LoadedFileId>)> {
        let registry = &mut self.0;
        let mut ready = Vec::new();
        for batch in registry.batches.values_mut() {
            batch.logs.retain(|id| pending.contains(id));
            if batch.progress.submitted.load(Ordering::Acquire)
                && batch.progress.recordings.load(Ordering::Acquire) == 0
            {
                for id in pending.iter().filter(|id| batch.logs.contains(id)) {
                    ready.push((*id, batch.ordered_candidates()));
                }
            }
        }
        registry.batches.retain(|_, batch| {
            !batch.progress.submitted.load(Ordering::Acquire)
                || !batch.logs.is_empty()
                || batch.progress.recordings.load(Ordering::Acquire) > 0
                || batch.progress.logs.load(Ordering::Acquire) > 0
        });
        ready
    }

    #[cfg(test)]
    pub(super) fn is_empty_for_test(&self) -> bool {
        self.0.batches.is_empty()
    }

    #[cfg(test)]
    pub(super) fn has_recording_work(&self) -> bool {
        self.0
            .batches
            .values()
            .any(|batch| batch.progress.recordings.load(Ordering::Acquire) > 0)
    }

    #[expect(
        clippy::panic,
        reason = "a missing batch for a live arrival token is an internal logic error"
    )]
    fn batch_for_live_arrival_mut(&mut self, id: AssociationBatchId) -> &mut BatchState {
        self.0
            .batches
            .get_mut(&id)
            .unwrap_or_else(|| panic!("a live arrival token must keep its association batch alive"))
    }
}

#[cfg(test)]
mod tests {
    use gt_loaded_files::{FileHistory, LoadedFiles};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::loose_log(false)]
    #[case::recording(true)]
    #[should_panic(expected = "a live arrival token must keep its association batch alive")]
    fn registration_panics_when_a_live_arrival_has_no_batch(#[case] recording: bool) {
        let mut batches = AssociationBatches::default();
        let submission = batches.begin_submission(std::iter::empty());
        let log = submission.log();
        let arrival = submission.recording();
        drop(submission);
        batches.0.batches.clear();
        if recording {
            let mut files = LoadedFiles::new();
            files.push(
                gt_test_utils::loaded_file_with_tracks(Vec::new()),
                FileHistory::None,
            );
            let id = files
                .view()
                .entries()
                .next()
                .expect("loaded recording")
                .id();
            batches.register_recording(&arrival, id);
        } else {
            batches.register_log(&log, LoadedLogId::new(0));
        }
    }

    #[test]
    fn dropping_an_empty_submission_removes_its_registry_entry() {
        let mut batches = AssociationBatches::default();
        let batch = batches.begin_submission(std::iter::empty());
        assert!(batches.ready_logs(&[]).is_empty());
        assert_eq!(batches.0.batches.len(), 1);
        drop(batch);
        assert!(batches.ready_logs(&[]).is_empty());
        assert!(batches.0.batches.is_empty());
    }

    #[test]
    fn submitted_batches_resolve_after_recording_operations_complete() {
        let mut batches = AssociationBatches::default();
        let batch = batches.begin_submission(std::iter::empty());
        let log = batch.log();
        let recording = batch.recording();
        let id = LoadedLogId::new(0);
        batches.register_log(&log, id);
        assert_eq!(batches.ready_logs(&[id]), Vec::new());
        drop(batch);
        assert_eq!(batches.ready_logs(&[id]), Vec::new());
        drop(recording);
        assert_eq!(batches.ready_logs(&[id]), vec![(id, Vec::new())]);
        assert!(batches.ready_logs(&[]).is_empty());
        assert_eq!(batches.0.batches.len(), 1);
        drop(log);
        assert!(batches.ready_logs(&[]).is_empty());
        assert!(batches.0.batches.is_empty());
    }

    #[rstest]
    fn repeated_recording_results_use_the_first_submission_ordinal(
        #[values(true, false)] reverse_completions: bool,
    ) {
        let mut files = LoadedFiles::new();
        for _ in 0..3 {
            files.push(
                gt_test_utils::loaded_file_with_tracks(Vec::new()),
                FileHistory::None,
            );
        }
        let ids: Vec<_> = files.view().entries().map(|entry| entry.id()).collect();
        let initial = *ids.first().expect("initial file");
        let first_arrival = *ids.get(2).expect("first arriving file");
        let second_arrival = *ids.get(1).expect("second arriving file");
        let mut batches = AssociationBatches::default();
        let batch = batches.begin_submission(std::iter::once(initial));
        let log = batch.log();
        let id = LoadedLogId::new(0);
        batches.register_log(&log, id);
        let mut arrivals = vec![
            (batch.recording(), first_arrival),
            (batch.recording(), second_arrival),
            (batch.recording(), first_arrival),
            (batch.recording(), initial),
        ];
        drop(batch);
        if reverse_completions {
            arrivals.reverse();
        }
        for (arrival, recording) in arrivals {
            batches.register_recording(&arrival, recording);
        }
        assert_eq!(
            batches.ready_logs(&[id]),
            vec![(id, vec![initial, first_arrival, second_arrival])]
        );
    }

    #[test]
    fn a_completed_batch_resolves_while_another_batch_has_recording_work() {
        let mut batches = AssociationBatches::default();
        let first = batches.begin_submission(std::iter::empty());
        let first_log = first.log();
        let first_recording = first.recording();
        let later = batches.begin_submission(std::iter::empty());
        let later_log = later.log();
        let later_recording = later.recording();
        let first_id = LoadedLogId::new(0);
        let later_id = LoadedLogId::new(1);
        batches.register_log(&first_log, first_id);
        batches.register_log(&later_log, later_id);
        drop(first);
        drop(later);
        drop(first_log);
        drop(later_log);
        drop(first_recording);
        assert_eq!(
            batches.ready_logs(&[later_id, first_id]),
            vec![(first_id, Vec::new())]
        );
        drop(later_recording);
        assert_eq!(
            batches.ready_logs(&[later_id]),
            vec![(later_id, Vec::new())]
        );
        assert!(batches.ready_logs(&[]).is_empty());
        assert!(batches.0.batches.is_empty());
    }
}
