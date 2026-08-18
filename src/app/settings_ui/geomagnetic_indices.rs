//! Settings for the Kp and Hp30 geomagnetic index downloads.

use crate::app::App;
use crate::app::backfill_ui::{self, BackfillAction};
use crate::app::day_fetch_status::{self, FetchRowHoverText};
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::source_page::{self, SourcePageSlots};

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    source_page::BASE_URL_LABEL,
    day_fetch_status::FETCH_QUEUE_LABEL,
    day_fetch_status::RECORDING_DAYS_LABEL,
    backfill_ui::DOWNLOAD_HISTORY_LABEL,
];

const URL_HOVER: &str = "Base URL of the host serving the Kp and Hp30 geomagnetic indices. The \
                         default is GFZ Potsdam, which publishes them. Point it at a mirror or an \
                         offline copy to fetch from there instead. Requests carry a date range \
                         and nothing about your recordings.";

const FETCH_ROW_HOVER: FetchRowHoverText = FetchRowHoverText {
    queue: "Index days waiting to be downloaded. One day is requested at a time, and one day \
            costs one request per index.",
    coverage: "UTC days the recordings loaded this session span, and how many of them the archive \
               holds every published index for. Days downloaded by a backfill are not counted \
               here.",
};

impl App {
    pub(super) fn show_geomagnetic_index_page(&mut self, ui: &mut egui::Ui) {
        let mut base_url = self.geomagnetic_index_settings.base_url.clone();
        let mut base_url_changed = false;
        let fetch_status = self.geomagnetic_indices.fetch_queue().fetch_status();
        let progress = self.geomagnetic_indices.fetch_queue().backfill_progress();
        let readiness = self.backfill_readiness(self.geomagnetic_indices.archive_available());
        let mut backfill_action = None;

        source_page::show_source_page(
            ui,
            SettingsPage::GeomagneticIndices,
            SourcePageSlots {
                endpoint: |ui: &mut egui::Ui| {
                    base_url_changed = source_page::show_base_url_row(ui, URL_HOVER, &mut base_url)
                },
                status: |ui: &mut egui::Ui| {
                    day_fetch_status::show_fetch_rows(ui, fetch_status, FETCH_ROW_HOVER);
                },
                failures: self.geomagnetic_indices.fetch_queue().failures(),
                backfill: |ui: &mut egui::Ui| {
                    backfill_action = self
                        .geomagnetic_index_backfill_ui
                        .ui(ui, progress, readiness);
                },
            },
        );

        if base_url_changed {
            self.geomagnetic_indices.set_base_url(&base_url);
            self.geomagnetic_index_settings.base_url = base_url;
        }
        if let Some(action) = backfill_action {
            match action {
                BackfillAction::Start { from, to } => {
                    let queued = self.geomagnetic_indices.backfill(from, to);
                    self.geomagnetic_index_backfill_ui.report_started(queued);
                }
                BackfillAction::Cancel => {
                    self.geomagnetic_indices.fetch_queue_mut().cancel_backfill();
                }
            }
        }
    }
}
