//! Settings for the daily aircraft interference datasets.

use gt_store::EnvironmentArchive;

use crate::app::App;
use crate::app::backfill_ui::{self, BackfillAction};
use crate::app::day_fetch_status::{self, FetchRowHoverText};
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::source_page::{self, ReferenceLink, SourcePageSlots};

const REFERENCE_LINK_LABEL: &str = gt_jam::reference::AIRCRAFT_INTERFERENCE.link_question;

const REFERENCE_LINK_HOVER: &str = "Reference material on what aircraft report, how the daily \
                                    cells are computed, and what the data does and does not show";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    source_page::BASE_URL_LABEL,
    day_fetch_status::FETCH_QUEUE_LABEL,
    day_fetch_status::RECORDING_DAYS_LABEL,
    backfill_ui::DOWNLOAD_HISTORY_LABEL,
    REFERENCE_LINK_LABEL,
];

const URL_HOVER: &str = "Base URL of the host serving the daily interference datasets. The \
                         default is gpsjam.org. Point it at a mirror or an offline copy to fetch \
                         from there instead. Requests carry a date and nothing about your \
                         recordings.";

const FETCH_ROW_HOVER: FetchRowHoverText = FetchRowHoverText {
    queue: gt_jam::text::FETCH_QUEUE_HOVER,
    coverage: gt_jam::text::RECORDING_DAY_COVERAGE_HOVER,
};

impl App {
    pub(super) fn show_interference_page(&mut self, ui: &mut egui::Ui) {
        let mut base_url = self.interference_settings.base_url.clone();
        let mut base_url_changed = false;
        let fetch_status = self.jamming.fetch_queue().fetch_status();
        let progress = self.jamming.fetch_queue().backfill_progress();
        let readiness = self.backfill_readiness(EnvironmentArchive::AircraftInterference);
        let mut backfill_action = None;

        let opened_reference = source_page::show_source_page(
            ui,
            SettingsPage::AircraftInterference,
            SourcePageSlots {
                endpoint: |ui: &mut egui::Ui| {
                    base_url_changed = source_page::show_base_url_row(ui, URL_HOVER, &mut base_url)
                },
                status: |ui: &mut egui::Ui| {
                    day_fetch_status::show_fetch_rows(ui, fetch_status, FETCH_ROW_HOVER);
                },
                failures: self.jamming.fetch_queue().failures(),
                backfill: |ui: &mut egui::Ui| {
                    backfill_action = self.interference_backfill_ui.ui(ui, progress, readiness);
                },
                reference: Some(ReferenceLink {
                    document: gt_jam::reference::AIRCRAFT_INTERFERENCE,
                    hover_text: REFERENCE_LINK_HOVER,
                }),
            },
        );

        if let Some(document) = opened_reference {
            self.reference_window.open(document);
        }
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
                BackfillAction::Cancel => self.jamming.fetch_queue_mut().cancel_backfill(),
            }
        }
    }
}
