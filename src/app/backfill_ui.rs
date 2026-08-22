//! The "download history" control in the settings dialog, shown under every
//! dataset that archives whole UTC days.
//!
//! Emits a [`BackfillAction`]: the control can be tested without an archive
//! or a host, because it never drives a scheduler itself.

use std::marker::PhantomData;
use std::time::Duration;

use chrono::{Datelike as _, Days, NaiveDate, Utc};
use egui::{Button, ProgressBar, RichText, Ui};
use egui_extras::DatePickerButton;
use egui_phosphor::regular::CLOUD_ARROW_DOWN as ICON_DOWNLOAD;
use egui_phosphor::regular::X as ICON_CANCEL;
use jiff::civil::Date;

use super::backfill::BackfillProgress;
use super::civil_date;

pub const DOWNLOAD_HISTORY_LABEL: &str = "Download history";

/// Estimates above this display in minutes.
const MINUTES_CUTOFF_SECS: u64 = 90;

/// Estimates above this display in hours.
const HOURS_CUTOFF_MINS: u64 = 90;

/// Days the range covers before the user picks one.
const DEFAULT_RANGE_DAYS: u64 = 30;

/// One range preset, offered as a button beside the pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillPreset {
    pub label: &'static str,
    /// Days back from today the range starts. [`None`] reaches back to the
    /// start of coverage.
    pub days_back: Option<u64>,
}

/// A dataset the control downloads history for: what its host publishes, and
/// what one day of it costs.
pub trait BackfillDataset {
    /// Distinguishes this control's date pickers from another control's.
    const ID_PREFIX: &'static str;
    /// Names the archive days are downloaded into, as the outcome line and
    /// the disabled hover spell it.
    const ARCHIVE_NAME: &'static str;
    /// Names what one day holds, as the download button's hover spells it.
    const DAY_SUBJECT: &'static str;
    /// How many requests one day costs.
    const REQUESTS_PER_DAY: u64;
    /// Gap the transport keeps between requests to the host.
    const REQUEST_INTERVAL: Duration;
    /// Bytes one archived day takes on disk, measured on a filled archive.
    const BYTES_PER_DAY: u64;
    /// The presets offered beside the pickers, widest last.
    const PRESETS: [BackfillPreset; 3];

    /// First UTC day the host publishes, the earliest the pickers offer.
    fn coverage_start() -> NaiveDate;

    /// The days in `from..=to` the host can serve.
    fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate>;

    /// A rough wall-clock and disk estimate for downloading `count` days.
    fn estimate(count: usize) -> String {
        let days = count as u64;
        let seconds = days * Self::REQUESTS_PER_DAY * Self::REQUEST_INTERVAL.as_secs();
        let minutes = seconds.div_ceil(60);
        let duration = if seconds < MINUTES_CUTOFF_SECS {
            format!("{seconds} s")
        } else if minutes < HOURS_CUTOFF_MINS {
            format!("{minutes} min")
        } else {
            #[expect(
                clippy::cast_precision_loss,
                reason = "an estimate in hours needs its leading digits only"
            )]
            let hours = seconds as f64 / 3600.0;
            format!("{hours:.1} h")
        };
        format!(
            "{days} {}, about {duration} and {}",
            gt_fmt::pluralize(count, "day", "days"),
            gt_fmt::format_bytes(days * Self::BYTES_PER_DAY)
        )
    }
}

/// The daily interference datasets from gpsjam.org.
pub struct InterferenceBackfill;

impl BackfillDataset for InterferenceBackfill {
    const ID_PREFIX: &'static str = "interference_backfill";
    const ARCHIVE_NAME: &'static str = "interference archive";
    const DAY_SUBJECT: &'static str = "daily interference datasets";
    const REQUESTS_PER_DAY: u64 = 1;
    const REQUEST_INTERVAL: Duration = gt_jam::transport::REQUEST_INTERVAL;
    const BYTES_PER_DAY: u64 = 81 * 1024;
    const PRESETS: [BackfillPreset; 3] = [
        BackfillPreset {
            label: "30 days",
            days_back: Some(30),
        },
        BackfillPreset {
            label: "1 year",
            days_back: Some(365),
        },
        BackfillPreset {
            label: "Everything",
            days_back: None,
        },
    ];

