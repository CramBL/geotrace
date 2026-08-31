//! The flow the association dialog fronts: the recording a log takes its
//! positions from, and the attachment that brings the log back with that
//! recording.

use std::mem;
use std::sync::Arc;

use gt_loaded_files::{LoadedFileId, RecordingNames};
use gt_log_view::LogAttachmentRef;
use gt_store::{DatabaseRef, DbError, LogAttachmentError};
use gt_ui_theme::EM_DASH;
use gt_ui_types::LoadedLogId;

use super::App;
use super::history_db::{RestoredLogAttachment, StoredLogAttachment};
use super::log_viewer::association_dialog::{LogAssociationChoice, LogAssociationDialog};

impl App {
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
            self.pending_writes.write_access(),
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
        if let Some(log_id) = requests.open_association_dialog {
            self.open_log_association_dialog(log_id);
        }
        if let Some(log_id) = requests.detach {
            self.detach_log(log_id);
        }
    }

    /// Writes back every attachment whose stored filter stack is behind the
    /// chips of the log it holds. A chip edit is stored as it is made: the
    /// stored stack is what a restored log comes back with.
    pub(super) fn store_attached_log_filter_edits(&mut self) {
        for (attachment, filters) in self.logs.take_filter_stack_edits_to_store() {
            self.history.set_attached_log_filters(attachment, filters);
        }
    }

    /// Loads the logs a recording opened from history carries, and reports the
    /// ones it could not read back.
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
        for RestoredLogAttachment { id, name, log } in attachments {
            match log {
                Ok(attached) => self.loader.spawn_attached_log(
                    attached,
                    LogAttachmentRef {
                        recording: recording.clone(),
                        id,
                    },
                ),
                Err(LogAttachmentError::MissingLog { .. }) => self
                    .log_viewer
                    .report_warning(format!("{name} {EM_DASH} attachment missing")),
                Err(err) => self
                    .log_viewer
                    .report_warning(format!("{name} {EM_DASH} {err}")),
            }
        }
    }

    /// Notes the attachment a log was stored as, or reports why it was not.
    pub(super) fn apply_log_attach_outcome(
        &mut self,
        log_id: LoadedLogId,
        name: &str,
        result: Result<StoredLogAttachment, LogAttachmentError>,
    ) {
        match result {
            Ok(StoredLogAttachment {
                attachment,
                filters,
            }) => {
                let shared = self.shared.borrow();
                match self.logs.get_mut_by_id(log_id) {
                    Some(log) => {
                        log.record_attachment(attachment, filters, &shared.loaded_files.view());
                    }
                    None => {
                        log::info!("The log {name:?} was unloaded before it finished attaching")
                    }
                }
            }
            Err(err) => self
                .log_viewer
                .report_warning(format!("Could not attach {name} {EM_DASH} {err}")),
        }
    }

    /// Takes the attachment off a log the database no longer holds one for.
    /// The log stays loaded either way.
    pub(super) fn apply_log_detach_outcome(
        &mut self,
        log_id: LoadedLogId,
        name: &str,
        result: Result<(), LogAttachmentError>,
    ) {
        match result {
            Ok(()) => {
                if let Some(log) = self.logs.get_mut_by_id(log_id) {
                    log.forget_attachment();
                }
                self.toasts
                    .info(format!("Removed the attachment of {name}"));
            }
            Err(err) => self.log_viewer.report_warning(format!(
                "Could not remove the attachment of {name} {EM_DASH} {err}"
            )),
        }
    }

    /// Hands the dialog what the chosen recording already holds its log as.
    pub(super) fn set_duplicate_log_attachment(
        &mut self,
        log_id: LoadedLogId,
        recording: &DatabaseRef,
        existing: Result<Option<String>, DbError>,
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
    fn open_log_association_dialog(&mut self, log_id: LoadedLogId) {
        let shared = self.shared.borrow();
        let selected = self.logs.get_by_id(log_id).and_then(|log| {
            log.associated_recording().or_else(|| {
                log.rank_association_candidates(&shared.loaded_files.view())
                    .unambiguous_target()
            })
        });
        drop(shared);
        self.association_dialog = Some(LogAssociationDialog::new(log_id, selected));
    }

    fn apply_log_association_choice(
        &mut self,
        dialog: &LogAssociationDialog,
        choice: LogAssociationChoice,
    ) {
        if dialog.dont_show_again() {
            self.ask_log_association_target = false;
        }
        let LogAssociationChoice::Confirmed { target, attach } = choice else {
            return;
        };
        let log_id = dialog.log();
        let shared = self.shared.borrow();
        let recordings = shared.loaded_files.view();
        if let Some(log) = self.logs.get_mut_by_id(log_id) {
            log.anchor_to_loaded_recording(target, &recordings);
        }
        drop(shared);
        if attach {
            self.attach_log_to_recording(log_id, target);
        }
    }

    fn attach_log_to_recording(&self, log_id: LoadedLogId, target: Option<LoadedFileId>) {
        let Some(log) = self.logs.get_by_id(log_id) else {
            return;
        };
        let shared = self.shared.borrow();
        let db_ref = target
            .and_then(|id| shared.loaded_files.view().entry_for_id(id))
            .and_then(|entry| entry.history().db_ref())
            .cloned();
        drop(shared);
        let Some(db_ref) = db_ref else {
            log::warn!(
                "Not attaching the log {:?}: the recording it was associated with is not in the history database",
                log.name()
            );
            return;
        };
        self.history.attach_log(
            db_ref,
            log_id,
            log.name().to_owned(),
            Arc::clone(log.parsed().text()),
            log.filters().to_stored_filters(),
        );
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
