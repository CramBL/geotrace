//! Presentation settings for names GeoTrace derives from a recording.

use egui::Grid;
use egui_phosphor::regular::TEXT_AA as ICON_TEXT_AA;

use crate::app::{App, recording_name_template};

impl App {
    pub(super) fn show_display_page(&self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(ICON_TEXT_AA);
            ui.strong("Display");
        });
        ui.separator();
        Grid::new("display_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let preview = self.name_template_preview_recording();
                let mut template = self.shared.borrow().recording_name_template.clone();
                if recording_name_template::recording_name_template_ui(
                    ui,
                    &mut template,
                    preview.as_ref(),
                ) {
                    self.shared.borrow_mut().recording_name_template = template;
                }
                ui.end_row();
            });
    }
}