    fn coverage_start() -> NaiveDate {
        gt_jam::calendar::COVERAGE_START
    }

    fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate> {
        gt_jam::calendar::fetchable_days(from, to, today_utc)
    }
}

/// The Kp and Hp30 indices from GFZ Potsdam.
pub struct GeomagneticIndexBackfill;

impl BackfillDataset for GeomagneticIndexBackfill {
    const ID_PREFIX: &'static str = "geomagnetic_index_backfill";
    const ARCHIVE_NAME: &'static str = gt_solar::text::ARCHIVE_NAME;
    const DAY_SUBJECT: &'static str = gt_solar::text::INDEX_NAMES;
    /// One request per index.
    const REQUESTS_PER_DAY: u64 = 2;
    const REQUEST_INTERVAL: Duration = gt_solar::transport::REQUEST_INTERVAL;
    const BYTES_PER_DAY: u64 = 12 * 1024;
    /// Kp reaches back to 1932, which no preset offers: downloading it whole
    /// takes days.
    const PRESETS: [BackfillPreset; 3] = [
        BackfillPreset {
            label: "30 days",
            days_back: Some(30),
        },
        BackfillPreset {
            label: "1 year",
            days_back: Some(365),
        },
        BackfillPreset {
            label: "5 years",
            days_back: Some(5 * 365),
        },
    ];

    fn coverage_start() -> NaiveDate {
        gt_solar::calendar::COVERAGE_START
    }

    fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate> {
        gt_solar::calendar::fetchable_days(from, to, today_utc)
    }
}

/// The daily global ionosphere maps from JPL.
pub struct TecMapBackfill;

impl BackfillDataset for TecMapBackfill {
    const ID_PREFIX: &'static str = "tec_map_backfill";
    const ARCHIVE_NAME: &'static str = gt_ionex::text::ARCHIVE_NAME;
    const DAY_SUBJECT: &'static str = gt_ionex::text::MAP_NAMES;
    /// The first mirror's settled file serves a past day, and a further
    /// request only goes out where that mirror holds no file.
    const REQUESTS_PER_DAY: u64 = 1;
    const REQUEST_INTERVAL: Duration = gt_ionex::transport::REQUEST_INTERVAL;
    /// Measured on an archive filled from JPL final files: a day of 13 maps on
    /// the published 71 by 73 grid adds about 128,000 bytes.
    const BYTES_PER_DAY: u64 = 125 * 1024;
    /// No preset reaches the 2008 coverage start: downloading it whole costs
    /// hours and about 800 MB.
    const PRESETS: [BackfillPreset; 3] = [
        BackfillPreset {
            label: "30 days",
            days_back: Some(30),
        },
        BackfillPreset {
            label: "1 year",
            days_back: Some(365),
        },
        BackfillPreset {
            label: "5 years",
            days_back: Some(5 * 365),
        },
    ];

    fn coverage_start() -> NaiveDate {
        gt_ionex::calendar::COVERAGE_START
    }

    fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate> {
        gt_ionex::calendar::fetchable_days(from, to, today_utc)
    }
}

/// The solar flare events from the NASA DONKI catalog.
pub struct SolarFlareBackfill;

impl BackfillDataset for SolarFlareBackfill {
    const ID_PREFIX: &'static str = "solar_flare_backfill";
    const ARCHIVE_NAME: &'static str = gt_flare::text::ARCHIVE_NAME;
    const DAY_SUBJECT: &'static str = gt_flare::text::EVENT_NAMES;
    const REQUESTS_PER_DAY: u64 = 1;
    const REQUEST_INTERVAL: Duration = gt_flare::transport::REQUEST_INTERVAL;
    /// Measured on an archive filled from the May 2024 storm, the busiest
    /// flare days on record at about 20 events a day.
    const BYTES_PER_DAY: u64 = 3 * 1024;
    /// No preset reaches the 2010 coverage start: the catalog's whole span is
    /// about 6000 days.
    const PRESETS: [BackfillPreset; 3] = [
        BackfillPreset {
            label: "30 days",
            days_back: Some(30),
        },
        BackfillPreset {
            label: "1 year",
            days_back: Some(365),
        },
        BackfillPreset {
            label: "5 years",
            days_back: Some(5 * 365),
        },
    ];

