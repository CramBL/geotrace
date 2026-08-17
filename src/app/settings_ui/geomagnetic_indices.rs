//! Settings for the Kp and Hp30 geomagnetic index downloads.

use crate::app::backfill_ui::BackfillAction;
use crate::app::{App, day_failures, geomagnetic_index_ui};

impl App {
    pub(super) fn show_geomagnetic_index_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui_phosphor::regular::MAGNET);
            ui.strong("Geomagnetic indices");
        });
        ui.separator();
        egui::Grid::new("geomagnetic_index_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let url_help = "Base URL of the host serving the Kp and Hp30 \
                                geomagnetic indices. The default is GFZ Potsdam, which \
                                publishes them. Point it at a mirror or an offline copy \
                                to fetch from there instead. Requests carry a date range \
                                and nothing about your recordings.";
                ui.label(format!("{} Base URL", egui_phosphor::regular::GLOBE_SIMPLE))
                    .on_hover_text(url_help);
                let mut base_url = self.geomagnetic_index_settings.base_url.clone();
                if ui
                    .text_edit_singleline(&mut base_url)
                    .on_hover_text(url_help)
                    .changed()
                {
                    self.geomagnetic_indices.set_base_url(&base_url);
                    self.geomagnetic_index_settings.base_url = base_url;
                }
                ui.end_row();

                geomagnetic_index_ui::show_fetch_rows(ui, self.geomagnetic_indices.fetch_status());
            });
        day_failures::show_failures(
            ui,
            "geomagnetic_index_failures",
            self.geomagnetic_indices.failures(),
        );
        ui.add_space(8.0);
        let readiness = self.backfill_readiness(self.geomagnetic_indices.archive_available());
        if let Some(action) = self.geomagnetic_index_backfill_ui.ui(
            ui,
            self.geomagnetic_indices.backfill_progress(),
            readiness,
        ) {
            match action {
                BackfillAction::Start { from, to } => {
                    let queued = self.geomagnetic_indices.backfill(from, to);
                    self.geomagnetic_index_backfill_ui.report_started(queued);
                }
                BackfillAction::Cancel => self.geomagnetic_indices.cancel_backfill(),
            }
        }
    }
}
