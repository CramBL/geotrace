//! Settings for the global ionosphere map downloads.

use egui::{RichText, Ui};
use gt_store::EnvironmentArchive;

use crate::app::backfill_ui::{self, BackfillAction};
use crate::app::day_fetch_status::{self, FetchRowHoverText};
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::source_page::{self, ReferenceLink, SourcePageSlots};
use crate::app::tec_mirrors_ui::EarthdataToken;
use crate::app::{App, tec_mirrors_ui};

const MIRRORS_LABEL: &str = "Mirrors";

const REFERENCE_LINK_LABEL: &str = gt_ionex::reference::IONOSPHERIC_TEC.link_question;

const REFERENCE_LINK_HOVER: &str = "Reference material on the ionosphere, total electron content, \
                                    and the delay it adds to satellite navigation signals";

pub(super) const EARTHDATA_TOKEN_LABEL: &str = "Earthdata token";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    MIRRORS_LABEL,
    EARTHDATA_TOKEN_LABEL,
    day_fetch_status::FETCH_QUEUE_LABEL,
    day_fetch_status::RECORDING_DAYS_LABEL,
    day_fetch_status::BACKGROUND_DAYS_LABEL,
    backfill_ui::DOWNLOAD_HISTORY_LABEL,
    REFERENCE_LINK_LABEL,
];

const MIRRORS_HOVER: &str = "Hosts serving the global ionosphere maps, tried in order until one \
                             has the day's file. The default is JPL, which publishes them, \
                             followed by the CDDIS archive. Add a mirror or an offline copy \
                             serving either layout to fetch from there instead.";

const TOKEN_HOVER: &str = "Your own NASA Earthdata token, sent to the CDDIS mirrors and to no \
                           other host. It is stored in the settings file as entered, and never \
                           written to a log or a download failure.";

const FETCH_ROW_HOVER: FetchRowHoverText = FetchRowHoverText {
    queue: gt_ionex::text::FETCH_QUEUE_HOVER,
    coverage: gt_ionex::text::RECORDING_DAY_COVERAGE_HOVER,
};

impl App {
    pub(super) fn show_tec_page(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.tec_settings.clone();
        let earthdata_token = match settings.earthdata_token() {
            Some(_) => EarthdataToken::Set,
            None => EarthdataToken::Missing,
        };
        let mut mirrors_changed = false;
        let mut token_changed = false;
        let fetch_status = self.tec_maps.fetch_queue().fetch_status();
        let background_days = self.tec_maps.background_day_coverage();
        let progress = self.tec_maps.fetch_queue().backfill_progress();
        let readiness = self.backfill_readiness(EnvironmentArchive::IonosphericTec);
        let mut backfill_action = None;

        let opened_reference = source_page::show_source_page(
            ui,
            SettingsPage::IonosphericTec,
            SourcePageSlots {
                endpoint: |ui: &mut Ui| {
                    ui.label(format!(
                        "{} {MIRRORS_LABEL}",
                        egui_phosphor::regular::GLOBE_SIMPLE
                    ))
                    .on_hover_text(MIRRORS_HOVER);
                    mirrors_changed = tec_mirrors_ui::show_mirror_list(
                        ui,
                        &mut settings.mirrors,
                        earthdata_token,
                    );
                    ui.end_row();
                    token_changed = show_earthdata_token_row(ui, &mut settings.earthdata_token);
                },
                status: |ui: &mut Ui| {
                    day_fetch_status::show_fetch_rows(ui, fetch_status, FETCH_ROW_HOVER);
                    day_fetch_status::show_background_day_row(
                        ui,
                        background_days,
                        gt_ionex::text::BACKGROUND_DAY_COVERAGE_HOVER.as_str(),
                    );
                },
                failures: self.tec_maps.fetch_queue().failures(),
                backfill: |ui: &mut Ui| {
                    backfill_action = self.tec_map_backfill_ui.ui(ui, progress, readiness);
                },
                reference: Some(ReferenceLink {
                    document: gt_ionex::reference::IONOSPHERIC_TEC,
                    hover_text: REFERENCE_LINK_HOVER,
                }),
            },
        );

        if let Some(document) = opened_reference {
            self.reference_window.open(document);
        }
        if mirrors_changed {
            self.tec_maps.set_mirrors(&settings.mirrors);
        }
        if token_changed {
            self.tec_maps
                .set_earthdata_token(settings.earthdata_token());
        }
        if mirrors_changed || token_changed {
            self.tec_settings = settings;
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

/// The token row, with what an empty field leaves unfetched beneath it.
/// Returns `true` in the frame the text changed.
fn show_earthdata_token_row(ui: &mut Ui, earthdata_token: &mut String) -> bool {
    ui.label(format!(
        "{} {EARTHDATA_TOKEN_LABEL}",
        egui_phosphor::regular::KEY
    ))
    .on_hover_text(TOKEN_HOVER);
    let changed = ui
        .vertical(|ui| {
            let changed = ui
                .text_edit_singleline(earthdata_token)
                .on_hover_text(TOKEN_HOVER)
                .changed();
            if earthdata_token.trim().is_empty() {
                ui.label(RichText::new(gt_ionex::text::MISSING_EARTHDATA_TOKEN.as_str()).weak());
            }
            changed
        })
        .inner;
    ui.end_row();
    changed
}
