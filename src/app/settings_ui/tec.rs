//! Settings for the global ionosphere map downloads.

use crate::app::backfill_ui::BackfillAction;
use crate::app::day_fetch_status::{self, FetchRowHoverText};
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::source_page::{self, SourcePageSlots};
use crate::app::{App, tec_mirrors_ui};

const MIRRORS_HOVER: &str = "Hosts serving the global ionosphere maps, tried in order until one \
                             has the day's file. The default is JPL, which publishes them. Add a \
                             mirror or an offline copy serving the same directory layout to fetch \
                             from there instead.";

const FETCH_ROW_HOVER: FetchRowHoverText = FetchRowHoverText {
    queue: gt_ionex::text::FETCH_QUEUE_HOVER,
    coverage: gt_ionex::text::RECORDING_DAY_COVERAGE_HOVER,
};

impl App {
    pub(super) fn show_tec_page(&mut self, ui: &mut egui::Ui) {
        let mut mirrors_changed = false;
        let fetch_status = self.tec_maps.fetch_queue().fetch_status();
        let progress = self.tec_maps.fetch_queue().backfill_progress();
        let readiness = self.backfill_readiness(self.tec_maps.archive_available());
        let mut backfill_action = None;

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
                status: |ui: &mut egui::Ui| {
                    day_fetch_status::show_fetch_rows(ui, fetch_status, FETCH_ROW_HOVER);
                },
                failures: self.tec_maps.fetch_queue().failures(),
                backfill: |ui: &mut egui::Ui| {
                    backfill_action = self.tec_map_backfill_ui.ui(ui, progress, readiness);
                },
            },
        );

        if mirrors_changed {
            self.tec_maps.set_mirrors(&self.tec_settings.mirrors);
        }
        if let Some(action) = backfill_action {
            match action {
                BackfillAction::Start { from, to } => {
                    let queued = self.tec_maps.backfill(from, to);
                    self.tec_map_backfill_ui.report_started(queued);
                }
                BackfillAction::Cancel => self.tec_maps.fetch_queue_mut().cancel_backfill(),
            }
        }
    }
}
