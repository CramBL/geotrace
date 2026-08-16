//! Which UTC days are worth requesting.
//!
//! Only two answers are knowable without asking: nothing exists before
//! [`COVERAGE_START`], and nothing exists for a day that has not happened.
//! Everything else is [`DayOutlook::Fetchable`], including days too recent
//! to be published - the lag runs one to three days and is unannounced, so
//! gating on it would hide days the host does serve.
//!
//! [`awaiting_publication`] only shapes how a 404 is worded.

use chrono::{DateTime, Days, NaiveDate, Utc};
use gt_types::TimeRange;

/// The first day of coverage, as (year, month, day).
///
/// Probed, not taken from the publisher's prose, which names February 2022
/// without a day: on 2026-07-31 every day from 2022-01-31 through
/// 2022-02-13 answered 404, and 2022-02-14 answered 200.
const COVERAGE_START_YMD: (i32, u32, u32) = (2022, 2, 14);

/// First UTC day the host published a dataset for.
pub const COVERAGE_START: NaiveDate = {
    let (year, month, day) = COVERAGE_START_YMD;
    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => date,
        // Dead arm: const evaluation cannot unwrap without panicking, and
        // the assert below fails the build if it ever stops being dead.
        None => NaiveDate::MIN,
    }
};

const _: () = {
    let (year, month, day) = COVERAGE_START_YMD;
    assert!(
        NaiveDate::from_ymd_opt(year, month, day).is_some(),
        "COVERAGE_START_YMD must name a real calendar date"
    );
};

/// How far behind the current day the host typically runs. Observed at one
/// to three days. The largest is used so a 404 inside the window is worded
/// as not published yet.
pub const TYPICAL_PUBLICATION_LAG: Days = Days::new(3);

/// What the calendar alone says about a day, before any request is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum DayOutlook {
    /// Inside the coverage window and not in the future. Worth requesting,
    /// even if it turns out to be unpublished or a gap.
    Fetchable,
    /// Earlier than [`COVERAGE_START`].
    BeforeCoverage,
    /// Later than the current UTC day.
    InFuture,
}

/// The calendar's verdict on `day`.
pub fn day_outlook(day: NaiveDate, today_utc: NaiveDate) -> DayOutlook {
    if day < COVERAGE_START {
        DayOutlook::BeforeCoverage
    } else if day > today_utc {
        DayOutlook::InFuture
    } else {
        DayOutlook::Fetchable
    }
}

/// Whether `day` is recent enough that the host is not expected to have
/// published it yet.
///
/// Distinguishes a pending day from a gap in the record when a dataset is
/// missing. Never decides whether to request one.
pub fn awaiting_publication(day: NaiveDate, today_utc: NaiveDate) -> bool {
    match today_utc.checked_sub_days(TYPICAL_PUBLICATION_LAG) {
        Some(newest_expected) => day > newest_expected,
        // Only reachable within three days of `NaiveDate::MIN`.
        None => true,
    }
}

/// Most UTC days one recording is allowed to pull in.
///
/// A track spanning longer than this is left to an explicit backfill: a
/// recording should not silently turn into hundreds of requests.
pub const MAX_DAYS_PER_TRACK: usize = 7;

/// The UTC days `start..=end` touches, oldest first.
///
/// [`None`] when the span covers more than [`MAX_DAYS_PER_TRACK`], or when
/// `end` precedes `start`.
pub fn days_spanned(start: DateTime<Utc>, end: DateTime<Utc>) -> Option<Vec<NaiveDate>> {
    TimeRange::new(start, end).utc_days(MAX_DAYS_PER_TRACK)
}

/// Every [`DayOutlook::Fetchable`] day in `from..=to`, oldest first.
pub fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate> {
    let mut days = Vec::new();
    // Bounds the walk for a caller that asks from the year 1.
    let mut day = from.max(COVERAGE_START);
    while day <= to {
        if day_outlook(day, today_utc) == DayOutlook::Fetchable {
            days.push(day);
        }
        let Some(next) = day.checked_add_days(Days::new(1)) else {
            break;
        };
        day = next;
    }
    days
}

