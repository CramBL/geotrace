//! The log viewer window: the loaded logs, the parse summary each of them
//! expands into, and the virtualized table of the selected log's lines.

pub(super) mod association_dialog;
mod association_window;
pub(super) mod filters;
mod line_table;
mod summary_panel;
#[cfg(test)]
mod tests;

use egui::{Button, ComboBox, DragValue, Label, RichText, Sides, Window};
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::PAPERCLIP as ICON_PAPERCLIP;
use egui_phosphor::regular::X as ICON_X;
use gt_loaded_files::{LoadedFileId, LoadedFilesView, RecordingNames};
use gt_log_view::{LoadedLog, LoadedLogs};
use gt_pending_writes::WriteAccess;
use gt_types::FileIdx;
use gt_ui_theme::EM_DASH;
use gt_ui_types::{LoadedLogId, LogMatchHover};
use rustc_hash::FxHashMap;
use strum::IntoEnumIterator as _;

use association_window::AssociationWindowUnit;
use line_table::LineTableRequests;

use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

/// The three ways a log gets in, shown by the viewer's empty state and by the
/// drag-and-drop overlay alike.
pub(super) const LOG_LOAD_HINT: &str = "Open a log file, drop it here, or paste log text (Ctrl+V)";

/// How the viewer writes a moment, in the table and in the summary panel alike.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

const SUMMARY_HOVER: &str = "Show what the parse read from this log";

const ASSOCIATION_WINDOW_HOVER: &str = "Furthest a line's timestamp may be from a fix of the association target for \
     the line to take its position";

/// Why a recording that ran at no time the log covers is still a choice: a
/// clock-skewed source is a recording too.
pub(super) const NO_OVERLAP_HOVER: &str =
    "This recording ran at no time the log covers: every line would stay unassociated";

pub(in crate::app) const ATTACH_LABEL: &str = "Attach to recording…";

const ATTACH_HOVER: &str = "Choose the recording this log belongs to, and store it there";

const ATTACH_NO_RECORDING_HOVER: &str = "Load a recording to attach this log to it";

pub(in crate::app) const DETACH_LABEL: &str = "Remove attachment";

const DETACH_HOVER: &str =
    "Take this log out of the recording in history. It stays loaded in this session.";

const DETACH_UNATTACHED_HOVER: &str = "This log is not stored with a recording in history";

const NOTICE_DISMISS_HOVER: &str = "Dismiss this warning";

/// Fixed width for the unit dropdown: a wider unit label must not shift the
/// controls beside it.
const UNIT_DROPDOWN_WIDTH_PX: f32 = 52.0;

/// The floating log viewer: which log it shows, how much of that log's parse
/// summary is unfolded, and the unit its association window is entered in.
pub(super) struct LogViewerWindow {
    pub open: bool,

    /// Indexes the loaded logs. Clamped against them before every render.
    selected: usize,

    summary_expanded: bool,
    association_window_unit: AssociationWindowUnit,

    /// When the shown log's filters started scanning, for the note the viewer
    /// shows once a scan runs long enough to notice.
    query_pending_since: Option<f64>,

    /// The table row the summary panel asked to scroll to, consumed by the
    /// table on the frame after it was asked for.
    scroll_to_row: Option<usize>,

    /// What went wrong with this session's attachments, shown until dismissed.
    notices: Vec<String>,
}

/// The app state the viewer reads and writes while it renders.
pub(super) struct LogViewerContext<'a> {
    pub recordings: LoadedFilesView<'a>,
    pub recording_names: &'a RecordingNames,
    pub map_center_request: &'a mut Option<(f64, f64)>,
    /// The hexagon the map found under the cursor, and the row this viewer
    /// puts under it in return.
    pub log_hover: &'a mut LogMatchHover,
    pub requests: &'a mut LogViewerRequests,

    /// Whether this session writes to the recording history, which is what
    /// attaching a log to a recording and taking it back out do.
    pub write_access: WriteAccess,

    /// Whether the association dialog is over the viewer, which then takes the
    /// Escape press for itself.
    pub dialog_open: bool,
}

/// What the viewer requests of the app for the log it is showing. The app owns
/// the history database, the dialog, and the log's association target.
#[derive(Debug, Default)]
pub(super) struct LogViewerRequests {
    /// Open the association dialog on this log.
    pub open_association_dialog: Option<LoadedLogId>,

    /// Remove this log's attachment from the history database.
    pub detach: Option<LoadedLogId>,
}

