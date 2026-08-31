//! The viewer's list of logs: every loaded log under the recording it takes
//! its positions from, and the attachments of those recordings that no loaded
//! log holds.

use egui::{Button, Label, RichText};
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::PAPERCLIP as ICON_PAPERCLIP;
use egui_phosphor::regular::X as ICON_X;
use gt_loaded_files::{LoadedFilesView, RecordingNames};
use gt_log_view::{LoadedLog, LoadedLogs, LogAttachmentRef, RecordingKey, SessionLogAttachments};
use gt_types::FileIdx;
use gt_ui_theme::EM_DASH;
use gt_ui_types::LoadedLogId;

use super::AttachmentToLoad;

/// Heading of the group holding the logs that take their positions from no
/// recording.
pub(super) const NOT_ANCHORED_HEADING: &str = "Not anchored";

pub(super) const UNLOAD_HOVER: &str = "Unload this log";

const VISIBILITY_HOVER: &str = "Draw this log's matches on the map";

pub(in crate::app) const LOAD_ATTACHMENT_LABEL: &str = "Load";

const LOAD_ATTACHMENT_HOVER: &str = "Read this log back out of the recording and load it";

/// One group of the list: a loaded recording with the logs anchored to it and
/// the attachments it holds that are not loaded, or the logs anchored to no
/// recording.
#[derive(Debug, PartialEq)]
pub(super) struct LogGroup {
    /// How the app names the recording these rows belong to, `None` for the
    /// group of logs anchored to no recording.
    pub recording: Option<String>,

    pub rows: Vec<LogRow>,
}

#[derive(Debug, PartialEq)]
pub(super) enum LogRow {
    Loaded(LoadedLogRow),
    Available(AvailableAttachmentRow),
}

/// A log of this session, selectable and unloadable, drawn on the map while
/// its own toggle is on.
#[derive(Debug, PartialEq)]
pub(super) struct LoadedLogRow {
    pub id: LoadedLogId,
    pub name: String,
    pub visible: bool,

    /// The recording this log is stored with, `None` for one stored nowhere.
    pub attached_to: Option<String>,
}

/// An attachment of a loaded recording that no loaded log holds.
#[derive(Debug, PartialEq)]
pub(super) struct AvailableAttachmentRow {
    pub attachment: LogAttachmentRef,
    pub name: String,

    /// How the app names the recording holding this attachment.
    pub recording: String,
}

/// What the rows requested while they drew.
#[derive(Debug, Default)]
pub(super) struct LogListActions {
    pub unload: Option<LoadedLogId>,
    pub load_attachment: Option<AttachmentToLoad>,
}

/// Every loaded log under the recording it is anchored to, in the order the
/// recordings loaded, followed by the logs anchored to no recording.
pub(super) fn group_logs_by_recording(
    logs: &LoadedLogs,
    recordings: LoadedFilesView<'_>,
    recording_names: &RecordingNames,
    attachments: &SessionLogAttachments,
) -> Vec<LogGroup> {
    let mut groups = Vec::new();
    let mut grouped: Vec<LoadedLogId> = Vec::new();
    for (index, entry) in recordings.entries().enumerate() {
        let recording = recording_names
            .get(FileIdx::new(index))
            .unwrap_or(EM_DASH)
            .to_owned();
        let key = RecordingKey::of_loaded_recording(entry);
        let anchored: Vec<(LoadedLogId, &LoadedLog)> =
            logs.anchored_to(std::slice::from_ref(&key)).collect();
        grouped.extend(anchored.iter().map(|(id, _)| *id));
        let mut rows: Vec<LogRow> = anchored
            .into_iter()
            .map(|(id, log)| LogRow::of_loaded_log(id, log, Some(&recording)))
            .collect();
        if let Some(db_ref) = entry.history().db_ref() {
            rows.extend(attachments.of_recording(db_ref).iter().filter_map(|entry| {
                let attachment = LogAttachmentRef {
                    recording: db_ref.clone(),
                    id: entry.id,
                };
                (!logs.any_loaded_log_holds(&attachment)).then(|| {
                    LogRow::Available(AvailableAttachmentRow {
                        attachment,
                        name: entry.attachment.name.clone(),
                        recording: recording.clone(),
                    })
                })
            }));
        }
        if !rows.is_empty() {
            groups.push(LogGroup {
                recording: Some(recording),
                rows,
            });
        }
    }

    let anchorless: Vec<LogRow> = logs
        .iter_with_ids()
        .filter(|(id, _)| !grouped.contains(id))
        .map(|(id, log)| LogRow::of_loaded_log(id, log, None))
        .collect();
    if !anchorless.is_empty() {
        groups.push(LogGroup {
            recording: None,
            rows: anchorless,
        });
    }
    groups
}

