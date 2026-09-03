//! The dialog choosing the recording a log associates against, and whether the
//! log is stored with that recording in history.

use egui::{Checkbox, Grid, Label, RichText};
use gt_fmt::MIDDLE_DOT;
use gt_loaded_files::{LoadedFileId, LoadedFilesView, RecordingNames};
use gt_log_view::{AssociationCandidate, LoadedLog};
use gt_pending_writes::WriteAccess;
use gt_store::DatabaseRef;
use gt_ui_theme::EM_DASH;
use gt_ui_types::LoadedLogId;

use crate::app::anchored_dialog::{
    AnchoredDialog, AnchoredDialogKind, DialogRegions, HeldBodyLines,
};
use crate::app::history_db::ExistingLogAttachment;
use crate::app::modals::{DialogActionRow, DialogBody};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

#[cfg(test)]
pub(in crate::app) mod tests;

use super::{NO_OVERLAP_HOVER, recording_names_by_id};

pub(in crate::app) const TITLE: &str = "Associate log";

pub(in crate::app) const ATTACH_LABEL: &str = "Also attach to this recording in history";

pub(in crate::app) const DONT_SHOW_AGAIN_LABEL: &str = "Don't show this again";

pub(in crate::app) const CONFIRM_LABEL: &str = "Associate";

const CANCEL_LABEL: &str = "Cancel";

pub(in crate::app) const NO_OVERLAP_LABEL: &str = "no overlap";

/// The region of the body listing the recordings, which keeps the height it
/// had when the dialog opened.
const CANDIDATES_REGION: &str = "association_candidates";

/// The region of the body the duplicate-attachment result lands in.
const STORED_ATTACHMENT_REGION: &str = "stored_attachment_note";

/// Lines the [`STORED_ATTACHMENT_REGION`] holds from the frame the dialog
/// opens, which is what the note takes for an attachment name of ordinary
/// length.
const STORED_ATTACHMENT_LINES: u8 = 2;

const ATTACH_HOVER: &str = "Store this log with the recording, so it comes back with its filters when the recording is \
     opened from history";

const ATTACH_UNSTORED_HOVER: &str =
    "Only a recording stored in the history database can hold an attachment";

const ATTACH_NO_TARGET_HOVER: &str = "Choose a recording to attach this log to";

const DONT_SHOW_AGAIN_HOVER: &str = "Associate a loading log by itself when exactly one loaded recording overlaps it, and leave it \
     untargeted otherwise. Switchable back on under Processing in the settings.";

const CONFIRM_HOVER: &str = "Take this log's positions from the chosen recording";

/// What the user decided in the association dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum LogAssociationChoice {
    /// Take the log's positions from `target`, and store it with that
    /// recording when `attach`.
    Confirmed {
        target: Option<LoadedFileId>,
        attach: bool,
    },

    /// Leave the log associated as it is, and store nothing.
    Cancelled,
}

/// The association dialog of one log: shown when a log finishes loading, and
/// from the viewer footer for a log already loaded.
pub(in crate::app) struct LogAssociationDialog {
    log: LoadedLogId,
    selected: Option<LoadedFileId>,
    attach: bool,
    dont_show_again: bool,

    /// The recording the duplicate-attachment query was sent for.
    duplicate_query_sent_for: Option<DatabaseRef>,

    /// The attachment under which the selected recording already holds this
    /// exact log.
    duplicate: Option<ExistingLogAttachment>,
}

impl LogAssociationDialog {
    /// `selected` is the recording the dialog opens on: the log's target when
    /// it has one, else the only recording overlapping it.
    pub(in crate::app) fn new(log: LoadedLogId, selected: Option<LoadedFileId>) -> Self {
        Self {
            log,
            selected,
            attach: false,
            dont_show_again: false,
            duplicate_query_sent_for: None,
            duplicate: None,
        }
    }

    pub(in crate::app) fn log(&self) -> LoadedLogId {
        self.log
    }

    /// Whether the user chose to decide without the dialog from here on.
    pub(in crate::app) fn dont_show_again(&self) -> bool {
        self.dont_show_again
    }

