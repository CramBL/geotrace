use egui::{Area, Button, Frame, Label, Order, Popup, RichText, Role, WidgetInfo};
use egui_phosphor::regular::ARROW_U_UP_LEFT as ICON_RETURN;
use gt_log_view::LoadedLog;
use gt_ui_types::LoadedLogId;

use super::LogViewerWindow;
use super::line_table::SourceRequest;

#[derive(Clone, Copy)]
pub(super) struct SourceNavigationFeedback {
    anchor: SourceFeedbackAnchor,
    started_at_secs: f64,
}

#[derive(Clone, Copy)]
enum SourceFeedbackAnchor {
    ShowAtControl { control: egui::Id, log: LoadedLogId },
    ShowAtMapPosition(egui::Pos2),
}

impl LogViewerWindow {
    pub(super) fn report_unavailable_map_source(&mut self, ctx: &egui::Context) {
        self.source_navigation_feedback = Some(SourceNavigationFeedback {
            anchor: SourceFeedbackAnchor::ShowAtMapPosition(
                ctx.pointer_interact_pos()
                    .unwrap_or_else(|| ctx.content_rect().center()),
            ),
            started_at_secs: ctx.input(|input| input.time),
        });
    }

    pub(super) fn source_feedback_ui(&mut self, ctx: &egui::Context) {
        let Some(feedback) = self.source_navigation_feedback else {
            return;
        };
        let remaining = SOURCE_FEEDBACK_DURATION_SECS
            - (ctx.input(|input| input.time) - feedback.started_at_secs);
        if remaining <= 0.0 {
            self.source_navigation_feedback = None;
            return;
        }
        ctx.request_repaint_after_secs(remaining as f32);
        if let SourceFeedbackAnchor::ShowAtMapPosition(position) = feedback.anchor {
            Area::new(egui::Id::unique("unavailable_log_source"))
                .order(Order::Tooltip)
                .fixed_pos(position + SOURCE_FEEDBACK_OFFSET_PX)
                .interactable(false)
                .show(ctx, |ui| {
                    Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(UNAVAILABLE_SOURCE_LABEL);
                    });
                });
        }
    }

    pub(super) fn source_link_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        id: LoadedLogId,
        request: SourceRequest,
        button: Button<'_>,
    ) {
        let hover = match request {
            SourceRequest::ShowBootSession(_) => "Show this boot session in the table",
            SourceRequest::NavigateToEntry(_)
            | SourceRequest::NavigateToLine(_)
            | SourceRequest::ShowSurroundingLines(_) => "Show this source line in the table",
        };
        let response = ui.add(button).on_hover_text(hover);
        if response.clicked() {
            if self.navigate_to_source(log, id, request) {
                self.source_navigation_feedback = None;
            } else {
                self.source_navigation_feedback = Some(SourceNavigationFeedback {
                    anchor: SourceFeedbackAnchor::ShowAtControl {
                        control: response.id,
                        log: id,
                    },
                    started_at_secs: ui.input(|input| input.time),
                });
                ui.ctx()
                    .request_repaint_after_secs(SOURCE_FEEDBACK_DURATION_SECS as f32);
            }
        }
        if self.source_navigation_feedback.is_some_and(|feedback| matches!(feedback.anchor, SourceFeedbackAnchor::ShowAtControl { control, log } if control == response.id && log == id)) {
            response.show_tooltip_text(UNAVAILABLE_SOURCE_LABEL);
        }
    }

    pub(super) fn source_navigation_status_ui(
        &mut self,
        ui: &mut egui::Ui,
        log: &LoadedLog,
        id: LoadedLogId,
    ) {
        let Some(reason) = self
            .log_state_mut(id)
            .source_reveal
            .and_then(|reveal| reveal.reason(log, self.show_structural_lines))
        else {
            return;
        };
        let reveal = self.log_state_mut(id).source_reveal;
        let additional = self.table_rows(log, id).additional_source_count();
        ui.push_id(("log_source_navigation", id), |ui| {
            ui.horizontal(|ui| {
                let chip = ui
                    .add(Button::new(format!("{additional} revealed")).small())
                    .on_hover_text(&reason);
                chip.ctx
                    .accesskit_node_builder(chip.id, |node| node.set_description(reason.as_str()));
                Popup::menu(&chip).show(|ui| {
                    ui.add(Label::new(&reason).selectable(true));
                    if let Some(reveal) = reveal {
                        let enabled = reveal.can_extend(log.parsed());
                        if ui
                            .add_enabled(enabled, Button::new("Show surrounding lines"))
                            .on_hover_text(
                                "Add nearby physical source lines within the context limit",
                            )
                            .on_disabled_hover_text(
                                "The complete source range or context limit is shown",
                            )
                            .clicked()
                        {
                            self.navigate_to_source(log, id, reveal.context_request());
                            ui.close();
                        }
                    }
                });
                let back = ui
                    .add(Button::new(RichText::new(ICON_RETURN)).small())
                    .on_hover_text(RETURN_LABEL);
                back.widget_info(|| WidgetInfo::labeled(Role::Button, true, RETURN_LABEL));
                if back.clicked() {
                    self.return_to_filtered_results(id);
                    self.source_navigation_feedback = None;
                }
            });
        });
    }
}

#[cfg(test)]
pub(super) const REVEAL_LABEL: &str = "1 revealed";
pub(super) const RETURN_LABEL: &str = "Return to filtered results";
pub(super) const UNAVAILABLE_SOURCE_LABEL: &str = "Source line unavailable";
const SOURCE_FEEDBACK_DURATION_SECS: f64 = 3.0;
const SOURCE_FEEDBACK_OFFSET_PX: egui::Vec2 = egui::vec2(12.0, 12.0);
