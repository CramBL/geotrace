//! The layout shared by the settings pages of the day-keyed data sources:
//! endpoint, fetch status, failed days, download history.

use egui::Ui;

use crate::app::day_failures::{self, DayFailure};
use crate::app::settings_ui::SettingsPage;

pub(super) const BASE_URL_LABEL: &str = "Base URL";

/// What one data source page puts in each part of the shared layout. The slots
/// are closures because each one reaches a differently typed scheduler field on
/// [`crate::app::App`].
pub(super) struct SourcePageSlots<'a, Endpoint, Status, Backfill> {
    pub(super) endpoint: Endpoint,
    pub(super) status: Status,
    pub(super) failures: &'a [DayFailure],
    pub(super) backfill: Backfill,
}

pub(super) fn show_source_page<Endpoint, Status, Backfill>(
    ui: &mut Ui,
    page: SettingsPage,
    slots: SourcePageSlots<'_, Endpoint, Status, Backfill>,
) where
    Endpoint: FnOnce(&mut Ui),
    Status: FnOnce(&mut Ui),
    Backfill: FnOnce(&mut Ui),
{
    page.show_header(ui);
    ui.push_id(page, |ui| {
        egui::Grid::new("endpoint_and_status")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                (slots.endpoint)(ui);
                (slots.status)(ui);
            });
        day_failures::show_failures(ui, "failures", slots.failures);
        ui.add_space(8.0);
        (slots.backfill)(ui);
    });
}

/// The endpoint row of a source that fetches from a single host. Returns `true`
/// in the frame the text changed.
pub(super) fn show_base_url_row(ui: &mut Ui, hover_text: &str, base_url: &mut String) -> bool {
    ui.label(format!(
        "{} {BASE_URL_LABEL}",
        egui_phosphor::regular::GLOBE_SIMPLE
    ))
    .on_hover_text(hover_text);
    let changed = ui
        .text_edit_singleline(base_url)
        .on_hover_text(hover_text)
        .changed();
    ui.end_row();
    changed
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use egui_kittest::kittest::Queryable as _;
    use gt_test_utils::TestHarness;

    use super::*;

    #[test]
    fn the_slots_stack_endpoint_status_failures_backfill() {
        let failures = [DayFailure {
            day: NaiveDate::from_ymd_opt(2026, 7, 21).unwrap_or_default(),
            detail: "Kp: HTTP 500 Internal Server Error".to_owned(),
        }];
        let mut harness = TestHarness::builder().ui(|ui| {
            show_source_page(
                ui,
                SettingsPage::GeomagneticIndices,
                SourcePageSlots {
                    endpoint: |ui: &mut Ui| {
                        ui.label("endpoint slot");
                        ui.end_row();
                    },
                    status: |ui: &mut Ui| {
                        ui.label("status slot");
                        ui.end_row();
                    },
                    failures: &failures,
                    backfill: |ui: &mut Ui| {
                        ui.label("backfill slot");
                    },
                },
            );
        });
        harness.run();

        let top = |label: &str| harness.inner.get_by_label_contains(label).rect().top();
        assert!(top("endpoint slot") < top("status slot"));
        assert!(top("status slot") < top("2026-07-21 - Kp: HTTP 500"));
        assert!(top("2026-07-21 - Kp: HTTP 500") < top("backfill slot"));
    }
}