    /// The recording to query the history database about, once per recording
    /// the user selects: whether it already holds this exact log.
    pub(in crate::app) fn duplicate_query_to_send(
        &mut self,
        recordings: LoadedFilesView<'_>,
    ) -> Option<DatabaseRef> {
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

    /// Takes the result of [`duplicate_query_to_send`](Self::duplicate_query_to_send),
    /// `existing` being the attachment that recording already holds this log
    /// as.
    pub(in crate::app) fn set_duplicate_attachment(
        &mut self,
        recording: &DatabaseRef,
        existing: Option<ExistingLogAttachment>,
    ) {
        if self.duplicate_query_sent_for.as_ref() == Some(recording) {
            self.duplicate = existing;
        }
    }

    /// The attachment `recording` already holds this log as, `None` while the
    /// query for that recording is pending or returned none.
    pub(in crate::app) fn duplicate_attachment_of(
        &self,
        recording: &DatabaseRef,
    ) -> Option<&ExistingLogAttachment> {
        if self.duplicate_query_sent_for.as_ref() != Some(recording) {
            return None;
        }
        self.duplicate.as_ref()
    }

    /// Draws the dialog, returning `None` while the user has not decided.
    pub(in crate::app) fn show(
        &mut self,
        ctx: &egui::Context,
        log: &LoadedLog,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
        write_access: WriteAccess,
    ) -> Option<LogAssociationChoice> {
        let escape_pressed =
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut choice = escape_pressed.then_some(LogAssociationChoice::Cancelled);

        let names = recording_names_by_id(recordings, recording_names);
        let candidates = log.rank_association_candidates(&recordings);
        let attachable = write_access.allows_writing()
            && self
                .selected
                .and_then(|id| recordings.entry_for_id(id))
                .is_some_and(|entry| entry.history().db_ref().is_some());
        self.attach &= attachable;

        let mut open = true;
        // Read after the window renders: the body's tickbox writes `attach`.
        let mut confirmed = false;
        // The body borrows `self`: the action row's tickbox writes a local.
        let mut dont_show_again = self.dont_show_again;
        let dialog = AnchoredDialog::new(AnchoredDialogKind::AssociateLog, TITLE)
            .with_close_button(&mut open);
        let regions = dialog.regions();
        dialog.show(
            ctx,
            DialogBody::new(|ui| {
                ui.add(
                    Label::new(format!(
                        "Which recording should {} take its positions from?",
                        log.name()
                    ))
                    .wrap(),
                );
                ui.add_space(4.0);
                regions.frozen_at_open(
                    ui,
                    CANDIDATES_REGION,
                    HeldBodyLines::what_the_content_took(),
                    |ui| {
                        Grid::new("log_association_candidates")
                            .num_columns(2)
                            .spacing([16.0, 4.0])
                            .show(ui, |ui| {
                                for candidate in candidates.ranked() {
                                    let name =
                                        names.get(&candidate.recording).copied().unwrap_or(EM_DASH);
                                    self.candidate_row_ui(ui, candidate, name);
                                }
                            });
                    },
                );
                ui.add_space(8.0);
                self.attach_ui(ui, regions, attachable, write_access);
            }),
            DialogActionRow::buttons(|ui| {
                confirmed = ui
                    .button(CONFIRM_LABEL)
                    .on_hover_text(CONFIRM_HOVER)
                    .clicked();
                if ui.button(CANCEL_LABEL).clicked() {
                    choice = Some(LogAssociationChoice::Cancelled);
                }
            })
            .with_leading_control(|ui| {
                ui.checkbox(&mut dont_show_again, DONT_SHOW_AGAIN_LABEL)
                    .on_hover_text(DONT_SHOW_AGAIN_HOVER);
            }),
        );

        self.dont_show_again = dont_show_again;
        if confirmed {
            choice = Some(LogAssociationChoice::Confirmed {
                target: self.selected,
                attach: self.attach,
            });
        }
        if !open {
            choice = Some(LogAssociationChoice::Cancelled);
        }
        choice
    }

    /// One recording the log could take its positions from, with how much of
    /// the log it ran alongside.
    fn candidate_row_ui(
        &mut self,
        ui: &mut egui::Ui,
        candidate: &AssociationCandidate,
        name: &str,
    ) {
        let overlapping = candidate.overlaps_the_log();
        let (label, overlap, hover) = if overlapping {
            (
                RichText::new(name),
                format!(
                    "overlaps {} {MIDDLE_DOT} {} of log",
                    gt_fmt::format_human_terse_duration(candidate.overlap),
                    gt_fmt::format_fraction_percent(candidate.fraction_of_log)
                ),
                "Every line within the association window of one of this recording's fixes takes \
                 its position"
                    .to_owned(),
            )
        } else {
            (
                RichText::new(name).weak(),
                NO_OVERLAP_LABEL.to_owned(),
                NO_OVERLAP_HOVER.to_owned(),
            )
        };
        let selected = self.selected == Some(candidate.recording);
        if ui
            .selectable_label(selected, label)
            .on_hover_text(&hover)
            .clicked()
        {
            self.selected = Some(candidate.recording);
        }
        ui.label(RichText::new(overlap).weak())
            .on_hover_text(&hover);
        ui.end_row();
    }

    /// The attach tickbox, under the note naming an attachment the chosen
    /// recording already holds this log as.
    fn attach_ui(
        &mut self,
        ui: &mut egui::Ui,
        regions: DialogRegions,
        attachable: bool,
        write_access: WriteAccess,
    ) {
        regions.frozen_at_open(
            ui,
            STORED_ATTACHMENT_REGION,
            HeldBodyLines::at_least(STORED_ATTACHMENT_LINES),
            |ui| {
                if let Some(existing) = &self.duplicate {
                    ui.add(
                        Label::new(format!(
                            "This recording already holds this log as \"{}\". Attaching reuses \
                             that attachment.",
                            existing.name
                        ))
                        .wrap(),
                    );
                }
            },
        );
        let attach = ui.add_enabled(attachable, Checkbox::new(&mut self.attach, ATTACH_LABEL));
        if attachable {
            attach.on_hover_text(ATTACH_HOVER);
        } else if !write_access.allows_writing() {
            attach.on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER);
        } else if self.selected.is_some() {
            attach.on_disabled_hover_text(ATTACH_UNSTORED_HOVER);
        } else {
            attach.on_disabled_hover_text(ATTACH_NO_TARGET_HOVER);
        }
    }
}