    fn coverage_start() -> NaiveDate {
        gt_flare::calendar::COVERAGE_START
    }

    fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate> {
        gt_flare::calendar::fetchable_days(from, to, today_utc)
    }
}

/// Whether a download can start, and what stops it when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillReadiness {
    Ready,
    /// There is nowhere to download to yet: the archives are still opening.
    ArchiveStillOpening,
    /// There is nowhere to download to: the archive could not be opened.
    WithoutArchive,
    /// No request may leave the machine: GeoTrace runs offline.
    Offline,
    /// The host needs a key the user has not entered.
    WithoutApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillAction {
    Start { from: NaiveDate, to: NaiveDate },
    Cancel,
}

/// Session state of the control: the two ends of the range.
///
/// Held as [`jiff::civil::Date`], which is what [`DatePickerButton`] edits.
/// The rest of the app is on [`chrono`], so the two convert here.
pub struct BackfillUi<D: BackfillDataset> {
    from: Date,
    to: Date,
    /// The outcome of the last start, shown until the next one.
    outcome: Option<String>,
    dataset: PhantomData<D>,
}

impl<D: BackfillDataset> Default for BackfillUi<D> {
    fn default() -> Self {
        Self::with_today(Utc::now().date_naive())
    }
}

impl<D: BackfillDataset> BackfillUi<D> {
    /// Seeded with the last [`DEFAULT_RANGE_DAYS`] days, ending `today`.
    ///
    /// `today` is a parameter so a snapshot of the settings window does not
    /// change every day.
    pub fn with_today(today: NaiveDate) -> Self {
        Self {
            from: civil_date::to_jiff(Self::preset_start(today, Some(DEFAULT_RANGE_DAYS))),
            to: civil_date::to_jiff(today),
            outcome: None,
            dataset: PhantomData,
        }
    }

    /// The start of a preset range: `days_back` before `today`, or the start
    /// of coverage when the preset is the whole window.
    fn preset_start(today: NaiveDate, days_back: Option<u64>) -> NaiveDate {
        days_back
            .and_then(|back| today.checked_sub_days(Days::new(back)))
            .unwrap_or_else(D::coverage_start)
            .max(D::coverage_start())
    }

    /// The selected range, or [`None`] when it runs backwards. The pickers
    /// only produce real dates, so that is the one unusable state.
    fn range(&self) -> Option<(NaiveDate, NaiveDate)> {
        let (from, to) = (
            civil_date::to_chrono(self.from)?,
            civil_date::to_chrono(self.to)?,
        );
        (from <= to).then_some((from, to))
    }

