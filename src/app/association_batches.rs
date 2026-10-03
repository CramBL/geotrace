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
}

impl AssociationSubmission {
    pub(super) fn recording(&self) -> RecordingArrival {
        self.progress.recordings.fetch_add(1, Ordering::Relaxed);
        RecordingArrival {
            batch: self.id,
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

struct BatchState {
    candidates: Vec<LoadedFileId>,
    logs: Vec<LoadedLogId>,
    progress: Arc<BatchProgress>,
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
                candidates: candidates.collect(),
                logs: Vec::new(),
                progress: Arc::clone(&progress),
            },
        );
        AssociationSubmission { id, progress }
    }

    pub(super) fn register_log(&mut self, arrival: &LogArrival, log: LoadedLogId) {
        if let Some(batch) = self.0.batches.get_mut(&arrival.batch)
            && !batch.logs.contains(&log)
        {
            batch.logs.push(log);
        }
    }

    pub(super) fn register_recording(&mut self, arrival: &RecordingArrival, id: LoadedFileId) {
        if let Some(batch) = self.0.batches.get_mut(&arrival.batch)
            && !batch.candidates.contains(&id)
        {
            batch.candidates.push(id);
        }
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
                    ready.push((*id, batch.candidates.clone()));
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
