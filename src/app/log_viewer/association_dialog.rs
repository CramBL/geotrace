use egui::{Button, Grid, Label, RichText, Sense, WidgetInfo, WidgetType};
use egui_phosphor::regular::INFO as ICON_INFO;
use gt_fmt::MIDDLE_DOT;
use gt_loaded_files::{LoadedFileId, LoadedFilesView, RecordingNames};
use gt_log_view::{AssociationCandidate, LoadedLog, LogCapabilities, PositionSourceSelection};
use gt_logfile::LogChronology;
use gt_pending_writes::WriteAccess;
use gt_store::DatabaseRef;
use gt_ui_components::HeldBodyLines;
use gt_ui_theme::EM_DASH;
use gt_ui_types::LoadedLogId;

use crate::app::anchored_dialog::{AnchoredDialog, AnchoredDialogKind};
use crate::app::history_db::ExistingLogAttachment;
use crate::app::modals::{DialogActionRow, DialogBody};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use crate::settings::InitialPositionSourcePolicy;

use super::NO_OVERLAP_HOVER;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum LogAssociationRequest {
    Save(LoadedLogId),
    SelectSource(LoadedLogId),
}

impl LogAssociationRequest {
    pub(in crate::app) fn log(self) -> LoadedLogId {
        match self {
            Self::Save(log) | Self::SelectSource(log) => log,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum LogAssociationChoice {
    Cancelled,
    SaveRequested(LoadedFileId),
    SourceSelected(PositionSourceSelection),
}

pub(in crate::app) struct LogAssociationDialog {
    log: LoadedLogId,
    purpose: DialogPurpose,
    selected: Option<LoadedFileId>,
    duplicate_query_sent_for: Option<DatabaseRef>,
    duplicate: Option<ExistingLogAttachment>,
}

impl LogAssociationDialog {
    pub(in crate::app) fn for_request(
        request: LogAssociationRequest,
        selected: Option<LoadedFileId>,
    ) -> Self {
        Self {
            log: request.log(),
            purpose: match request {
                LogAssociationRequest::Save(_) => DialogPurpose::Save,
                LogAssociationRequest::SelectSource(_) => DialogPurpose::SelectSource {
                    initial_candidates: None,
                },
            },
            selected,
            duplicate_query_sent_for: None,
            duplicate: None,
        }
    }

    pub(in crate::app) fn for_initial_selection(
        log: LoadedLogId,
        selected: Option<LoadedFileId>,
        candidates: Vec<LoadedFileId>,
    ) -> Self {
        Self {
            purpose: DialogPurpose::SelectSource {
                initial_candidates: Some(candidates),
            },
            ..Self::for_request(LogAssociationRequest::SelectSource(log), selected)
        }
    }

    pub(in crate::app) fn duplicate_query_to_send(
        &mut self,
        recordings: LoadedFilesView<'_>,
    ) -> Option<DatabaseRef> {
        if !matches!(self.purpose, DialogPurpose::Save) {
            return None;
        }
        let db_ref = self
            .selected
            .and_then(|id| recordings.entry_for_id(id))
            .and_then(|entry| entry.history().db_ref())?;
        if self.duplicate_query_sent_for.as_ref() == Some(db_ref) {
            return None;
        }
        self.duplicate_query_sent_for = Some(db_ref.clone());
        self.duplicate = None;
        Some(db_ref.clone())
    }

    pub(in crate::app) fn set_duplicate_attachment(
        &mut self,
        recording: &DatabaseRef,
        existing: Option<ExistingLogAttachment>,
    ) {
        if self.duplicate_query_sent_for.as_ref() == Some(recording) {
            self.duplicate = existing;
        }
    }

    pub(in crate::app) fn show(
        &mut self,
        ctx: &egui::Context,
        log: &LoadedLog,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
        write_access: WriteAccess,
        initial_position_source_policy: &mut InitialPositionSourcePolicy,
    ) -> Option<LogAssociationChoice> {
        let escape_pressed =
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut choice = escape_pressed.then_some(LogAssociationChoice::Cancelled);
        let names = super::recording_names_by_id(recordings, recording_names);
        let candidates = log.rank_association_candidates(&recordings);
        let candidates = match &self.purpose {
            DialogPurpose::SelectSource {
                initial_candidates: Some(scope),
            } => candidates.rank_with_recording_order(scope),
            DialogPurpose::Save
            | DialogPurpose::SelectSource {
                initial_candidates: None,
            } => candidates,
        };
        let saving = matches!(self.purpose, DialogPurpose::Save);
        let capabilities = log.capabilities(write_access);
        if !capabilities.select_position_source {
            self.selected = log.associated_recording();
        }
        let disabled_reason = self.operation_unavailable_reason(write_access, capabilities);
        let target_is_stored = self
            .selected
            .and_then(|id| recordings.entry_for_id(id))
            .is_some_and(|entry| entry.history().db_ref().is_some());
        let confirm_disabled = disabled_reason.or_else(|| {
            if !saving || target_is_stored {
                None
            } else if self.selected.is_some() {
                Some(ATTACH_UNSTORED_HOVER)
            } else {
                Some(ATTACH_NO_TARGET_HOVER)
            }
        });
        let unavailable_overlap = match log.parsed().chronology() {
            LogChronology::Unresolved(_) => Some(UNRESOLVED_OVERLAP_HOVER),
            LogChronology::Resolved(_) if log.entry_time_range().is_none() => {
                Some(NO_ABSOLUTE_TIME_HOVER)
            }
            LogChronology::Resolved(_) => None,
        };
        let (kind, title, confirm_label) = if saving {
            (AnchoredDialogKind::SaveLog, SAVE_TITLE, SAVE_CONFIRM_LABEL)
        } else {
            (AnchoredDialogKind::AssociateLog, TITLE, CONFIRM_LABEL)
        };
        let mut open = true;
        let mut confirmed = false;
        let mut automatically_choose_position_source =
            initial_position_source_policy.automatically_chooses_position_source();
        let confirm_description = match self.selected.and_then(|id| names.get(&id).copied()) {
            Some(name) if saving => format!("Save {} with {name}", log.name()),
            Some(name) => format!("Use {name} as the position source for this session"),
            None if saving => "Choose a recording to save this log with".to_owned(),
            None => "Use no position source for this session".to_owned(),
        };
        let dialog = AnchoredDialog::new(kind, title).with_close_button(&mut open);
        let regions = dialog.regions();
        let body = DialogBody::new(|ui| {
            ui.add(Label::new(log.name()).selectable(true).wrap());
            ui.label(if saving {
                "Choose a recording in history"
            } else {
                "Position source for this session"
            });
            if let Some(attachment) = log.attachment() {
                ui.add(
                    Label::new(format!(
                        "Saved with {}/{}",
                        gt_loaded_files::display_identity(&attachment.recording.identity).0,
                        attachment.recording.group_name
                    ))
                    .selectable(true)
                    .wrap(),
                );
            }
            ui.add_space(4.0);
            regions.freeze_at_open_ui(
                ui,
                CANDIDATES_REGION,
                HeldBodyLines::measured_content(),
                |ui| {
                    Grid::new("log_association_candidates")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            if !saving {
                                if ui
                                    .add_enabled(
                                        disabled_reason.is_none(),
                                        Button::selectable(
                                            self.selected.is_none(),
                                            NO_SOURCE_LABEL,
                                        ),
                                    )
                                    .on_hover_text(
                                        "Keep this log without positions in this session",
                                    )
                                    .on_disabled_hover_text(OPERATION_UNAVAILABLE_HOVER)
                                    .clicked()
                                {
                                    self.selected = None;
                                }
                                ui.label("");
                                ui.end_row();
                            }
                            for candidate in candidates.ranked() {
                                let stored = recordings
                                    .entry_for_id(candidate.recording)
                                    .is_some_and(|entry| entry.history().db_ref().is_some());
                                let row_disabled = disabled_reason.or_else(|| {
                                    (saving && !stored).then_some(ATTACH_UNSTORED_HOVER)
                                });
                                self.candidate_row_ui(
                                    ui,
                                    candidate,
                                    CandidatePresentation {
                                        name: names
                                            .get(&candidate.recording)
                                            .copied()
                                            .unwrap_or(EM_DASH),
                                        disabled_reason: row_disabled,
                                        unavailable_overlap,
                                    },
                                );
                            }
                        });
                },
            );
            if saving {
                ui.add_space(8.0);
                regions.freeze_at_open_ui(
                    ui,
                    STORED_ATTACHMENT_REGION,
                    HeldBodyLines::at_least(STORED_ATTACHMENT_LINES),
                    |ui| {
                        if let Some(existing) = self
                            .selected
                            .and_then(|id| recordings.entry_for_id(id))
                            .and_then(|entry| entry.history().db_ref())
                            .and_then(|recording| self.duplicate_attachment_of(recording))
                        {
                            ui.add(
                                Label::new(format!(
                                    r#"Already saved as "{}". Saving reuses this attachment."#,
                                    existing.name
                                ))
                                .wrap(),
                            );
                        }
                    },
                );
            }
        });
        let actions = DialogActionRow::buttons(|ui| {
            let confirm = ui
                .add_enabled(confirm_disabled.is_none(), Button::new(confirm_label))
                .on_disabled_hover_text(confirm_disabled.unwrap_or(OPERATION_UNAVAILABLE_HOVER));
            confirm.ctx.accesskit_node_builder(confirm.id, |node| {
                node.set_description(confirm_description.as_str())
            });
            confirmed = confirm.on_hover_text(confirm_description).clicked();
            if ui.button(CANCEL_LABEL).clicked() {
                choice = Some(LogAssociationChoice::Cancelled);
            }
        });
        if saving {
            dialog.show(ctx, body, actions);
        } else {
            dialog.show(
                ctx,
                body,
                actions.with_leading_control(|ui| {
                    let preference = ui.checkbox(
                        &mut automatically_choose_position_source,
                        AUTOMATIC_POSITION_SOURCE_DISPLAY_LABEL,
                    );
                    preference.widget_info(|| {
                        WidgetInfo::selected(
                            WidgetType::Checkbox,
                            ui.is_enabled(),
                            automatically_choose_position_source,
                            AUTOMATIC_POSITION_SOURCE_LABEL,
                        )
                    });
                    preference.on_hover_text(AUTOMATIC_POSITION_SOURCE_HOVER);
                }),
            );
        }
        if !saving {
            *initial_position_source_policy = InitialPositionSourcePolicy::from_automatic_selection(
                automatically_choose_position_source,
            );
        }
        if confirmed {
            choice = if saving {
                self.selected.map(LogAssociationChoice::SaveRequested)
            } else {
                Some(LogAssociationChoice::SourceSelected(self.selected.map_or(
                    PositionSourceSelection::NoSource,
                    PositionSourceSelection::Recording,
                )))
            };
        }
        if !open {
            choice = Some(LogAssociationChoice::Cancelled);
        }
        choice
    }

    pub(in crate::app) fn log(&self) -> LoadedLogId {
        self.log
    }

    pub(in crate::app) fn is_initial_selection(&self) -> bool {
        matches!(
            self.purpose,
            DialogPurpose::SelectSource {
                initial_candidates: Some(_)
            }
        )
    }

    fn candidate_row_ui(
        &mut self,
        ui: &mut egui::Ui,
        candidate: &AssociationCandidate,
        presentation: CandidatePresentation<'_>,
    ) {
        let overlapping = candidate.overlaps_the_log();
        let (overlap, hover) = match presentation.unavailable_overlap {
            Some(reason) => (format!("{ICON_INFO} {UNAVAILABLE_OVERLAP_LABEL}"), reason.to_owned()),
            None if overlapping => (format!("overlaps {} {MIDDLE_DOT} {} of log", gt_fmt::format_human_terse_duration(candidate.overlap),
                gt_fmt::format_fraction_percent(candidate.fraction_of_log)), "Every line within the association window of one of this recording's fixes takes its position".to_owned()),
            None => (NO_OVERLAP_LABEL.to_owned(), NO_OVERLAP_HOVER.to_owned()),
        };
        let name = RichText::new(presentation.name);
        let name = if matches!(self.purpose, DialogPurpose::Save)
            || overlapping
            || presentation.unavailable_overlap.is_some()
        {
            name
        } else {
            name.weak()
        };
        if ui
            .add_enabled(
                presentation.disabled_reason.is_none(),
                Button::selectable(self.selected == Some(candidate.recording), name),
            )
            .on_hover_text(&hover)
            .on_disabled_hover_text(
                presentation
                    .disabled_reason
                    .unwrap_or(OPERATION_UNAVAILABLE_HOVER),
            )
            .clicked()
        {
            self.selected = Some(candidate.recording);
        }
        let status = ui
            .add(Label::new(RichText::new(overlap).weak()).sense(Sense::focusable_noninteractive()))
            .on_hover_text(&hover);
        status
            .ctx
            .accesskit_node_builder(status.id, |node| node.set_description(hover.as_str()));
        if presentation.unavailable_overlap.is_some() && status.has_focus() {
            status.show_tooltip_text(hover);
        }
        ui.end_row();
    }

    fn operation_unavailable_reason(
        &self,
        access: WriteAccess,
        capabilities: LogCapabilities,
    ) -> Option<&'static str> {
        if matches!(self.purpose, DialogPurpose::Save) && !access.allows_writing() {
            return Some(READ_ONLY_RECORDING_HISTORY_HOVER);
        }
        let available = match self.purpose {
            DialogPurpose::Save => capabilities.save_with_recording,
            DialogPurpose::SelectSource { .. } => capabilities.select_position_source,
        };
        (!available).then_some(OPERATION_UNAVAILABLE_HOVER)
    }

