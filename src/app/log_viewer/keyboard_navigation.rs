use egui::{Role, Sense, StrokeKind, WidgetInfo};
use gt_log_view::LoadedLogs;
use gt_ui_types::LoadedLogId;

use super::LogViewerWindow;
use super::log_list::LogTab;

pub(super) struct TableFocusScope(egui::Id);

impl TableFocusScope {
    pub(super) fn for_log(log: LoadedLogId) -> Self {
        Self(egui::Id::unique((TABLE_FOCUS_ID, log)))
    }

    pub(super) fn register_ui(&self, ui: &egui::Ui) -> egui::Response {
        let response = ui.interact(
            ui.available_rect_before_wrap(),
            self.0,
            Sense::focusable_noninteractive(),
        );
        response.widget_info(|| WidgetInfo::labeled(Role::Unknown, true, TABLE_LABEL));
        ui.ctx().accesskit_node_builder(self.0, |node| {
            node.set_role(Role::Table);
            node.set_label(TABLE_LABEL);
            node.set_author_id(TABLE_FOCUS_ID);
            node.set_description(TABLE_KEYBOARD_HELP);
        });
        response
    }

    pub(super) fn has_focus(&self, ui: &egui::Ui) -> bool {
        ui.input(|input| input.focused) && ui.memory(|memory| memory.has_focus(self.0))
    }

    pub(super) fn draw_focus(&self, ui: &egui::Ui, rect: egui::Rect) {
        if self.has_focus(ui) {
            ui.painter()
                .rect_stroke(rect, 0, ui.visuals().selection.stroke, StrokeKind::Inside);
        }
    }
}

impl LogViewerWindow {
    pub(in crate::app) fn return_table_focus(&mut self, log: LoadedLogId, logs: &LoadedLogs) {
        if self.open && self.selected == Some(log) && logs.get_by_id(log).is_some() {
            self.log_state_mut(log).table_focus_requested = true;
        }
    }

    pub(super) fn keyboard_shortcuts_ui(&mut self, ui: &egui::Ui, tabs: &[LogTab]) {
        if !self.keyboard_scope.owns_keyboard(ui.ctx(), ui.layer_id())
            || !self.keyboard_access.available(ui.ctx())
        {
            return;
        }
        for step in LogTabStep::consume(ui.ctx()) {
            if let Some(id) = step.target(tabs, self.selected) {
                self.open_on_log(id);
                self.log_state_mut(id).table_focus_requested = true;
            }
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
            && let Some(id) = self.selected
        {
            self.set_find_open(id, true);
        }
    }
}

enum LogTabStep {
    ActivateNext,
    ActivatePrevious,
}

impl LogTabStep {
    fn consume(ctx: &egui::Context) -> Vec<Self> {
        ctx.input_mut(|input| {
            let mut steps = Vec::new();
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && modifiers.ctrl
                    && !modifiers.alt
                    && !modifiers.mac_cmd
                {
                    steps.push(if modifiers.shift {
                        Self::ActivatePrevious
                    } else {
                        Self::ActivateNext
                    });
                    false
                } else {
                    true
                }
            });
            steps
        })
    }

    fn target(self, tabs: &[LogTab], selected: Option<LoadedLogId>) -> Option<LoadedLogId> {
        let index = tabs.iter().position(|tab| Some(tab.id) == selected)?;
        let next = match self {
            Self::ActivatePrevious => index
                .checked_sub(1)
                .unwrap_or_else(|| tabs.len().saturating_sub(1)),
            Self::ActivateNext => (index + 1) % tabs.len(),
        };
        tabs.get(next).map(|tab| tab.id)
    }
}

pub(super) const TABLE_FOCUS_ID: &str = "log_viewer_table_focus";
pub(super) const TABLE_LABEL: &str = "Log table";
const TABLE_KEYBOARD_HELP: &str = "Use Up and Down for rows, Page Up and Page Down for pages, and Home and End for the first and last rows";
