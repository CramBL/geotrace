//! The log viewer window: the loaded logs, the parse summary each of them
//! expands into, and the virtualized table of the selected log's lines.

mod association_window;
pub(super) mod filters;
mod line_table;
mod summary_panel;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use egui::{ComboBox, DragValue, Label, RichText, Sides, Window};
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::X as ICON_X;
use gt_loaded_files::{LoadedFileId, LoadedFilesView, RecordingNames};
use gt_log_view::{LoadedLog, LoadedLogs};
use gt_types::FileIdx;
use gt_ui_theme::EM_DASH;
use strum::IntoEnumIterator as _;

use association_window::AssociationWindowUnit;

/// What the window says while no log is loaded.
const EMPTY_STATE_HINT: &str = "Open a log file or drop it here";

/// How the viewer writes a moment, in the table and in the summary panel alike.
const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

const SUMMARY_HOVER: &str = "Show what the parse read from this log";

const ASSOCIATION_WINDOW_HOVER: &str = "Furthest a line's timestamp may be from a fix of the association target for \
     the line to take its position";

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
}

/// The app state the viewer reads and writes while it renders.
pub(super) struct LogViewerContext<'a> {
    pub recordings: LoadedFilesView<'a>,
    pub recording_names: &'a RecordingNames,
    pub map_center_request: &'a mut Option<(f64, f64)>,
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
        }
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
        }: LogViewerContext<'_>,
    ) {
        // The scans run on worker threads: whatever finished since the last
        // frame becomes what this one draws.
        logs.apply_finished_queries();
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
                if logs.is_empty() {
                    ui.label(RichText::new(EMPTY_STATE_HINT).weak());
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
                        self.footer_ui(ui, logs, recordings, recording_names);
                    });
                if let Some(log) = logs.get(self.selected) {
                    self.line_table_ui(ui, log, map_center_request);
                }
            });

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
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
    ) {
        let selected = self.selected;
        let Some(log) = logs.get(selected) else {
            return;
        };
        let target = log.association_target();
        let candidates = log.rank_association_candidates(&recordings);
        let entered_unit = self.association_window_unit;
        let mut value = entered_unit.measure(log.association_window());
        let mut unit = entered_unit;
        let mut chosen_target = target;
        let mut window_edited = false;

        let names: HashMap<LoadedFileId, &str> = recordings
            .entries()
            .enumerate()
            .map(|(index, entry)| {
                (
                    entry.id(),
                    recording_names.get(FileIdx::new(index)).unwrap_or(EM_DASH),
                )
            })
            .collect();

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
                                "This recording ran at no time the log covers: every line would \
                                 stay unassociated"
                                    .to_owned()
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
