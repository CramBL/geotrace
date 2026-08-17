//! Settings for the global ionosphere map downloads.

use crate::app::settings_ui::SettingsPage;
use crate::app::{App, day_failures, tec_mirrors_ui};

impl App {
    pub(super) fn show_tec_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::IonosphericTec.show_header(ui);
        egui::Grid::new("tec_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let mirrors_help = "Hosts serving the global ionosphere maps, tried in \
                                    order until one has the day's file. The default is \
                                    JPL, which publishes them. Add a mirror or an offline \
                                    copy serving the same directory layout to fetch from \
                                    there instead.";
                ui.label(format!("{} Mirrors", egui_phosphor::regular::GLOBE_SIMPLE))
                    .on_hover_text(mirrors_help);
                if tec_mirrors_ui::show_mirror_list(ui, &mut self.tec_settings.mirrors) {
                    self.tec_maps.set_mirrors(&self.tec_settings.mirrors);
                }
                ui.end_row();
            });
        day_failures::show_failures(ui, "tec_failures", self.tec_maps.failures());
    }
}
