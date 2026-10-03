use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gt_loaded_files::LoadedFileId;
use gt_ui_types::LoadedLogId;
use parking_lot::Mutex;

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
    implicit: Option<AssociationSubmission>,
    loaded: Vec<LoadedFileId>,
}

impl BatchRegistry {
    fn implicit_submission(&mut self) -> &AssociationSubmission {
        let batch = self
            .implicit
            .take()
            .unwrap_or_else(|| self.begin_submission());
        self.implicit.insert(batch)
    }

    fn begin_submission(&mut self) -> AssociationSubmission {
        let id = AssociationBatchId(self.next_id);
        self.next_id += 1;
        let progress = Arc::new(BatchProgress::default());
        self.batches.insert(
            id,
            BatchState {
                candidates: self.loaded.clone(),
                logs: Vec::new(),
                progress: Arc::clone(&progress),
            },
        );
        AssociationSubmission { id, progress }
    }
}

#[derive(Clone, Default)]
pub(super) struct AssociationBatches(Arc<Mutex<BatchRegistry>>);

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

impl LogArrival {
    pub(super) fn batch(&self) -> AssociationBatchId {
        self.batch
    }
}

impl Drop for LogArrival {
    fn drop(&mut self) {
        let previous = self.progress.logs.fetch_sub(1, Ordering::Release);
        debug_assert!(previous > 0);
    }
}

impl AssociationBatches {
    /// Each arrival set uses recordings already loaded at submission and its own results.
    /// Later batches do not extend this candidate set or delay its resolution.
    pub(super) fn begin_submission(&self) -> AssociationSubmission {
        self.0.lock().begin_submission()
    }

    pub(super) fn implicit_recording(&self) -> RecordingArrival {
        self.0.lock().implicit_submission().recording()
    }

    #[cfg(test)]
    pub(super) fn implicit_log(&self) -> LogArrival {
        self.0.lock().implicit_submission().log()
    }

    pub(super) fn seal_implicit(&self) {
        self.0.lock().implicit = None;
    }

    pub(super) fn register_log(&self, batch: AssociationBatchId, log: LoadedLogId) {
        let mut registry = self.0.lock();
        if let Some(batch) = registry.batches.get_mut(&batch)
            && !batch.logs.contains(&log)
        {
            batch.logs.push(log);
        }
    }

    pub(super) fn record_loaded(&self, arrival: Option<&RecordingArrival>, id: LoadedFileId) {
        let mut registry = self.0.lock();
        if !registry.loaded.contains(&id) {
            registry.loaded.push(id);
        }
        if let Some(arrival) = arrival
            && let Some(batch) = registry.batches.get_mut(&arrival.batch)
            && !batch.candidates.contains(&id)
        {
            batch.candidates.push(id);
        }
    }

    pub(super) fn sync_loaded(&self, ids: impl Iterator<Item = LoadedFileId>) {
        self.0.lock().loaded = ids.collect();
    }

    #[cfg(test)]
    pub(super) fn has_recording_work(&self) -> bool {
        self.0
            .lock()
            .batches
            .values()
            .any(|batch| batch.progress.recordings.load(Ordering::Acquire) > 0)
    }

    pub(super) fn ready_logs(
        &self,
        pending: &[LoadedLogId],
    ) -> Vec<(LoadedLogId, Vec<LoadedFileId>)> {
        let mut registry = self.0.lock();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_an_empty_submission_removes_its_registry_entry() {
        let batches = AssociationBatches::default();
        let batch = batches.begin_submission();
        assert!(batches.ready_logs(&[]).is_empty());
        assert_eq!(batches.0.lock().batches.len(), 1);
        drop(batch);
        assert!(batches.ready_logs(&[]).is_empty());
        assert!(batches.0.lock().batches.is_empty());
    }

    #[test]
    fn submitted_batches_resolve_after_recording_operations_complete() {
        let batches = AssociationBatches::default();
        let batch = batches.begin_submission();
        let log = batch.log();
        let recording = batch.recording();
        let id = LoadedLogId::new(0);
        batches.register_log(log.batch(), id);
        assert_eq!(batches.ready_logs(&[id]), Vec::new());
        drop(batch);
        assert_eq!(batches.ready_logs(&[id]), Vec::new());
        drop(recording);
        assert_eq!(batches.ready_logs(&[id]), vec![(id, Vec::new())]);
        assert!(batches.ready_logs(&[]).is_empty());
        assert_eq!(batches.0.lock().batches.len(), 1);
        drop(log);
        assert!(batches.ready_logs(&[]).is_empty());
        assert!(batches.0.lock().batches.is_empty());
    }

    #[test]
    fn a_completed_batch_resolves_while_another_batch_has_recording_work() {
        let batches = AssociationBatches::default();
        let first = batches.begin_submission();
        let first_log = first.log();
        let first_recording = first.recording();
        let later = batches.begin_submission();
        let later_log = later.log();
        let later_recording = later.recording();
        let first_id = LoadedLogId::new(0);
        let later_id = LoadedLogId::new(1);
        batches.register_log(first_log.batch(), first_id);
        batches.register_log(later_log.batch(), later_id);
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
        assert!(batches.0.lock().batches.is_empty());
    }
}
