//! Settings for the daily aircraft interference datasets.

use crate::app::App;
use crate::app::backfill_ui::BackfillAction;
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::source_page::{self, NO_SLOT, SourcePageSlots};

const URL_HOVER: &str = "Base URL of the host serving the daily interference datasets. The \
                         default is gpsjam.org. Point it at a mirror or an offline copy to fetch \
                         from there instead. Requests carry a date and nothing about your \
                         recordings.";

impl App {
    pub(super) fn show_interference_page(&mut self, ui: &mut egui::Ui) {
        let mut base_url = self.interference_settings.base_url.clone();
        let mut base_url_changed = false;
        let progress = self.jamming.backfill_progress();
        let readiness = self.backfill_readiness(self.jamming.archive_available());
        let mut backfill_action = None;

        source_page::show_source_page(
            ui,
            SettingsPage::AircraftInterference,
            SourcePageSlots {
                endpoint: |ui: &mut egui::Ui| {
                    base_url_changed = source_page::show_base_url_row(ui, URL_HOVER, &mut base_url)
                },
                status: NO_SLOT,
                failures: &[],
                backfill: Some(|ui: &mut egui::Ui| {
                    backfill_action = self.interference_backfill_ui.ui(ui, progress, readiness);
                }),
            },
        );

        if base_url_changed {
            self.jamming.set_base_url(&base_url);
            self.interference_settings.base_url = base_url;
        }
        if let Some(action) = backfill_action {
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
