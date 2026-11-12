use egui::accesskit::Role;
use egui::{Button, CursorIcon, RichText, ScrollArea, Sense};
use egui_phosphor::regular::CALENDAR_DOTS as ICON_CALENDAR;
use gt_log_view::{LoadedLog, LoadedLogs};
use gt_logfile::YearResolution;
use gt_ui_types::LoadedLogId;

use super::line_table::LineTableRequests;
use super::{LogViewerRequests, LogViewerWindow, YearsToResolve, line_table};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum InspectorSection {
    ShowChronology,
    ShowSummary,
}

impl LogViewerWindow {
    pub(super) fn toggle_inspector(&mut self, id: LoadedLogId, section: InspectorSection) {
        let state = self.log_state_mut(id);
        state.inspector = (state.inspector != Some(section)).then_some(section);
        if state.scroll_to_row.is_none() && state.scroll_to_source_line.is_none() {
            state.scroll_to_source_line = state.table_reading_anchor;
        }
    }

    pub(super) fn date_status_ui(&mut self, ui: &mut egui::Ui, log: &LoadedLog, id: LoadedLogId) {
        let resolved = log.parsed().chronology().is_resolved();
        let expanded = self.log_state_mut(id).inspector == Some(InspectorSection::ShowChronology);
        let label = if !resolved {
            "Dates unresolved"
        } else if log.parsed().year_resolution() == YearResolution::Infer
            || log.parsed().format().writes_years()
        {
            "Dates"
        } else {
            "Dates set"
        };
        let mut text = RichText::new(format!("{ICON_CALENDAR} {label}"));
        if !resolved {
            text = text.color(gt_ui_theme::warning_amber(ui.visuals().dark_mode));
        }
        let response = ui
            .add(Button::new(text).selected(expanded))
            .on_hover_text("Inspect dates and year resolution");
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_label(label);
            node.set_description("Inspect dates and year resolution");
            node.set_author_id(DATE_STATUS_ID);
            node.set_expanded(expanded);
        });
        if response.clicked() {
            self.toggle_inspector(id, InspectorSection::ShowChronology);
        }
    }

    pub(super) fn reading_regions_ui(
        &mut self,
        ui: &mut egui::Ui,
        logs: &mut LoadedLogs,
        requests: &mut LogViewerRequests,
        pointer_over_the_window: bool,
        table_requests: &mut LineTableRequests<'_>,
    ) {
        let content_height = ui.available_height();
        let spacing = ui.spacing().item_spacing;
        let inspector = self
            .selected
            .and_then(|id| self.log_state_mut(id).inspector);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let controls_share = 1.0
                - MINIMUM_TABLE_HEIGHT_FRACTION
                - if inspector.is_some() {
                    MAXIMUM_INITIAL_INSPECTOR_HEIGHT_FRACTION
                } else {
                    0.0
                };
            let controls_height = content_height * controls_share;
            ScrollArea::both()
                .id_salt(("log_viewer_header", self.selected))
                .min_scrolled_height(0.0)
                .max_height(controls_height.max(0.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = spacing;
                    self.notices_ui(ui);
                    if logs.is_empty() {
                        ui.label(RichText::new(super::LOG_LOAD_HINT).weak());
                    }
                    if let Some(id) = self.selected {
                        self.filters_ui(ui, logs, id);
                        self.find_bar_ui(ui, logs, id);
                    }
                });
            let Some(id) = self.selected else {
                return;
            };
            let Some(log) = logs.get_by_id(id) else {
                return;
            };
            if let Some(section) = inspector {
                self.inspector_ui(
                    ui,
                    log,
                    id,
                    section,
                    &InspectorLayout {
                        total_content_height: content_height,
                        item_spacing: spacing,
                    },
                    requests,
                );
            }
            self.line_table_ui(ui, log, id, pointer_over_the_window, table_requests);
        });
    }

    fn inspector_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        id: LoadedLogId,
        section: InspectorSection,
        layout: &InspectorLayout,
        requests: &mut LogViewerRequests,
    ) {
        let available = ui.available_height();
        let maximum =
            (available - INSPECTOR_DIVIDER_HEIGHT_PX - line_table::row_height(ui)).max(0.0);
        let automatic = (layout.total_content_height * MAXIMUM_INITIAL_INSPECTOR_HEIGHT_FRACTION)
            .min(
                available
                    - layout.total_content_height * MINIMUM_TABLE_HEIGHT_FRACTION
                    - INSPECTOR_DIVIDER_HEIGHT_PX,
            )
            .max(0.0);
        let height = self
            .log_state_mut(id)
            .inspector_height
            .unwrap_or(automatic)
            .clamp(0.0, maximum);
        let top = ui.cursor().top();
        ScrollArea::both()
            .id_salt(("log_viewer_inspector", id, section))
            .min_scrolled_height(0.0)
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = layout.item_spacing;
                match section {
                    InspectorSection::ShowSummary => self.summary_panel_ui(ui, log, id),
                    InspectorSection::ShowChronology => {
                        self.chronology_inspector_ui(ui, log, id, requests);
                    }
                }
            });
        let (rect, divider) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), INSPECTOR_DIVIDER_HEIGHT_PX),
            Sense::click_and_drag(),
        );
        let divider = divider
            .on_hover_cursor(CursorIcon::ResizeVertical)
            .on_hover_text("Drag to resize log details");
        divider.ctx.accesskit_node_builder(divider.id, |node| {
            node.set_role(Role::Splitter);
            node.set_label("Resize log details");
        });
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            ui.visuals().widgets.noninteractive.bg_stroke,
        );
        if divider.dragged()
            && let Some(pointer) = divider.interact_pointer_pos()
        {
            let state = self.log_state_mut(id);
            state.inspector_height =
                Some((pointer.y - top - INSPECTOR_DIVIDER_HEIGHT_PX / 2.0).clamp(0.0, maximum));
            if state.scroll_to_row.is_none() && state.scroll_to_source_line.is_none() {
                state.scroll_to_source_line = state.table_reading_anchor;
            }
        }
    }

    fn chronology_inspector_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        id: LoadedLogId,
        requests: &mut LogViewerRequests,
    ) {
        if let Some(year_resolution) = self.chronology_editor_ui(ui, id, log) {
            requests.resolve_years = Some(YearsToResolve {
                log: id,
                year_resolution,
            });
        }
    }
}

struct InspectorLayout {
    total_content_height: f32,
    item_spacing: egui::Vec2,
}

pub(in crate::app) const DATE_STATUS_ID: &str = "log-date-status";

const INSPECTOR_DIVIDER_HEIGHT_PX: f32 = 6.0;

const MAXIMUM_INITIAL_INSPECTOR_HEIGHT_FRACTION: f32 = 1.0 / 3.0;

const MINIMUM_TABLE_HEIGHT_FRACTION: f32 = 0.5;
