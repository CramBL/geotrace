use egui::accesskit::Role;
use egui::{EventFilter, FocusDirection, Popup, Sense, StrokeKind, WidgetInfo, WidgetType};
use gt_ui_types::LoadedLogId;

pub(super) struct TableFocusScope(egui::Id);

impl TableFocusScope {
    pub(super) fn keyboard_available(ui: &egui::Ui) -> bool {
        !Popup::is_any_open(ui.ctx())
            && ui.ctx().top_layer_id() == Some(ui.layer_id())
            && ui.memory(|memory| memory.top_modal_layer().is_none())
    }

    pub(super) fn for_log(log: LoadedLogId) -> Self {
        Self(egui::Id::new((TABLE_FOCUS_ID, log)))
    }

    pub(super) fn register_ui(&self, ui: &egui::Ui) -> egui::Response {
        let response = ui.interact(
            ui.available_rect_before_wrap(),
            self.0,
            Sense::focusable_noninteractive(),
        );
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, TABLE_LABEL));
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

    pub(super) fn lock_vertical_arrows(&self, ui: &egui::Ui) {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                self.0,
                EventFilter {
                    vertical_arrows: true,
                    ..EventFilter::default()
                },
            );
        });
    }

    pub(super) fn draw_focus(&self, ui: &egui::Ui, rect: egui::Rect) {
        if self.has_focus(ui) {
            ui.painter()
                .rect_stroke(rect, 0, ui.visuals().selection.stroke, StrokeKind::Inside);
        }
    }
}

pub(super) struct KeyboardScrollSteps {
    pub(super) row_height_px: f32,
    pub(super) page_height_px: f32,
    pub(super) content_height_px: f32,
}

impl KeyboardScrollSteps {
    pub(super) fn scroll_the_table(&self, ui: &egui::Ui) {
        let distance_px: f32 = [
            (egui::Key::ArrowDown, self.row_height_px),
            (egui::Key::ArrowUp, -self.row_height_px),
            (egui::Key::PageDown, self.page_height_px),
            (egui::Key::PageUp, -self.page_height_px),
            (egui::Key::Home, -self.content_height_px),
            (egui::Key::End, self.content_height_px),
        ]
        .into_iter()
        .map(|(key, step_px)| {
            let presses =
                ui.input_mut(|input| input.count_and_consume_key(egui::Modifiers::NONE, key));
            if presses > 0 {
                ui.memory_mut(|memory| memory.move_focus(FocusDirection::None));
            }
            presses as f32 * step_px
        })
        .sum();
        ui.scroll_with_delta(egui::vec2(0.0, -distance_px));
    }
}

pub(super) const TABLE_FOCUS_ID: &str = "log_viewer_table_focus";
pub(super) const TABLE_LABEL: &str = "Log table";
const TABLE_KEYBOARD_HELP: &str = "Use Up and Down for rows, Page Up and Page Down for pages, and Home and End for the first and last rows";
