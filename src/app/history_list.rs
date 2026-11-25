use gt_store::{DbError, HistoryEntries, NavPointTimeRange, RecordingEntry};

use super::history_db::{HistoryWorker, ListRequestId};

#[derive(Default)]
pub(super) struct HistoryListCache {
    entries: Option<HistoryEntries>,
    revision: u64,
    pending_request: Option<ListRequestId>,
    last_request_failed: bool,
}

impl HistoryListCache {
    pub(super) fn request_if_missing(&mut self, worker: &HistoryWorker) {
        if self.entries.is_none() && self.pending_request.is_none() && worker.available() {
            self.pending_request = worker.list();
            if self.pending_request.is_none() {
                self.last_request_failed = true;
            }
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.entries = None;
        self.pending_request = None;
        self.last_request_failed = false;
    }

    pub(super) fn accept_response(
        &mut self,
        request: ListRequestId,
        entries: Result<HistoryEntries, DbError>,
    ) -> Option<Result<&[RecordingEntry], DbError>> {
        if self.pending_request != Some(request) {
            return None;
        }
        Some(match entries {
            Ok(entries) => Ok(self.replace_entries(entries)),
            Err(error) => {
                self.pending_request = None;
                self.last_request_failed = true;
                Err(error)
            }
        })
    }

    #[cfg(test)]
    pub(super) fn set_entries_for_test(
        &mut self,
        entries: Vec<RecordingEntry>,
    ) -> &[RecordingEntry] {
        self.replace_entries(HistoryEntries::new(entries))
    }

    pub(super) fn view(&self) -> RecordingListing<'_> {
        if let Some(entries) = &self.entries {
            RecordingListing::Ready {
                revision: self.revision,
                entries,
            }
        } else if self.last_request_failed {
            RecordingListing::Failed
        } else {
            RecordingListing::Pending
        }
    }

    pub(super) fn latest_recording(&self) -> Option<&RecordingEntry> {
        self.entries.as_ref()?.iter().max_by(|a, b| {
            a.meta
                .time_range
                .map(NavPointTimeRange::start_us)
                .cmp(&b.meta.time_range.map(NavPointTimeRange::start_us))
                .then_with(|| a.db_ref.identity.cmp(&b.db_ref.identity))
                .then_with(|| a.db_ref.group_name.cmp(&b.db_ref.group_name))
        })
    }

    fn replace_entries(&mut self, entries: HistoryEntries) -> &[RecordingEntry] {
        self.revision = self.revision.wrapping_add(1);
        self.pending_request = None;
        self.last_request_failed = false;
        self.entries.insert(entries)
    }
}

pub(super) enum RecordingListing<'a> {
    Failed,
    Pending,
    Ready {
        revision: u64,
        entries: &'a HistoryEntries,
    },
}