/// Draws `groups`, one row per log and per available attachment.
///
/// A session holding one log alone gets that row without a heading over it.
pub(super) fn log_list_ui(
    ui: &mut egui::Ui,
    groups: &[LogGroup],
    logs: &mut LoadedLogs,
    selected: &mut Option<LoadedLogId>,
) -> LogListActions {
    let headings = groups.len() > 1 || groups.first().is_some_and(|group| group.rows.len() > 1);
    let mut actions = LogListActions::default();
    for (index, group) in groups.iter().enumerate() {
        if !headings {
            rows_ui(ui, group, logs, selected, &mut actions);
            continue;
        }
        ui.label(
            RichText::new(group.recording.as_deref().unwrap_or(NOT_ANCHORED_HEADING)).strong(),
        );
        ui.indent(("log_viewer_group", index), |ui| {
            rows_ui(ui, group, logs, selected, &mut actions);
        });
    }
    actions
}

fn rows_ui(
    ui: &mut egui::Ui,
    group: &LogGroup,
    logs: &mut LoadedLogs,
    selected: &mut Option<LoadedLogId>,
    actions: &mut LogListActions,
) {
    for row in &group.rows {
        match row {
            LogRow::Loaded(loaded) => {
                if loaded.ui(ui, logs, selected) {
                    actions.unload = Some(loaded.id);
                }
            }
            LogRow::Available(available) => {
                if available.ui(ui) {
                    actions.load_attachment = Some(AttachmentToLoad {
                        attachment: available.attachment.clone(),
                        name: available.name.clone(),
                    });
                }
            }
        }
    }
}

impl LogRow {
    /// `recording` is how the app names the group's recording, `None` for the
    /// group of logs anchored to none. An attached log listed there names the
    /// identity its attachment holds instead.
    fn of_loaded_log(id: LoadedLogId, log: &LoadedLog, recording: Option<&str>) -> Self {
        Self::Loaded(LoadedLogRow {
            id,
            name: log.name().to_owned(),
            visible: log.is_visible(),
            attached_to: log.attachment().map(|attachment| match recording {
                Some(recording) => recording.to_owned(),
                None => gt_loaded_files::display_identity(&attachment.recording.identity)
                    .0
                    .to_owned(),
            }),
        })
    }
}

impl LoadedLogRow {
    /// Whether the row's unload button was pressed. The visibility toggle acts
    /// on the spot, whether or not the row is the selected one.
    fn ui(
        &self,
        ui: &mut egui::Ui,
        logs: &mut LoadedLogs,
        selected: &mut Option<LoadedLogId>,
    ) -> bool {
        ui.horizontal(|ui| {
            let eye = if self.visible {
                ICON_EYE
            } else {
                ICON_EYE_SLASH
            };
            if ui
                .selectable_label(self.visible, eye)
                .on_hover_text(VISIBILITY_HOVER)
                .clicked()
                && let Some(log) = logs.get_mut_by_id(self.id)
            {
                log.set_visible(!self.visible);
            }
            // A long name truncates before the eye and unload buttons do, so
            // those stay full size.
            if ui
                .add(Button::selectable(*selected == Some(self.id), self.name.as_str()).truncate())
                .clicked()
            {
                *selected = Some(self.id);
            }
            if let Some(recording) = &self.attached_to {
                ui.label(ICON_PAPERCLIP)
                    .on_hover_text(format!("Stored with the recording {recording}"));
            }
            let unload_hover = match &self.attached_to {
                Some(recording) => {
                    format!("Unload this log. It stays attached to {recording}.")
                }
                None => UNLOAD_HOVER.to_owned(),
            };
            ui.small_button(ICON_X)
                .on_hover_text(unload_hover)
                .clicked()
        })
        .inner
    }
}

impl AvailableAttachmentRow {
    /// Whether the row's load button was pressed.
    fn ui(&self, ui: &mut egui::Ui) -> bool {
        ui.horizontal(|ui| {
            ui.add(
                Label::new(RichText::new(format!("{ICON_PAPERCLIP} {}", self.name)).weak())
                    .truncate(),
            )
            .on_hover_text(format!(
                "Stored with the recording {}, not loaded in this session",
                self.recording
            ));
            ui.add(Button::new(LOAD_ATTACHMENT_LABEL).small())
                .on_hover_text(LOAD_ATTACHMENT_HOVER)
                .clicked()
        })
        .inner
    }
}
