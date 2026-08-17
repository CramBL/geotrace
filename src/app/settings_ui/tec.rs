//! Settings for the global ionosphere map downloads.

use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::source_page::{self, NO_SLOT, SourcePageSlots};
use crate::app::{App, tec_mirrors_ui};

const MIRRORS_HOVER: &str = "Hosts serving the global ionosphere maps, tried in order until one \
                             has the day's file. The default is JPL, which publishes them. Add a \
                             mirror or an offline copy serving the same directory layout to fetch \
                             from there instead.";

impl App {
    pub(super) fn show_tec_page(&mut self, ui: &mut egui::Ui) {
        let mut mirrors_changed = false;

        source_page::show_source_page(
            ui,
            SettingsPage::IonosphericTec,
            SourcePageSlots {
                endpoint: |ui: &mut egui::Ui| {
                    ui.label(format!("{} Mirrors", egui_phosphor::regular::GLOBE_SIMPLE))
                        .on_hover_text(MIRRORS_HOVER);
                    mirrors_changed =
                        tec_mirrors_ui::show_mirror_list(ui, &mut self.tec_settings.mirrors);
                    ui.end_row();
                },
                status: NO_SLOT,
                failures: self.tec_maps.failures(),
                backfill: NO_SLOT,
            },
        );

        if mirrors_changed {
            self.tec_maps.set_mirrors(&self.tec_settings.mirrors);
        }
    }
}