    fn duplicate_attachment_of(&self, recording: &DatabaseRef) -> Option<&ExistingLogAttachment> {
        if self.duplicate_query_sent_for.as_ref() != Some(recording) {
            return None;
        }
        self.duplicate.as_ref()
    }
}

enum DialogPurpose {
    Save,
    SelectSource {
        initial_candidates: Option<Vec<LoadedFileId>>,
    },
}

#[derive(Clone, Copy, Debug)]
struct CandidatePresentation<'a> {
    name: &'a str,
    disabled_reason: Option<&'a str>,
    unavailable_overlap: Option<&'a str>,
}

pub(in crate::app) const OPERATION_UNAVAILABLE_HOVER: &str =
    "Remove the saved log from history or wait for the pending save to finish";
pub(in crate::app) const TITLE: &str = "Choose position source";
pub(in crate::app) const SAVE_TITLE: &str = "Save log with recording";
pub(in crate::app) const CONFIRM_LABEL: &str = "Use for session";
pub(in crate::app) const SAVE_CONFIRM_LABEL: &str = "Save log";
pub(in crate::app) const NO_SOURCE_LABEL: &str = "No position source";
pub(in crate::app) const UNAVAILABLE_OVERLAP_LABEL: &str = "Unavailable";
pub(in crate::app) const UNRESOLVED_OVERLAP_HOVER: &str =
    "Resolve this log's years to compare recording overlap";
