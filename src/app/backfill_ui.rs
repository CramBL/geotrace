//! The "download interference history" control in the settings dialog.
//!
//! Emits a [`BackfillAction`]. The scheduler is not driven directly, so the
//! control can be tested without an archive or a host.

use chrono::{Datelike as _, Days, NaiveDate};
use egui::{Button, ProgressBar, RichText, Ui};
use egui_extras::DatePickerButton;
use egui_phosphor::regular::CLOUD_ARROW_DOWN as ICON_DOWNLOAD;
use egui_phosphor::regular::X as ICON_CANCEL;
use jiff::civil::Date;

use gt_jam::calendar::{self, COVERAGE_START};

use super::format::format_size;
use super::jamming::BackfillProgress;

/// How much one archived day costs on disk, measured over the captured
/// fixtures, for the size estimate.
const BYTES_PER_DAY: u64 = 81 * 1024;

/// Above this, the estimate reads in minutes.
const MINUTES_CUTOFF_SECS: u64 = 90;

/// Range presets, as days back from today. [`None`] is the whole coverage
/// window.
const PRESETS: [(&str, Option<u64>); 3] = [
    ("30 days", Some(30)),
    ("1 year", Some(365)),
    ("Everything", None),
];

/// What the control is asking the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillAction {
    Start { from: NaiveDate, to: NaiveDate },
    Cancel,
}

/// Session state of the control: the two ends of the range.
///
/// Held as [`jiff::civil::Date`], which is what [`DatePickerButton`] edits.
/// The rest of the app is on [`chrono`], so the two convert here.
pub struct BackfillUi {
    from: Date,
    to: Date,
    /// The outcome of the last start, shown until the next one.
    outcome: Option<String>,
}

impl Default for BackfillUi {
    fn default() -> Self {
        Self::with_today(calendar::today_utc())
    }
}

/// [`chrono`] to [`jiff`].
fn to_jiff(date: NaiveDate) -> Date {
    i16::try_from(date.year())
        .ok()
        .zip(i8::try_from(date.month()).ok())
        .zip(i8::try_from(date.day()).ok())
        .and_then(|((year, month), day)| Date::new(year, month, day).ok())
        .unwrap_or_default()
}

/// [`jiff`] to [`chrono`].
fn to_chrono(date: Date) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(
        i32::from(date.year()),
        u32::try_from(date.month()).ok()?,
        u32::try_from(date.day()).ok()?,
    )
}

/// A rough wall-clock and disk estimate for `days`, since a full backfill
/// runs for the better part of an hour.
fn estimate(days: usize) -> String {
    let days = days as u64;
    let seconds = days * gt_jam::transport::REQUEST_INTERVAL.as_secs();
    let duration = if seconds < MINUTES_CUTOFF_SECS {
        format!("{seconds} s")
    } else {
        format!("{} min", seconds.div_ceil(60))
    };
    let plural = if days == 1 { "day" } else { "days" };
    format!(
        "{days} {plural}, about {duration} and {}",
        format_size(days * BYTES_PER_DAY)
    )
}

/// The start of a preset range: `days_back` before `today`, or the start of
/// coverage when the preset is the whole window.
fn preset_start(today: NaiveDate, days_back: Option<u64>) -> NaiveDate {
    days_back
        .and_then(|back| today.checked_sub_days(Days::new(back)))
        .unwrap_or(COVERAGE_START)
        .max(COVERAGE_START)
}

impl BackfillUi {
    /// Seeded with the thirty days ending `today`.
    ///
    /// `today` is a parameter so a snapshot of the settings window does not
    /// change every day.
    pub fn with_today(today: NaiveDate) -> Self {
        Self {
            from: to_jiff(preset_start(today, Some(30))),
            to: to_jiff(today),
            outcome: None,
        }
    }

    /// The selected range, or [`None`] when it runs backwards. The pickers
    /// only produce real dates, so that is the one unusable state.
    fn range(&self) -> Option<(NaiveDate, NaiveDate)> {
        let (from, to) = (to_chrono(self.from)?, to_chrono(self.to)?);
        (from <= to).then_some((from, to))
    }

