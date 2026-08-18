//! Settings for the solar flare downloads.

use egui::{RichText, Ui};

use crate::app::App;
use crate::app::backfill_ui::{self, BackfillAction};
use crate::app::day_fetch_status::{self, FetchRowHoverText};
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::source_page::{self, SourcePageSlots};

pub(super) const API_KEY_LABEL: &str = "API key";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    source_page::BASE_URL_LABEL,
    API_KEY_LABEL,
    day_fetch_status::FETCH_QUEUE_LABEL,
    day_fetch_status::RECORDING_DAYS_LABEL,
    backfill_ui::DOWNLOAD_HISTORY_LABEL,
];

const URL_HOVER: &str = "Base URL of the host serving the solar flare catalog. The default is \
                         api.nasa.gov, which serves DONKI. Point it at a proxy or an offline copy \
                         to fetch from there instead. Requests carry a date range and nothing \
                         about your recordings.";

const KEY_HOVER: &str = "Your own api.nasa.gov key, which every request to the catalog carries. \
                         It is stored in the settings file as entered, and never written to a log \
                         or a download failure.";

const FETCH_ROW_HOVER: FetchRowHoverText = FetchRowHoverText {
    queue: "Catalog days waiting to be downloaded. One day is requested at a time, and one day \
            costs one request.",
    coverage: "UTC days the recordings loaded this session span, and how many of them the archive \
               holds the catalog's flares for. Days downloaded by a backfill are not counted here.",
};

impl App {
    pub(super) fn show_solar_flare_page(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.solar_flare_settings.clone();
        let mut base_url_changed = false;
        let mut api_key_changed = false;
        let fetch_status = self.solar_flares.fetch_queue().fetch_status();
        let progress = self.solar_flares.fetch_queue().backfill_progress();
        let readiness = self.solar_flare_backfill_readiness();
        let mut backfill_action = None;

        source_page::show_source_page(
            ui,
            SettingsPage::SolarFlares,
            SourcePageSlots {
                endpoint: |ui: &mut Ui| {
                    base_url_changed =
                        source_page::show_base_url_row(ui, URL_HOVER, &mut settings.base_url);
                    api_key_changed = show_api_key_row(ui, &mut settings.api_key);
                },
                status: |ui: &mut Ui| {
                    day_fetch_status::show_fetch_rows(ui, fetch_status, FETCH_ROW_HOVER);
                },
                failures: self.solar_flares.fetch_queue().failures(),
                backfill: |ui: &mut Ui| {
                    backfill_action = self.solar_flare_backfill_ui.ui(ui, progress, readiness);
                },
            },
        );

        if api_key_changed {
            self.solar_flares.set_api_key(settings.api_key());
        }
        if base_url_changed {
            self.solar_flares.set_base_url(&settings.base_url);
        }
        if base_url_changed || api_key_changed {
            self.solar_flare_settings = settings;
        }
        if let Some(action) = backfill_action {
            match action {
                BackfillAction::Start { from, to } => {
                    let queued = self.solar_flares.backfill(from, to);
                    self.solar_flare_backfill_ui.report_started(queued);
                }
                BackfillAction::Cancel => {
                    self.solar_flares.fetch_queue_mut().cancel_backfill();
                }
            }
        }
    }
}

/// The key row, with what an empty field means beneath it. Returns `true` in
/// the frame the text changed.
fn show_api_key_row(ui: &mut Ui, api_key: &mut String) -> bool {
    ui.label(format!("{} {API_KEY_LABEL}", egui_phosphor::regular::KEY))
        .on_hover_text(KEY_HOVER);
    let changed = ui
        .vertical(|ui| {
            let changed = ui
                .text_edit_singleline(api_key)
                .on_hover_text(KEY_HOVER)
                .changed();
            if api_key.trim().is_empty() {
                ui.label(RichText::new(gt_flare::text::MISSING_KEY.as_str()).weak());
            }
            changed
        })
        .inner;
    ui.end_row();
    changed
}
