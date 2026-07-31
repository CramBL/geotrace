//! Which UTC days are worth requesting.
//!
//! Only two answers are knowable without asking: nothing exists before
//! [`COVERAGE_START`], and nothing exists for a day that has not happened.
//! Everything else is [`DayOutlook::Fetchable`], including days too recent
//! to be published - the lag runs one to three days and is unannounced, so
//! gating on it would hide days the host does serve.
//!
//! [`awaiting_publication`] only shapes how a 404 is worded.

use chrono::{Days, NaiveDate, Utc};

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
/// to three days; the largest is used so a 404 inside the window reads as
/// "not published yet" rather than as a hole in the record.
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
///
/// `today_utc` is a parameter rather than a clock read to keep callers
/// testable; the application supplies [`today_utc`].
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
