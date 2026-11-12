//! The flow the association dialog fronts: the recording a log takes its
//! positions from, and the attachment that brings the log back with that
//! recording.

use std::mem;
use std::sync::Arc;

use gt_loaded_files::{LoadedFileId, RecordingNames};
use gt_log_view::{
    AttachmentSettingsCompletion, InitialLogSaveOutcome, LogAttachmentRef, LogSaveCompletion,
    LogSaveRequest, PositionSourceSelection, PositionSourceState,
};
use gt_pending_writes::WriteAccess;
use gt_store::{AttachedLog, DatabaseRef, DbError, LogAttachmentError};
use gt_ui_theme::EM_DASH;
use gt_ui_types::LoadedLogId;

use crate::settings::InitialPositionSourcePolicy;

use super::App;
use super::history_db::{ExistingLogAttachment, RestoredLogAttachment};
use super::load_failures::{LoadFailure, LoadFailureCause, LoadReadRequest, LoadSource};
use super::loader;
use super::log_viewer::association_dialog::{
    LogAssociationChoice, LogAssociationDialog, LogAssociationRequest,
};
use super::log_viewer::{AttachmentToLoad, YearsToResolve};

impl App {
    pub(super) fn resolve_initial_log_associations(&mut self) {
        let pending = self.logs.pending_initial_position_sources();
        let ready = self.association_batches.ready_logs(&pending);
        let shared = self.shared.borrow();
        let recordings = shared.loaded_files.view();
        for (id, candidates) in ready {
            if self
                .association_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.log() == id)
            {
                continue;
            }
            let Some(log) = self.logs.get_by_id(id) else {
                continue;
            };
            if !log
                .capabilities(WriteAccess::ReadOnly)
                .select_position_source
            {
                continue;
            }
            let ranked = log
                .rank_association_candidates(&recordings)
                .rank_with_recording_order(&candidates);
            let unambiguous = ranked.unambiguous_target();
            let years_resolved = log.parsed().chronology().is_resolved();
            if self.initial_position_source_policy == InitialPositionSourcePolicy::Ask
                && years_resolved
                && !ranked.ranked().is_empty()
            {
                if self.association_dialog.is_some() {
                    break;
                }
                self.association_dialog = Some(LogAssociationDialog::for_initial_selection(
                    id,
                    unambiguous,
                    candidates,
                ));
                break;
            }
            if let Err(error) = self.logs.select_position_source(
                id,
                unambiguous.map_or(
                    PositionSourceSelection::NoSource,
                    PositionSourceSelection::Recording,
                ),
                &recordings,
            ) {
                log::warn!("Selecting a log position source failed: {error}");
            }
        }
    }

    /// Draws the association dialog of the log it is open on, and applies what
    /// the user decided.
    pub(super) fn show_log_association_dialog(&mut self, ui: &egui::Ui) {
        let Some(mut dialog) = self.association_dialog.take() else {
            return;
        };
        let shared = self.shared.borrow();
        let recordings = shared.loaded_files.view();
        // A log unloaded while its dialog was open leaves nothing to decide.
        let Some(log) = self.logs.get_by_id(dialog.log()) else {
            drop(shared);
            return;
        };
        let names = RecordingNames::resolve(recordings, &shared.recording_name_template);
        let choice = dialog.show(
            ui.ctx(),
            log,
            recordings,
            &names,
            self.attachment_settings_write_access(),
            &mut self.initial_position_source_policy,
        );
        let duplicate_query = dialog
            .duplicate_query_to_send(recordings)
            .map(|db_ref| (db_ref, Arc::clone(log.parsed().text())));
        drop(shared);

        if let Some((db_ref, text)) = duplicate_query {
            self.history
                .find_duplicate_attachment(db_ref, dialog.log(), text);
        }
        match choice {
            None => self.association_dialog = Some(dialog),
            Some(choice) => self.apply_log_association_choice(&dialog, choice),
        }
    }

    /// Applies the requests the log viewer's footer made while it drew.
    pub(super) fn apply_log_viewer_requests(&mut self) {
        let requests = mem::take(&mut self.log_viewer_requests);
        if let Some(request) = requests.open_association_dialog {
            self.open_log_association_dialog(request);
        }
        if let Some(log_id) = requests.detach {
            self.detach_log(log_id);
        }
        if let Some(AttachmentToLoad { attachment, name }) = requests.load_attachment {
            self.history.load_attached_log(attachment, name);
        }
        if let Some(db_ref) = requests.open_recording {
            self.history.open(db_ref);
        }
        if let Some(id) = requests.retry_save {
            self.retry_log_save(id);
        }
        if let Some(years) = requests.resolve_years {
            self.resolve_log_years(years);
        }
    }

    pub(super) fn store_attached_log_settings_edits(&mut self) {
        let access = self.attachment_settings_write_access();
        for write in self.logs.prepare_settings_writes(access) {
            if let Some(completion) = self.history.store_attached_log_settings(write) {
                self.apply_attached_log_settings_outcome(completion);
            }
        }
    }

    pub(super) fn apply_attached_log_settings_outcome(
        &mut self,
        completion: AttachmentSettingsCompletion,
    ) {
        let error = completion.error().map(str::to_owned);
        if self.logs.acknowledge_settings(completion)
            && let Some(error) = error
        {
            log::warn!("Storing an attached log's settings failed: {error}");
        }
    }

    pub(super) fn apply_initial_log_save_outcome(&mut self, completion: LogSaveCompletion) {
        let recording = completion.recording().clone();
        let entry = completion
            .result()
            .ok()
            .map(|receipt| receipt.entry.clone());
        let id = completion.log();
        let outcome = self
            .logs
            .acknowledge_initial_save(completion, &self.shared.borrow().loaded_files.view());
        if let Some(entry) = entry {
            self.log_attachments.record_attachment(recording, entry);
        }
        match outcome {
            Some(InitialLogSaveOutcome::AlreadyLoaded(existing)) => {
                if self.log_viewer.open && self.log_viewer.selected_log() == Some(id) {
                    self.log_viewer.open_on_log(existing);
                }
            }
            Some(InitialLogSaveOutcome::Failed) => {
                if let Some(log) = self.logs.get_by_id(id)
                    && let Some(error) = log.save_error()
                {
                    log::warn!("Saving log {:?} failed: {error}", log.name());
                }
            }
            Some(InitialLogSaveOutcome::Saved(_)) | None => {}
        }
    }

    pub(super) fn attachment_settings_write_access(&self) -> WriteAccess {
        if self.pending_writes.rejection().is_some() {
            WriteAccess::ReadOnly
        } else {
            self.history.attachment_settings_write_access()
        }
    }

    fn resolve_log_years(
        &mut self,
        YearsToResolve {
            log: log_id,
            year_resolution,
        }: YearsToResolve,
    ) {
        let shared = self.shared.borrow();
        let recordings = shared.loaded_files.view();
        let Some(mut log) = self.logs.get_mut_by_id(log_id) else {
            return;
        };
        let applied = log.resolve_years_as(year_resolution, &recordings);
        self.log_viewer
            .record_chronology_apply_result(log_id, applied);
        if !applied {
            return;
        }
        self.log_viewer
            .preserve_source_position_after_chronology_change(&log, log_id);
        let needs_a_position_source = log.parsed().chronology().is_resolved()
            && log.anchor_key().is_none()
            && log
                .capabilities(WriteAccess::ReadOnly)
                .select_position_source;
        drop(shared);
        if needs_a_position_source {
            self.choose_position_source_of_loaded_log(log_id);
        }
    }

    /// Opens the association dialog on the log, or anchors it to the one recording it
    /// unambiguously overlaps, as [`App::resolve_initial_log_associations`] does for a loaded log.
    fn choose_position_source_of_loaded_log(&mut self, log_id: LoadedLogId) {
        let shared = self.shared.borrow();
        let recordings = shared.loaded_files.view();
        let Some(log) = self.logs.get_by_id(log_id) else {
            return;
        };
        if !log
            .capabilities(WriteAccess::ReadOnly)
            .select_position_source
        {
            return;
        }
        let candidates = log.rank_association_candidates(&recordings);
        if self.initial_position_source_policy == InitialPositionSourcePolicy::Ask
            && !candidates.ranked().is_empty()
        {
            drop(shared);
            self.open_log_association_dialog(LogAssociationRequest::SelectSourceAfterDates(log_id));
            return;
        }
        if let Err(error) = self.logs.select_position_source(
            log_id,
            candidates.unambiguous_target().map_or(
                PositionSourceSelection::NoSource,
                PositionSourceSelection::Recording,
            ),
            &recordings,
        ) {
            log::warn!("Selecting a log position source failed: {error}");
        }
    }

    /// Lists the attachments a recording opened from history holds, loads each
    /// of them, and reports the ones it could not read back.
    pub(super) fn restore_attached_logs(
        &mut self,
        recording: &DatabaseRef,
        attachments: Result<Vec<RestoredLogAttachment>, DbError>,
    ) {
        let attachments = match attachments {
            Ok(attachments) => attachments,
            Err(err) => {
                log::warn!("Listing a recording's attached logs failed: {err}");
                return;
            }
        };
        self.log_attachments.set_attachments_of(
            recording.clone(),
            attachments
                .iter()
                .map(|restored| restored.entry.clone())
                .collect(),
        );
        for RestoredLogAttachment { entry, log } in attachments {
            self.load_attachment_that_was_read_back(
                LogAttachmentRef {
                    recording: recording.clone(),
                    id: entry.id,
                },
                &entry.display_name(),
                log,
                loader::AttachedLogRequester::RecordingLoad,
            );
        }
    }

    /// Loads the one attachment the user opened in the viewer's list or in the
    /// history window, or reports why it did not come back.
    pub(super) fn load_the_attachment_the_user_opened(
        &mut self,
        attachment: LogAttachmentRef,
        name: &str,
        log: Result<AttachedLog, LogAttachmentError>,
    ) {
        self.load_attachment_that_was_read_back(
            attachment,
            name,
            log,
            loader::AttachedLogRequester::UserOpenedTheAttachment,
        );
    }

    fn load_attachment_that_was_read_back(
        &mut self,
        attachment: LogAttachmentRef,
        name: &str,
        log: Result<AttachedLog, LogAttachmentError>,
        requested_by: loader::AttachedLogRequester,
    ) {
        match log {
            Ok(attached) => self
                .loader
                .spawn_attached_log(attached, attachment, requested_by),
            Err(error @ LogAttachmentError::Database(DbError::UnreadableLogAttachment { .. })) => {
                let request = LoadReadRequest::ReadAttachment(attachment.clone());
                self.load_failures.push(
                    LoadFailure::new(
                        LoadSource::Attachment {
                            attachment,
                            name: name.to_owned(),
                        },
                        LoadFailureCause::ReadLogAttachment(Box::new(error)),
                    )
                    .with_original_read(Some(request)),
                );
            }
            Err(LogAttachmentError::MissingLog { .. }) => self.log_viewer.report_warning(format!(
                "{name} {EM_DASH} log missing from recording history"
            )),
            Err(err) => self
                .log_viewer
                .report_warning(format!("{name} {EM_DASH} {err}")),
        }
    }

    /// Takes the attachment off a log the database no longer holds one for.
    /// The log stays loaded either way.
    pub(super) fn apply_log_detach_outcome(
        &mut self,
        attachment: &LogAttachmentRef,
        _log_id: LoadedLogId,
        name: &str,
        result: Result<(), LogAttachmentError>,
    ) {
        match result {
            Ok(()) => {
                self.log_attachments.remove_attachment(attachment);
                self.logs.forget_attachment(attachment);
                self.toasts
                    .info(format!("Removed {name} from recording history"));
            }
            Err(err) => self.log_viewer.report_warning(format!(
                "Could not remove {name} from recording history {EM_DASH} {err}"
            )),
        }
    }

    /// Hands the dialog what the chosen recording already holds its log as.
    pub(super) fn set_duplicate_log_attachment(
        &mut self,
        log_id: LoadedLogId,
        recording: &DatabaseRef,
        existing: Result<Option<ExistingLogAttachment>, DbError>,
    ) {
        let existing = match existing {
            Ok(existing) => existing,
            Err(err) => {
                log::warn!("Looking for a duplicate log attachment failed: {err}");
                return;
            }
        };
        let Some(dialog) = self.association_dialog.as_mut() else {
            return;
        };
        if dialog.log() == log_id {
            dialog.set_duplicate_attachment(recording, existing);
        }
    }

    /// Opens the dialog on a log already loaded, on the recording it is
    /// associated with, or the only one overlapping it.
    fn open_log_association_dialog(&mut self, request: LogAssociationRequest) {
        let log_id = request.log();
        let shared = self.shared.borrow();
        let selected = self.logs.get_by_id(log_id).and_then(|log| {
            log.associated_recording().or_else(|| {
                log.rank_association_candidates(&shared.loaded_files.view())
                    .unambiguous_target()
            })
        });
        drop(shared);
        self.association_dialog = Some(LogAssociationDialog::for_request(request, selected));
    }

    fn apply_log_association_choice(
        &mut self,
        dialog: &LogAssociationDialog,
        choice: LogAssociationChoice,
    ) {
        let selection = match choice {
            LogAssociationChoice::SaveRequested(target) => {
                self.save_log_with_recording(dialog.log(), target);
                return;
            }
            LogAssociationChoice::SourceSelected(selection) => selection,
            LogAssociationChoice::Cancelled => {
                if !dialog.is_initial_selection() {
                    return;
                }
                if self.logs.get_by_id(dialog.log()).is_none_or(|log| {
                    log.position_source_state() != PositionSourceState::PendingInitialSelection
                }) {
                    return;
                }
                PositionSourceSelection::NoSource
            }
        };
        let result = self.logs.select_position_source(
            dialog.log(),
            selection,
            &self.shared.borrow().loaded_files.view(),
        );
        if let Err(error) = result {
            self.log_viewer.report_warning(error.to_string());
        }
    }

    fn save_log_with_recording(&mut self, log: LoadedLogId, target: LoadedFileId) {
        let access = self.attachment_settings_write_access();
        let request = self.logs.prepare_save(
            log,
            target,
            &self.shared.borrow().loaded_files.view(),
            access,
        );
        match request {
            Ok(request) => self.submit_initial_log_save(request),
            Err(error) => self.log_viewer.report_warning(error.to_string()),
        }
    }

    fn retry_log_save(&mut self, id: LoadedLogId) {
        let access = self.attachment_settings_write_access();
        if !access.allows_writing() {
            return;
        }
        if self
            .logs
            .get_by_id(id)
            .is_some_and(|log| log.attachment().is_some())
        {
            self.logs.retry_settings_save(id);
            return;
        }
        let request =
            self.logs
                .retry_initial_save(id, &self.shared.borrow().loaded_files.view(), access);
        match request {
            Ok(request) => self.submit_initial_log_save(request),
            Err(error) => log::warn!("Retrying a log save failed: {error}"),
        }
    }

    fn submit_initial_log_save(&mut self, request: LogSaveRequest) {
        if let Some(completion) = self.history.save_log(request) {
            self.apply_initial_log_save_outcome(completion);
        }
    }

    fn detach_log(&self, log_id: LoadedLogId) {
        let Some(log) = self.logs.get_by_id(log_id) else {
            return;
        };
        let Some(attachment) = log.attachment() else {
            return;
        };
        self.history
            .detach_log(attachment.clone(), log_id, log.name().to_owned());
    }
}