    /// Record what a start produced, so the user learns that a range they
    /// already hold cost nothing.
    ///
    /// [`None`] is [`super::jamming::JammingScheduler::backfill`] reporting
    /// no archive.
    pub fn report_started(&mut self, queued: Option<usize>) {
        self.outcome = Some(match queued {
            None => "No interference archive to download into".to_owned(),
            Some(0) => "Every day in that range is already archived".to_owned(),
            Some(queued) => format!("Downloading {}", estimate(queued)),
        });
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        progress: Option<BackfillProgress>,
        archive_available: bool,
    ) -> Option<BackfillAction> {
        let mut action = None;
        let running = progress.is_some();
        let today = calendar::today_utc();
        // The host serves nothing outside this window, so the calendar does
        // not offer it.
        let years = i16::try_from(COVERAGE_START.year()).unwrap_or(i16::MIN)
            ..=i16::try_from(today.year()).unwrap_or(i16::MAX);

        ui.horizontal(|ui| {
            ui.label("Range")
                .on_hover_text("UTC days, both ends included");
            for (date, salt) in [
                (&mut self.from, "backfill_from"),
                (&mut self.to, "backfill_to"),
            ] {
                ui.add_enabled(
                    !running,
                    DatePickerButton::new(date)
                        .id_salt(salt)
                        .start_end_years(years.clone())
                        .highlight_weekends(false),
                )
                .on_disabled_hover_text("Cancel the running download to change the range");
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Preset").weak());
            for (label, days_back) in PRESETS {
                if ui
                    .add_enabled(!running, Button::new(label).small())
                    .clicked()
                {
                    self.from = to_jiff(preset_start(today, days_back));
                    self.to = to_jiff(today);
                }
            }
        });

        if let Some(progress) = progress {
            ui.horizontal(|ui| {
                ui.add(
                    ProgressBar::new(progress.fraction())
                        .desired_width(160.0)
                        .text(format!("{} / {}", progress.done, progress.total)),
                );
                if ui
                    .button(format!("{ICON_CANCEL} Cancel"))
                    .on_hover_text("Stop after the day being downloaded finishes")
                    .clicked()
                {
                    action = Some(BackfillAction::Cancel);
                }
            });
            return action;
        }

        ui.horizontal(|ui| {
            let range = self.range().filter(|_| archive_available);
            let button = ui.add_enabled(
                range.is_some(),
                Button::new(format!("{ICON_DOWNLOAD} Download history")),
            );
            let button = match range {
                Some((from, to)) => button.on_hover_text(format!(
                    "Download the daily interference datasets for this range: at most {}. \
                     Days already archived are skipped.",
                    estimate(calendar::fetchable_days(from, to, today).len())
                )),
                None => button,
            };
            if button
                .on_disabled_hover_text(if archive_available {
                    "Pick an end date on or after the start date"
                } else {
                    "The interference archive could not be opened, so there is nowhere \
                     to download to"
                })
                .clicked()
                && let Some((from, to)) = range
            {
                action = Some(BackfillAction::Start { from, to });
            }
            if let Some(outcome) = self.outcome.as_ref() {
                ui.label(RichText::new(outcome).weak());
            }
        });

        action
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use rstest::rstest;

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    /// Pinned at both ends of the coverage window and across a leap day.
    #[rstest]
    #[case::coverage_start(COVERAGE_START)]
    #[case::a_leap_day(date(2024, 2, 29))]
    #[case::new_years_eve(date(2026, 12, 31))]
    #[case::the_first_of_a_month(date(2026, 8, 1))]
    fn dates_survive_the_round_trip_through_jiff(#[case] original: NaiveDate) {
        assert_eq!(to_chrono(to_jiff(original)), Some(original));
    }

    #[rstest]
    #[case::ascending(date(2026, 7, 20), date(2026, 7, 26), true)]
    #[case::one_day(date(2026, 7, 20), date(2026, 7, 20), true)]
    #[case::reversed(date(2026, 7, 26), date(2026, 7, 20), false)]
    fn range_accepts_only_an_ascending_range(
        #[case] from: NaiveDate,
        #[case] to: NaiveDate,
        #[case] usable: bool,
    ) {
        let state = BackfillUi {
            from: to_jiff(from),
            to: to_jiff(to),
            outcome: None,
        };
        assert_eq!(state.range(), usable.then_some((from, to)));
    }

    #[test]
    fn the_default_range_is_the_last_thirty_days() {
        let state = BackfillUi::default();
        let (from, to) = state.range().expect("the default range is usable");
        assert_eq!(to, calendar::today_utc());
        assert_eq!((to - from).num_days(), 30);
    }

    /// A preset never reaches back past the first published day.
    #[test]
    fn presets_stay_inside_the_coverage_window() {
        let today = calendar::today_utc();
        for (_, days_back) in PRESETS {
            assert!(preset_start(today, days_back) >= COVERAGE_START);
        }
        assert_eq!(preset_start(today, None), COVERAGE_START);
    }

    #[test]
    fn estimates_scale_from_seconds_to_minutes() {
        insta::assert_snapshot!(
            "backfill_estimates",
            [1, 30, 365, 1600]
                .map(|days| format!("{days:>4} -> {}", estimate(days)))
                .join("\n")
        );
    }

    /// Nothing is requested until the button is pressed.
    #[test]
    fn rendering_the_control_starts_nothing() {
        let mut state = BackfillUi::default();
        let actions = RefCell::new(Vec::new());
        let mut harness = Harness::new_ui(|ui| {
            actions.borrow_mut().extend(state.ui(ui, None, true));
        });
        harness.run();
        assert!(actions.borrow().is_empty());
    }

    #[test]
    fn pressing_download_starts_the_selected_range() {
        let mut state = BackfillUi::default();
        let expected = state.range().expect("the default range is usable");
        let action = RefCell::new(None);
        let mut harness = Harness::new_ui(|ui| {
            if let Some(emitted) = state.ui(ui, None, true) {
                *action.borrow_mut() = Some(emitted);
            }
        });
        harness.get_by_label_contains("Download history").click();
        harness.run();
        assert_eq!(
            *action.borrow(),
            Some(BackfillAction::Start {
                from: expected.0,
                to: expected.1
            })
        );
    }

    #[test]
    fn a_backwards_range_disables_the_button() {
        let mut state = BackfillUi {
            from: to_jiff(date(2026, 7, 26)),
            to: to_jiff(date(2026, 7, 20)),
            outcome: None,
        };
        let mut harness = Harness::new_ui(|ui| {
            state.ui(ui, None, true);
        });
        harness.run();
        assert!(
            harness
                .get_by_label_contains("Download history")
                .accesskit_node()
                .is_disabled()
        );
    }

    /// Never hidden, per DESIGN.md: without an archive the button is there
    /// and says why it does nothing.
    #[test]
    fn without_an_archive_the_button_is_disabled() {
        let mut state = BackfillUi::default();
        let mut harness = Harness::new_ui(|ui| {
            state.ui(ui, None, false);
        });
        harness.run();
        assert!(
            harness
                .get_by_label_contains("Download history")
                .accesskit_node()
                .is_disabled()
        );
    }

    /// While running, the range is locked and the only action is cancelling.
    #[test]
    fn a_running_backfill_offers_cancel_instead_of_download() {
        let mut state = BackfillUi::default();
        let action = RefCell::new(None);
        let progress = BackfillProgress { done: 3, total: 10 };
        let mut harness = Harness::new_ui(|ui| {
            if let Some(emitted) = state.ui(ui, Some(progress), true) {
                *action.borrow_mut() = Some(emitted);
            }
        });
        harness.run();
        assert!(
            harness
                .query_by_label_contains("Download history")
                .is_none()
        );
        harness.get_by_label_contains("Cancel").click();
        harness.run();
        assert_eq!(*action.borrow(), Some(BackfillAction::Cancel));
    }

    #[rstest]
    #[case::no_archive(None)]
    #[case::nothing_to_do(Some(0))]
    #[case::a_month(Some(30))]
    fn the_outcome_line_reports_what_was_queued(#[case] queued: Option<usize>) {
        let mut state = BackfillUi::default();
        state.report_started(queued);
        insta::assert_snapshot!(
            format!(
                "backfill_outcome_{}",
                queued.map_or_else(|| "none".to_owned(), |days| days.to_string())
            ),
            state.outcome.expect("an outcome was recorded")
        );
    }
}
