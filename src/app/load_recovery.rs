use std::path::PathBuf;

use gt_log_view::LogAttachmentRef;
use gt_store::{AttachedLog, LogAttachmentError};

use super::App;
use super::association_batches::{LogArrival, RecordingOperationOrigin};
use super::load_failures::{
    LoadFailure, LoadFailureCause, LoadFailureId, LoadProblemRequest, LoadReadRequest,
    LoadRetryAttempt, LoadSource,
};
use super::loader::{AttachedLogRequester, CompletedLoad, GtdLoadMode};

impl App {
    pub(super) fn retry_load_failure(&mut self, failure: LoadFailureId) {
        let Some(retry) = self.load_failures.begin_retry(failure) else {
            return;
        };
        let submitted = match retry.request {
            LoadReadRequest::OpenStoredRecording {
                recording,
                placement,
            } => self
                .history
                .retry_recording(recording, placement, retry.attempt),
            LoadReadRequest::ReadAttachment(attachment) => self.history.retry_attached_log(
                attachment,
                retry.source.into_filename().unwrap_or_default(),
                retry.attempt,
            ),
            LoadReadRequest::ReadLogFile(path) => {
                let arrival = self.recovery_log_arrival();
                let job = self.loader.spawn_log_path(path, arrival);
                self.loader.mark_retry(job, retry.attempt);
                true
            }
            LoadReadRequest::ReadRecordingFile(request) => {
                self.loader.spawn_gtd_path(
                    request.path,
                    request.config,
                    request.mode,
                    request.open,
                    RecordingOperationOrigin::Retry(retry.attempt),
                );
                true
            }
        };
        if !submitted {
            self.load_failures.cancel_retry(retry.attempt);
            self.toasts.error("Recording history is unavailable");
        }
    }

    pub(super) fn apply_load_problem_request(&mut self, request: LoadProblemRequest) {
        match request {
            LoadProblemRequest::ChooseFile(attempt) => {
                if self.load_failures.file_input_is_pending(attempt) {
                    self.loader.open_recovery_file_dialog(attempt);
                }
            }
            LoadProblemRequest::LoadText(attempt) => {
                if let Some(text) = self.load_failures.submit_text_input(attempt) {
                    let arrival = self.recovery_log_arrival();
                    self.loader.spawn_recovery_text(text, arrival, attempt);
                }
            }
            LoadProblemRequest::Retry(failure) => self.retry_load_failure(failure),
        }
    }

    pub(super) fn apply_recovery_file_selection(
        &mut self,
        attempt: LoadRetryAttempt,
        selection: Option<(PathBuf, GtdLoadMode)>,
    ) {
        let Some((path, mode)) = selection else {
            self.load_failures.cancel_input(attempt);
            return;
        };
        if !self.load_failures.accept_file_input(attempt, &path) {
            return;
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case(GTD_FILE_EXTENSION) {
            self.loader.spawn_gtd_path(
                path,
                self.processing_config,
                mode,
                None,
                RecordingOperationOrigin::Retry(attempt),
            );
        } else {
            let arrival = self.recovery_log_arrival();
            let job = self.loader.spawn_log_path(path, arrival);
            self.loader.mark_retry(job, attempt);
        }
    }

    pub(super) fn finish_attachment_retry_read(
        &mut self,
        attempt: LoadRetryAttempt,
        attachment: LogAttachmentRef,
        name: String,
        result: Result<AttachedLog, LogAttachmentError>,
    ) {
        if !self.load_failures.retry_is_pending(attempt) {
            return;
        }
        match result {
            Ok(attached) => {
                let job = self.loader.spawn_attached_log(
                    attached,
                    attachment,
                    AttachedLogRequester::RecordingLoad,
                );
                self.loader.mark_retry(job, attempt);
            }
            Err(error) => {
                log::error!("Failed to read attachment {attachment:?}: {error}");
                let request = LoadReadRequest::ReadAttachment(attachment.clone());
                let failure = LoadFailure::new(
                    LoadSource::Attachment { attachment, name },
                    LoadFailureCause::ReadLogAttachment(Box::new(error)),
                )
                .with_original_read(Some(request));
                self.load_failures.finish_retry(attempt, Err(failure));
            }
        }
    }

    pub(super) fn complete_load_retry(
        &mut self,
        mut completed: CompletedLoad,
    ) -> Option<CompletedLoad> {
        let Some(attempt) = completed.retry else {
            return Some(completed);
        };
        if !self.load_failures.retry_is_pending(attempt) {
            return None;
        }
        match completed.completion.into_retry_result() {
            Ok(completion) => {
                self.load_failures.finish_retry(attempt, Ok(()));
                completed.completion = completion;
                Some(completed)
            }
            Err(failure) => {
                log::error!("Failed to load {}: {}", failure.source, failure.cause);
                self.load_failures.finish_retry(attempt, Err(failure));
                None
            }
        }
    }

    fn recovery_log_arrival(&mut self) -> LogArrival {
        let batch = self.association_batches.begin_submission(
            self.shared
                .borrow()
                .loaded_files
                .view()
                .entries()
                .map(|entry| entry.id()),
        );
        batch.log()
    }
}

const GTD_FILE_EXTENSION: &str = "gtd";
