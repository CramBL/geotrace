//! Days a fetch worker could not archive, and the list the settings dialog
//! shows them in.
//!
//! Shared by every day-keyed fetch worker: what a failed day is and how it
//! reads is the same whichever host it came from.

use chrono::NaiveDate;
use egui::{Label, RichText, Ui};
use egui_phosphor::regular::WARNING as ICON_WARNING;

/// The failure list stops after this many entries, newest first: a host that
/// refuses every request cannot fill the dialog.
const MAX_LISTED_FAILURES: usize = 5;

/// A day that could not be added to an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayFailure {
    pub day: NaiveDate,
    pub detail: String,
}

impl std::fmt::Display for DayFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} - {}", self.day, self.detail)
    }
}

/// The days that could not be archived, newest first. `list_id` separates one
/// section's list from another's.
pub fn show_failures(ui: &mut Ui, list_id: &str, failures: &[DayFailure]) {
    if failures.is_empty() {
        return;
    }
    let amber = gt_ui_theme::warning_amber(ui.visuals().dark_mode);
    ui.label(
        RichText::new(format!(
            "{ICON_WARNING} {} {} could not be downloaded",
            failures.len(),
            gt_fmt::pluralize(failures.len(), "day", "days")
        ))
        .color(amber),
    );
    ui.indent(list_id, |ui| {
        for failure in failures.iter().rev().take(MAX_LISTED_FAILURES) {
            let line = failure.to_string();
            ui.add(Label::new(RichText::new(&line).weak()).truncate())
                .on_hover_text(&line);
        }
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable as _;
    use gt_test_utils::TestHarness;

    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn failure(day_of_july: u32) -> DayFailure {
        DayFailure {
            day: day(2026, 7, day_of_july),
            detail: "Kp: HTTP 500 Internal Server Error".to_owned(),
        }
    }

    #[test]
    fn a_failed_day_is_listed_with_its_cause() {
        let failures = [failure(21)];
        let mut harness = TestHarness::builder().ui(|ui| show_failures(ui, "failures", &failures));
        harness.run();
        assert!(
            harness
                .inner
                .query_by_label_contains("1 day could not")
                .is_some()
        );
        assert!(
            harness
                .inner
                .query_by_label_contains("2026-07-21 - Kp: HTTP 500 Internal Server Error")
                .is_some()
        );
    }

    /// A host refusing everything states the count and lists the newest few.
    #[test]
    fn a_long_failure_list_is_capped_under_its_count() {
        let failures: Vec<DayFailure> = (1..=20).map(failure).collect();
        let mut harness = TestHarness::builder().ui(|ui| show_failures(ui, "failures", &failures));
        harness.run();
        assert!(
            harness
                .inner
                .query_by_label_contains("20 days could not")
                .is_some()
        );
        assert!(
            harness
                .inner
                .query_by_label_contains("2026-07-20 -")
                .is_some(),
            "the newest failure is listed"
        );
        assert!(
            harness
                .inner
                .query_by_label_contains("2026-07-15 -")
                .is_none(),
            "the sixth-newest failure is past the cap"
        );
    }

    #[test]
    fn nothing_is_drawn_without_a_failure() {
        let mut harness = TestHarness::builder().ui(|ui| show_failures(ui, "failures", &[]));
        harness.run();
        assert!(harness.inner.query_by_label_contains("could not").is_none());
    }
}
