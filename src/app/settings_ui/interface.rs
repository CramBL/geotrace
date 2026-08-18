//! Presentation settings: the names GeoTrace derives from a recording, the
//! theme, and the map's tile layer with the Mapbox token it needs.

use egui::Grid;
use egui_phosphor::regular::KEY as ICON_KEY;
use egui_phosphor::regular::MAP_TRIFOLD as ICON_MAP_TRIFOLD;
use egui_phosphor::regular::PAINT_BRUSH as ICON_PAINT_BRUSH;
use gt_map::SatelliteLayerAccess;

use crate::app::settings_ui::SettingsPage;
use crate::app::{self, App, mapbox_token, recording_name_template};

impl App {
    pub(super) fn show_interface_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Interface.show_header(ui);
        Grid::new("interface_grid")
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

                ui.label(format!("{ICON_PAINT_BRUSH} Theme"));
                egui::widgets::global_theme_preference_buttons(ui);
                ui.end_row();

                ui.label(format!("{ICON_MAP_TRIFOLD} Map layer"));
                ui.horizontal(|ui| {
                    self.map
                        .show_layer_selector(ui, SatelliteLayerAccess::TokenRequired);
                });
                ui.end_row();

                ui.label(format!("{ICON_KEY} {}", mapbox_token::TOKEN_LABEL))
                    .on_hover_text("Mapbox serves the satellite layer's tiles");
                ui.horizontal(|ui| {
                    self.mapbox_token_field.show(
                        ui,
                        &mut self.map,
                        mapbox_token::MapboxTokenCommit::OnEnterOrFocusLoss,
                    );
                    self.mapbox_token_test.show_test_button_and_result(
                        ui,
                        self.mapbox_token_field.text(),
                        app::transport_source(self.offline),
                    );
                });
                ui.end_row();
            });
    }
}