/// The current UTC day. Datasets are UTC-day granular, so a local date is
/// never the right input.
pub fn today_utc() -> NaiveDate {
    Utc::now().date_naive()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;
    use strum::IntoEnumIterator;

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn coverage_start_is_the_first_published_day() {
        assert_eq!(COVERAGE_START, date(2022, 2, 14));
    }

    #[rstest]
    #[case::the_day_before_coverage(date(2022, 2, 13), DayOutlook::BeforeCoverage)]
    #[case::long_before_coverage(date(2019, 6, 4), DayOutlook::BeforeCoverage)]
    #[case::the_first_covered_day(date(2022, 2, 14), DayOutlook::Fetchable)]
    #[case::a_settled_past_day(date(2026, 7, 20), DayOutlook::Fetchable)]
    // Inside the publication lag: still requested, the host decides.
    #[case::yesterday(date(2026, 7, 28), DayOutlook::Fetchable)]
    #[case::today(date(2026, 7, 29), DayOutlook::Fetchable)]
    #[case::tomorrow(date(2026, 7, 30), DayOutlook::InFuture)]
    fn day_outlook_covers_every_calendar_boundary(
        #[case] day: NaiveDate,
        #[case] expected: DayOutlook,
    ) {
        assert_eq!(day_outlook(day, date(2026, 7, 29)), expected);
    }

    /// A variant cannot be added without a day that reaches it.
    #[test]
    fn every_outlook_is_reachable() {
        let today = date(2026, 7, 29);
        let reached: HashSet<DayOutlook> = [
            day_outlook(date(2021, 1, 1), today),
            day_outlook(today, today),
            day_outlook(date(2030, 1, 1), today),
        ]
        .into_iter()
        .collect();
        let declared: HashSet<DayOutlook> = DayOutlook::iter().collect();
        assert_eq!(reached, declared);
    }

    fn at(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        date(year, month, day)
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            .and_utc()
    }

    #[rstest]
    #[case::within_one_day(at(2026, 7, 20, 8), at(2026, 7, 20, 17), Some(vec![date(2026, 7, 20)]))]
    #[case::across_midnight(
        at(2026, 7, 20, 23),
        at(2026, 7, 21, 1),
        Some(vec![date(2026, 7, 20), date(2026, 7, 21)])
    )]
    #[case::exactly_the_limit(
        at(2026, 7, 20, 0),
        at(2026, 7, 26, 23),
        Some(vec![
            date(2026, 7, 20),
            date(2026, 7, 21),
            date(2026, 7, 22),
            date(2026, 7, 23),
            date(2026, 7, 24),
            date(2026, 7, 25),
            date(2026, 7, 26),
        ])
    )]
    #[case::one_past_the_limit(at(2026, 7, 20, 0), at(2026, 7, 27, 0), None)]
    #[case::end_before_start(at(2026, 7, 21, 0), at(2026, 7, 20, 0), None)]
    fn days_spanned_covers_the_recording(
        #[case] start: DateTime<Utc>,
        #[case] end: DateTime<Utc>,
        #[case] expected: Option<Vec<NaiveDate>>,
    ) {
        assert_eq!(days_spanned(start, end), expected);
    }

    #[rstest]
    #[case::a_week(date(2026, 7, 20), date(2026, 7, 26), 7)]
    #[case::one_day(date(2026, 7, 20), date(2026, 7, 20), 1)]
    #[case::reversed(date(2026, 7, 26), date(2026, 7, 20), 0)]
    #[case::clamped_to_coverage(date(2020, 1, 1), COVERAGE_START, 1)]
    #[case::clamped_to_today(date(2026, 7, 29), date(2027, 1, 1), 3)]
    #[case::entirely_before_coverage(date(2019, 1, 1), date(2020, 1, 1), 0)]
    #[case::entirely_in_the_future(date(2027, 1, 1), date(2027, 2, 1), 0)]
    fn fetchable_days_covers_the_range_inside_coverage(
        #[case] from: NaiveDate,
        #[case] to: NaiveDate,
        #[case] expected: usize,
    ) {
        let days = fetchable_days(from, to, date(2026, 7, 31));
        assert_eq!(days.len(), expected);
        assert!(days.iter().all(|day| *day >= COVERAGE_START));
        assert!(days.windows(2).all(|pair| pair[0] < pair[1]), "ascending");
    }

    #[rstest]
    // The lag is three days, so the three most recent days are pending.
    #[case::today(date(2026, 7, 29), true)]
    #[case::yesterday(date(2026, 7, 28), true)]
    #[case::two_days_back(date(2026, 7, 27), true)]
    #[case::the_newest_expected_day(date(2026, 7, 26), false)]
    #[case::a_settled_day(date(2026, 7, 20), false)]
    fn awaiting_publication_marks_only_the_lag_window(
        #[case] day: NaiveDate,
        #[case] expected: bool,
    ) {
        assert_eq!(awaiting_publication(day, date(2026, 7, 29)), expected);
    }
}