const NO_ABSOLUTE_TIME_HOVER: &str =
    "This log has no usable absolute timestamps for recording overlap";
pub(in crate::app) const AUTOMATIC_POSITION_SOURCE_LABEL: &str =
    "Automatically choose an unambiguous position source";
const AUTOMATIC_POSITION_SOURCE_DISPLAY_LABEL: &str =
    "Automatically choose an\nunambiguous position source";
const CANCEL_LABEL: &str = "Cancel";
pub(in crate::app) const NO_OVERLAP_LABEL: &str = "no overlap";
const CANDIDATES_REGION: &str = "association_candidates";
const STORED_ATTACHMENT_REGION: &str = "stored_attachment_note";
const STORED_ATTACHMENT_LINES: u8 = 2;
const ATTACH_UNSTORED_HOVER: &str = "Save the recording in history before saving this log with it";
const ATTACH_NO_TARGET_HOVER: &str = "Choose a recording to save this log with";
pub(in crate::app) const AUTOMATIC_POSITION_SOURCE_HOVER: &str = "Skip the dialog for future log loads. Automatically use the recording as the position source \
     when exactly one loaded recording has tracks overlapping the log. With zero or multiple \
     overlapping recordings, leave the log without a position source. Turn this off to show the \
     dialog for future log loads.";

#[cfg(test)]
pub(in crate::app) mod tests;