    /// Sets the outcome line shown under the range controls.
    ///
    /// [`None`] is the scheduler reporting no archive.
    pub fn report_started(&mut self, queued: Option<usize>) {
        self.outcome = Some(match queued {
            None => format!("No {} to download into", D::ARCHIVE_NAME),
            Some(0) => "Every day in that range is already archived".to_owned(),
            Some(queued) => format!("Downloading {}", D::estimate(queued)),
        });
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        progress: Option<BackfillProgress>,
        readiness: BackfillReadiness,
    ) -> Option<BackfillAction> {
        let mut action = None;
        let running = progress.is_some();
        let today = Utc::now().date_naive();
        // The host serves nothing outside this window, so the calendar does
        // not offer it.
        let years = i16::try_from(D::coverage_start().year()).unwrap_or(i16::MIN)
            ..=i16::try_from(today.year()).unwrap_or(i16::MAX);

        ui.horizontal(|ui| {
            ui.label("Range")
                .on_hover_text("UTC days, both ends included");
            for (date, salt) in [
                (&mut self.from, format!("{}_from", D::ID_PREFIX)),
                (&mut self.to, format!("{}_to", D::ID_PREFIX)),
            ] {
                ui.add_enabled(
                    !running,
                    DatePickerButton::new(date)
                        .id_salt(&salt)
                        .start_end_years(years.clone())
                        .highlight_weekends(false),
                )
                .on_disabled_hover_text("Cancel the running download to change the range");
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Preset").weak());
            for preset in D::PRESETS {
                if ui
                    .add_enabled(!running, Button::new(preset.label).small())
                    .clicked()
                {
                    self.from = civil_date::to_jiff(Self::preset_start(today, preset.days_back));
                    self.to = civil_date::to_jiff(today);
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
            let range = self
                .range()
                .filter(|_| readiness == BackfillReadiness::Ready);
            let button = ui.add_enabled(
                range.is_some(),
                Button::new(format!("{ICON_DOWNLOAD} {DOWNLOAD_HISTORY_LABEL}")),
            );
            let button = match range {
                Some((from, to)) => button.on_hover_text(format!(
                    "Download the {} for this range: at most {}. Days already archived are \
                     skipped.",
                    D::DAY_SUBJECT,
                    D::estimate(D::fetchable_days(from, to, today).len())
                )),
                None => button,
            };
            if button
                .on_disabled_hover_text(Self::blocked_hover(readiness))
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

    /// Why the download button is grayed out. A readiness that allows a
    /// download leaves the range as the one thing that can be wrong.
    fn blocked_hover(readiness: BackfillReadiness) -> String {
        match readiness {
            BackfillReadiness::Ready => "Pick an end date on or after the start date".to_owned(),
            BackfillReadiness::ArchiveStillOpening => {
                format!("The {} is still opening", D::ARCHIVE_NAME)
            }
            BackfillReadiness::WithoutArchive => format!(
                "There is nowhere to download to: the {} could not be opened",
                D::ARCHIVE_NAME
            ),
            BackfillReadiness::Offline => "Downloading is disabled in offline mode".to_owned(),
            BackfillReadiness::WithoutApiKey => {
                format!("Enter an API key above to download {}", D::DAY_SUBJECT)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_test_utils::TestHarness;
    use rstest::rstest;

    use super::*;

    /// The interference control stands in for both: the dataset only supplies
    /// constants, and the geomagnetic ones are covered where they differ.
    type TestBackfillUi = BackfillUi<InterferenceBackfill>;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn state(from: NaiveDate, to: NaiveDate) -> TestBackfillUi {
        TestBackfillUi {
            from: civil_date::to_jiff(from),
            to: civil_date::to_jiff(to),
            outcome: None,
            dataset: PhantomData,
        }
    }

    /// Pinned at both ends of the coverage window and across a leap day.
    #[rstest]
    #[case::coverage_start(gt_jam::calendar::COVERAGE_START)]
    #[case::a_leap_day(date(2024, 2, 29))]
    #[case::new_years_eve(date(2026, 12, 31))]
    #[case::the_first_of_a_month(date(2026, 8, 1))]
    fn dates_survive_the_round_trip_through_jiff(#[case] original: NaiveDate) {
        assert_eq!(
            civil_date::to_chrono(civil_date::to_jiff(original)),
            Some(original)
        );
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
        assert_eq!(state(from, to).range(), usable.then_some((from, to)));
    }

    #[test]
    fn the_default_range_is_the_last_thirty_days() {
        let state = TestBackfillUi::default();
        let (from, to) = state.range().expect("the default range is usable");
        assert_eq!(to, Utc::now().date_naive());
        assert_eq!((to - from).num_days(), 30);
    }

    /// A preset never reaches back past the first published day.
    #[test]
    fn presets_stay_inside_the_coverage_window() {
        let today = Utc::now().date_naive();
        for preset in InterferenceBackfill::PRESETS {
            assert!(
                TestBackfillUi::preset_start(today, preset.days_back)
                    >= gt_jam::calendar::COVERAGE_START
            );
        }
        assert_eq!(
            TestBackfillUi::preset_start(today, None),
            gt_jam::calendar::COVERAGE_START
        );
    }

    /// The widest preset of a bounded dataset and where it starts today.
    fn widest_preset<D: BackfillDataset>() -> (Option<u64>, NaiveDate) {
        let days_back = D::PRESETS
            .into_iter()
            .filter_map(|preset| preset.days_back)
            .max();
        (
            days_back,
            BackfillUi::<D>::preset_start(Utc::now().date_naive(), days_back),
        )
    }

    /// The geomagnetic and TEC presets reach back five years and stop there,
    /// short of Kp's 1932 start and JPL's 2008 one.
    #[test]
    fn the_bounded_presets_stay_within_five_years() {
        for ((widest, start), coverage_start) in [
            (
                widest_preset::<GeomagneticIndexBackfill>(),
                gt_solar::calendar::COVERAGE_START,
            ),
            (
                widest_preset::<TecMapBackfill>(),
                gt_ionex::calendar::COVERAGE_START,
            ),
        ] {
            assert_eq!(widest, Some(5 * 365));
            assert!(start > coverage_start);
        }
    }

    #[rstest]
    #[case::one_day(1, "1 day, about 2 s and 81.0 KB")]
    #[case::a_month(30, "30 days, about 60 s and 2.4 MB")]
    #[case::a_year(365, "365 days, about 13 min and 28.9 MB")]
    #[case::the_whole_archive(1600, "1600 days, about 54 min and 126.6 MB")]
    fn estimates_scale_from_seconds_to_minutes(#[case] days: usize, #[case] expected: &str) {
        assert_eq!(InterferenceBackfill::estimate(days), expected);
    }

    /// A day of indices costs two requests, and a range long enough to run
    /// for hours is stated in hours.
    #[rstest]
    #[case::one_day(1, "1 day, about 4 s and 12.0 KB")]
    #[case::a_year(365, "365 days, about 25 min and 4.3 MB")]
    #[case::five_years(1825, "1825 days, about 2.0 h and 21.4 MB")]
    fn geomagnetic_estimates_count_a_request_per_index(
        #[case] days: usize,
        #[case] expected: &str,
    ) {
        assert_eq!(GeomagneticIndexBackfill::estimate(days), expected);
    }

    /// A map day costs one request, and its archived footprint dominates the
    /// estimate.
    #[rstest]
    #[case::one_day(1, "1 day, about 2 s and 125.0 KB")]
    #[case::five_years(1825, "1825 days, about 61 min and 222.8 MB")]
    fn tec_estimates_count_one_request_per_day(#[case] days: usize, #[case] expected: &str) {
        assert_eq!(TecMapBackfill::estimate(days), expected);
    }

    /// Nothing is requested until the button is pressed.
    #[test]
    fn rendering_the_control_starts_nothing() {
        let mut state = TestBackfillUi::default();
        let actions = RefCell::new(Vec::new());
        let mut harness = TestHarness::builder().ui(|ui| {
            actions
                .borrow_mut()
                .extend(state.ui(ui, None, BackfillReadiness::Ready));
        });
        harness.run();
        assert!(actions.borrow().is_empty());
    }

    #[test]
    fn pressing_download_starts_the_selected_range() {
        let mut state = TestBackfillUi::default();
        let expected = state.range().expect("the default range is usable");
        let action = RefCell::new(None);
        let mut harness = TestHarness::builder().ui(|ui| {
            if let Some(emitted) = state.ui(ui, None, BackfillReadiness::Ready) {
                *action.borrow_mut() = Some(emitted);
            }
        });
        harness
            .inner
            .get_by_label_contains("Download history")
            .click();
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
        let mut state = state(date(2026, 7, 26), date(2026, 7, 20));
        let mut harness = TestHarness::builder().ui(|ui| {
            state.ui(ui, None, BackfillReadiness::Ready);
        });
        harness.run();
        assert!(
            harness
                .inner
                .get_by_label_contains("Download history")
                .accesskit_node()
                .is_disabled()
        );
    }

    /// Never hidden, per DESIGN.md: what blocks a download grays the button
    /// and says so on hover.
    #[rstest]
    #[case::while_the_archive_opens(BackfillReadiness::ArchiveStillOpening)]
    #[case::without_an_archive(BackfillReadiness::WithoutArchive)]
    #[case::offline(BackfillReadiness::Offline)]
    #[case::without_an_api_key(BackfillReadiness::WithoutApiKey)]
    fn a_blocked_download_leaves_the_button_disabled(#[case] readiness: BackfillReadiness) {
        let mut state = TestBackfillUi::default();
        let mut harness = TestHarness::builder().ui(|ui| {
            state.ui(ui, None, readiness);
        });
        harness.run();
        assert!(
            harness
                .inner
                .get_by_label_contains("Download history")
                .accesskit_node()
                .is_disabled()
        );
    }

    #[rstest]
    #[case::ready(
        BackfillReadiness::Ready,
        "Pick an end date on or after the start date"
    )]
    #[case::while_the_archive_opens(
        BackfillReadiness::ArchiveStillOpening,
        "The interference archive is still opening"
    )]
    #[case::without_an_archive(
        BackfillReadiness::WithoutArchive,
        "There is nowhere to download to: the interference archive could not be opened"
    )]
    #[case::offline(BackfillReadiness::Offline, "Downloading is disabled in offline mode")]
    #[case::without_an_api_key(
        BackfillReadiness::WithoutApiKey,
        "Enter an API key above to download daily interference datasets"
    )]
    fn the_disabled_hover_names_what_blocks_the_download(
        #[case] readiness: BackfillReadiness,
        #[case] expected: &str,
    ) {
        assert_eq!(TestBackfillUi::blocked_hover(readiness), expected);
    }

    /// While running, the range is locked and the only action is cancelling.
    #[test]
    fn a_running_backfill_offers_cancel_instead_of_download() {
        let mut state = TestBackfillUi::default();
        let action = RefCell::new(None);
        let progress = BackfillProgress { done: 3, total: 10 };
        let mut harness = TestHarness::builder().ui(|ui| {
            if let Some(emitted) = state.ui(ui, Some(progress), BackfillReadiness::Ready) {
                *action.borrow_mut() = Some(emitted);
            }
        });
        harness.run();
        assert!(
            harness
                .inner
                .query_by_label_contains("Download history")
                .is_none()
        );
        harness.inner.get_by_label_contains("Cancel").click();
        harness.run();
        assert_eq!(*action.borrow(), Some(BackfillAction::Cancel));
    }

    #[rstest]
    #[case::no_archive(None, "No interference archive to download into")]
    #[case::nothing_to_do(Some(0), "Every day in that range is already archived")]
    #[case::a_month(Some(30), "Downloading 30 days, about 60 s and 2.4 MB")]
    fn the_outcome_line_reports_what_was_queued(
        #[case] queued: Option<usize>,
        #[case] expected: &str,
    ) {
        let mut state = TestBackfillUi::default();
        state.report_started(queued);
        assert_eq!(state.outcome.as_deref(), Some(expected));
    }

    /// The key a source needs is named after the events it would download.
    #[test]
    fn the_solar_flare_control_asks_for_the_key_its_host_needs() {
        assert_eq!(
            BackfillUi::<SolarFlareBackfill>::blocked_hover(BackfillReadiness::WithoutApiKey),
            "Enter an API key above to download solar flare events"
        );
    }

    /// The outcome line and the disabled hover name the archive of the dataset
    /// the control downloads.
    #[test]
    fn the_geomagnetic_control_names_its_own_archive() {
        let mut state = BackfillUi::<GeomagneticIndexBackfill>::default();
        state.report_started(None);
        assert_eq!(
            state.outcome.as_deref(),
            Some("No geomagnetic index archive to download into")
        );
        assert_eq!(
            BackfillUi::<GeomagneticIndexBackfill>::blocked_hover(
                BackfillReadiness::WithoutArchive
            ),
            "There is nowhere to download to: the geomagnetic index archive could not be opened"
        );
    }
}
