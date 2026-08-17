//! Settings for the daily aircraft interference datasets.

use crate::app::App;
use crate::app::backfill_ui::BackfillAction;

impl App {
    pub(super) fn show_interference_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui_phosphor::regular::AIRPLANE_TILT);
            ui.strong("Aircraft interference");
        });
        ui.separator();
        egui::Grid::new("interference_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let url_help = "Base URL of the host serving the daily interference \
                                datasets. The default is gpsjam.org; point it at a \
                                mirror or an offline copy to fetch from there instead. \
                                Requests carry a date and nothing about your recordings.";
                ui.label(format!("{} Base URL", egui_phosphor::regular::GLOBE_SIMPLE))
                    .on_hover_text(url_help);
                let mut base_url = self.interference_settings.base_url.clone();
                if ui
                    .text_edit_singleline(&mut base_url)
                    .on_hover_text(url_help)
                    .changed()
                {
                    self.jamming.set_base_url(&base_url);
                    self.interference_settings.base_url = base_url;
                }
                ui.end_row();
            });
        ui.add_space(8.0);
        let readiness = self.backfill_readiness(self.jamming.archive_available());
        if let Some(action) =
            self.interference_backfill_ui
                .ui(ui, self.jamming.backfill_progress(), readiness)
        {
            match action {
                BackfillAction::Start { from, to } => {
                    let queued = self.jamming.backfill(from, to);
                    self.interference_backfill_ui.report_started(queued);
                }
                BackfillAction::Cancel => self.jamming.cancel_backfill(),
            }
        }
    }
}
