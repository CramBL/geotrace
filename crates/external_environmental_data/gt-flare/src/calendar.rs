//! Which UTC days are worth requesting.
//!
//! Two facts are knowable without a request: the catalog lists nothing before
//! [`COVERAGE_START`], and nothing for a day that has not happened. Everything
//! else is [`DayOutlook::Fetchable`], the current day included, since an event
//! is submitted after it is observed.

use chrono::NaiveDate;

/// First day the catalog lists an event for, probed against the endpoint.
pub const COVERAGE_START: NaiveDate = coverage_start();

const COVERAGE_START_YMD: (i32, u32, u32) = (2010, 4, 3);

const fn coverage_start() -> NaiveDate {
    let (year, month, day) = COVERAGE_START_YMD;
    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => date,
        // Dead arm: `const` evaluation cannot unwrap without panicking, and the
        // assertion below fails the build if it ever stops being dead.
        None => NaiveDate::MIN,
    }
}

const _: () = {
    let (year, month, day) = COVERAGE_START_YMD;
    assert!(
        NaiveDate::from_ymd_opt(year, month, day).is_some(),
        "COVERAGE_START_YMD must name a real calendar date"
    );
};

/// Most UTC days one recording is allowed to pull in.
///
/// A recording spanning longer than this is left to an explicit backfill: a
/// recording should not silently turn into hundreds of requests.
pub const MAX_DAYS_PER_TRACK: usize = 7;

/// What the calendar alone says about one day, before any request is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum DayOutlook {
    /// Inside the catalog's coverage and not in the future. Worth requesting,
    /// even if the catalog turns out to have no flare for it.
    Fetchable,
    /// Earlier than the catalog's first listed day.
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

/// Every day in `from..=to` the catalog can list events for, oldest first.
pub fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate> {
    // The lower bound keeps a range starting in the year 1 out of the walk.
    gt_types::utc_days::days_in_range(from.max(COVERAGE_START)..=to, |day| {
        day_outlook(day, today_utc) == DayOutlook::Fetchable
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;
    use strum::IntoEnumIterator as _;

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn today() -> NaiveDate {
        date(2026, 7, 29)
    }

    #[rstest]
    #[case::before_the_catalog(date(2010, 4, 2), DayOutlook::BeforeCoverage)]
    #[case::the_first_listed_day(date(2010, 4, 3), DayOutlook::Fetchable)]
    #[case::a_settled_day(date(2024, 5, 9), DayOutlook::Fetchable)]
    #[case::today(date(2026, 7, 29), DayOutlook::Fetchable)]
    #[case::tomorrow(date(2026, 7, 30), DayOutlook::InFuture)]
    fn day_outlook_covers_every_calendar_boundary(
        #[case] day: NaiveDate,
        #[case] expected: DayOutlook,
    ) {
        assert_eq!(day_outlook(day, today()), expected);
    }

    /// A variant cannot be added without a day that reaches it.
    #[test]
    fn every_outlook_is_reachable() {
        let reached: HashSet<DayOutlook> = [
            day_outlook(date(1970, 1, 1), today()),
            day_outlook(today(), today()),
            day_outlook(date(2030, 1, 1), today()),
        ]
        .into_iter()
        .collect();
        assert_eq!(reached, DayOutlook::iter().collect::<HashSet<_>>());
    }

    #[rstest]
    #[case::a_week(date(2026, 7, 20), date(2026, 7, 26), 7)]
    #[case::one_day(date(2026, 7, 20), date(2026, 7, 20), 1)]
    #[case::reversed(date(2026, 7, 26), date(2026, 7, 20), 0)]
    #[case::clamped_to_coverage(date(1900, 1, 1), COVERAGE_START, 1)]
    #[case::stops_at_today(date(2026, 7, 27), date(2026, 8, 10), 3)]
    #[case::entirely_in_the_future(date(2026, 8, 1), date(2026, 8, 10), 0)]
    fn fetchable_days_covers_the_range_the_catalog_can_serve(
        #[case] from: NaiveDate,
        #[case] to: NaiveDate,
        #[case] expected: usize,
    ) {
        let days = fetchable_days(from, to, today());
        assert_eq!(days.len(), expected);
        assert!(days.iter().all(|day| *day >= COVERAGE_START));
        assert!(days.iter().all(|day| *day <= today()));
    }
}
