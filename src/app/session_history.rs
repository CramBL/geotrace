use gt_session_bridge::{ErrorCode, HistoryListing, ListHistoryRecordings, Reply};
use uuid::Uuid;

use super::App;
use super::history::RecordingListing;

impl App {
    pub(super) fn list_history_recordings(
        &mut self,
        session: Uuid,
        request: ListHistoryRecordings,
    ) -> Result<Reply, ErrorCode> {
        if self.shutdown_requested() {
            return Err(ErrorCode::ShuttingDown);
        }
        let ctx = self.ctx.clone();
        self.apply_finished_recording_work(&ctx);
        if self.storage_open.databases_pending().is_some() {
            return Err(ErrorCode::HistoryNotReady);
        }
        let database = self.history.path().ok_or(ErrorCode::HistoryUnavailable)?;
        self.history_window
            .request_recording_list_if_missing(&self.history);
        match self.history_window.recording_listing() {
            RecordingListing::Ready { revision, entries } => HistoryListing {
                database,
                entries,
                revision,
                session,
            }
            .page(request)
            .map(|page| Reply::HistoryRecordings { page }),
            _ if request.cursor.is_some() => Err(ErrorCode::InvalidHistoryCursor),
            RecordingListing::Failed => Err(ErrorCode::HistoryListingFailed),
            RecordingListing::Pending => Err(ErrorCode::HistoryNotReady),
        }
    }
}