impl LogViewerWindow {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            selected: 0,
            summary_expanded: false,
            association_window_unit: AssociationWindowUnit::Seconds,
            query_pending_since: None,
            scroll_to_row: None,
            notices: Vec::new(),
        }
    }

    /// Shows `notice` in the viewer until the user dismisses it, opening the
    /// window on it: an attachment that did not come back is only visible here.
    pub(super) fn report_warning(&mut self, notice: String) {
        log::warn!("{notice}");
        self.notices.push(notice);
        self.open = true;
    }

    /// Shows the log that just finished loading, opening the window on it.
    pub(super) fn open_on_newly_loaded_log(&mut self, logs: &LoadedLogs) {
        self.selected = logs.len().saturating_sub(1);
        self.open = true;
    }

    /// Which of the loaded logs the window is showing.
    #[cfg(test)]
    pub(super) fn selected_log_index(&self) -> usize {
        self.selected
    }

    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        logs: &mut LoadedLogs,
        LogViewerContext {
            recordings,
            recording_names,
            map_center_request,
            log_hover,
            requests,
            write_access,
            dialog_open,
        }: LogViewerContext<'_>,
    ) {
        // The scans run on worker threads: whatever finished since the last
        // frame becomes what this one draws.
        logs.apply_finished_queries();
        // The ring on the map lives exactly as long as the cursor is on a
        // row: the rows below fill this in again while they draw.
        log_hover.row_position = None;
        if !self.open {
            return;
        }
        self.selected = self.selected.min(logs.len().saturating_sub(1));

        let mut open = self.open;
        let mut unload_selected = false;
        Window::new("Log viewer")
            .open(&mut open)
            .default_width(680.0)
            .default_height(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.notices_ui(ui);
                if logs.is_empty() {
                    ui.label(RichText::new(LOG_LOAD_HINT).weak());
                    return;
                }
                unload_selected = self.selector_row_ui(ui, logs);
                if self.summary_expanded
                    && let Some(log) = logs.get(self.selected)
                {
                    self.summary_panel_ui(ui, log);
                }
                self.filters_ui(ui, logs);
                ui.separator();
                // The footer claims its height first: the table then fills
                // what remains of the window.
                egui::Panel::bottom("log_viewer_footer")
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        self.footer_ui(
                            ui,
                            logs,
                            recordings,
                            recording_names,
                            requests,
                            write_access,
                        );
                    });
                if let Some((log_id, log)) = logs.get_with_id(self.selected) {
                    self.line_table_ui(
                        ui,
                        log,
                        log_id,
                        &mut LineTableRequests {
                            map_center: map_center_request,
                            hover: log_hover,
                        },
                    );
                }
            });

        if !dialog_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }
        self.open = open;

        if unload_selected {
            let unloaded = logs.remove(self.selected);
            if let Some(log) = unloaded {
                log::info!("Unloaded log {:?}", log.name());
            }
            self.selected = self.selected.min(logs.len().saturating_sub(1));
        }
    }

    /// The warnings this session's attachments produced, each dismissed on its
    /// own.
    fn notices_ui(&mut self, ui: &mut egui::Ui) {
        let mut dismissed = None;
        for (index, notice) in self.notices.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(notice).color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                );
                if ui
                    .small_button(ICON_X)
                    .on_hover_text(NOTICE_DISMISS_HOVER)
                    .clicked()
                {
                    dismissed = Some(index);
                }
            });
        }
        if let Some(index) = dismissed {
            self.notices.remove(index);
        }
    }

    /// The log being shown, the actions on it, and the parse summary that
    /// unfolds the summary panel. Returns whether the log was asked to unload.
    fn selector_row_ui(&mut self, ui: &mut egui::Ui, logs: &mut LoadedLogs) -> bool {
        let selected = self.selected;
        let summary = logs
            .get(selected)
            .map(LoadedLog::parse_summary_line)
            .unwrap_or_default();
        let selected_name = logs
            .get(selected)
            .map_or(EM_DASH, LoadedLog::name)
            .to_owned();
        let visible = logs.get(selected).is_some_and(LoadedLog::is_visible);

        let mut chosen = selected;
        let mut summary_expanded = self.summary_expanded;
        let mut unload = false;
        Sides::new().show(
            ui,
            |ui| {
                if logs.len() > 1 {
                    ComboBox::from_id_salt("log_viewer_selected_log")
                        .selected_text(&selected_name)
                        .show_ui(ui, |ui| {
                            for (index, log) in logs.iter().enumerate() {
                                ui.selectable_value(&mut chosen, index, log.name());
                            }
                        });
                } else {
                    ui.strong(&selected_name);
                }
                let eye = if visible { ICON_EYE } else { ICON_EYE_SLASH };
                if ui
                    .selectable_label(visible, eye)
                    .on_hover_text("Draw this log's matches on the map")
                    .clicked()
                    && let Some(log) = logs.get_mut(selected)
                {
                    log.set_visible(!visible);
                }
                if let Some(attachment) = logs.get(selected).and_then(LoadedLog::attachment) {
                    let (recording, _) =
                        gt_loaded_files::display_identity(&attachment.recording.identity);
                    ui.label(ICON_PAPERCLIP)
                        .on_hover_text(format!("Stored with the recording {recording}"));
                }
                unload = ui
                    .small_button(ICON_X)
                    .on_hover_text("Unload this log")
                    .clicked();
            },
            |ui| {
                if ui
                    .add(Label::new(RichText::new(summary).weak()).sense(egui::Sense::click()))
                    .on_hover_text(SUMMARY_HOVER)
                    .clicked()
                {
                    summary_expanded = !summary_expanded;
                }
            },
        );
        self.selected = chosen;
        self.summary_expanded = summary_expanded;
        unload
    }

    /// The association target the log takes its positions from, and how far an
    /// entry may be from a fix to take one.
    fn footer_ui(
        &mut self,
        ui: &mut egui::Ui,
        logs: &mut LoadedLogs,
        recordings: LoadedFilesView<'_>,
        recording_names: &RecordingNames,
        requests: &mut LogViewerRequests,
        write_access: WriteAccess,
    ) {
        let selected = self.selected;
        let Some((log_id, log)) = logs.get_with_id(selected) else {
            return;
        };
        let target = log.association_target();
        let candidates = log.rank_association_candidates(&recordings);
        let entered_unit = self.association_window_unit;
        let mut value = entered_unit.measure(log.association_window());
        let mut unit = entered_unit;
        let mut chosen_target = target;
        let mut window_edited = false;

        let attached = log.attachment().is_some();
        let names = recording_names_by_id(recordings, recording_names);

        ui.horizontal_wrapped(|ui| {
            ui.label("Associated with");
            let ranked = candidates.ranked();
            ui.add_enabled_ui(!ranked.is_empty(), |ui| {
                ComboBox::from_id_salt("log_viewer_association_target")
                    .selected_text(
                        target
                            .and_then(|id| names.get(&id).copied())
                            .unwrap_or(EM_DASH),
                    )
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(target.is_none(), EM_DASH)
                            .on_hover_text("Take this log's positions from no recording")
                            .clicked()
                        {
                            chosen_target = None;
                        }
                        for candidate in ranked {
                            let name = names.get(&candidate.recording).copied().unwrap_or(EM_DASH);
                            let overlapping = candidate.overlaps_the_log();
                            let label = if overlapping {
                                RichText::new(name)
                            } else {
                                RichText::new(name).weak()
                            };
                            let hover = if overlapping {
                                format!(
                                    "Ran alongside {} of the log, {} of its span",
                                    gt_fmt::format_human_terse_duration(candidate.overlap),
                                    gt_fmt::format_fraction_percent(candidate.fraction_of_log)
                                )
                            } else {
                                NO_OVERLAP_HOVER.to_owned()
                            };
                            if ui
                                .selectable_label(target == Some(candidate.recording), label)
                                .on_hover_text(hover)
                                .clicked()
                            {
                                chosen_target = Some(candidate.recording);
                            }
                        }
                    })
                    .response
                    .on_disabled_hover_text("Load a recording to associate this log against");
            });

            ui.separator();
            ui.label("Association window");
            window_edited = ui
                .add(DragValue::new(&mut value).range(0.0..=entered_unit.largest_value()))
                .on_hover_text(ASSOCIATION_WINDOW_HOVER)
                .changed();
            ComboBox::from_id_salt("log_viewer_association_window_unit")
                .selected_text(unit.label())
                .width(UNIT_DROPDOWN_WIDTH_PX)
                .show_ui(ui, |ui| {
                    for choice in AssociationWindowUnit::iter() {
                        ui.selectable_value(&mut unit, choice, choice.label());
                    }
                });

            ui.separator();
            let writes_recordings = write_access.allows_writing();
            let attach = ui.add_enabled(
                !recordings.is_empty() && writes_recordings,
                Button::new(ATTACH_LABEL),
            );
            if attach.clicked() {
                requests.open_association_dialog = Some(log_id);
            }
            attach
                .on_hover_text(ATTACH_HOVER)
                .on_disabled_hover_text(if writes_recordings {
                    ATTACH_NO_RECORDING_HOVER
                } else {
                    READ_ONLY_RECORDING_HISTORY_HOVER
                });
            let detach = ui.add_enabled(attached && writes_recordings, Button::new(DETACH_LABEL));
            if detach.clicked() {
                requests.detach = Some(log_id);
            }
            detach
                .on_hover_text(DETACH_HOVER)
                .on_disabled_hover_text(if writes_recordings {
                    DETACH_UNATTACHED_HOVER
                } else {
                    READ_ONLY_RECORDING_HISTORY_HOVER
                });
        });

        self.association_window_unit = unit;
        let Some(log) = logs.get_mut(selected) else {
            return;
        };
        if window_edited {
            log.set_association_window(entered_unit.window_of(value), &recordings);
        }
        if chosen_target != target {
            log.associate_with(chosen_target, &recordings);
        }
    }
}

/// The name each loaded recording goes by, under the identity a log names its
/// association target with.
fn recording_names_by_id<'a>(
    recordings: LoadedFilesView<'a>,
    recording_names: &'a RecordingNames,
) -> FxHashMap<LoadedFileId, &'a str> {
    recordings
        .entries()
        .enumerate()
        .map(|(index, entry)| {
            (
                entry.id(),
                recording_names.get(FileIdx::new(index)).unwrap_or(EM_DASH),
            )
        })
        .collect()
}
