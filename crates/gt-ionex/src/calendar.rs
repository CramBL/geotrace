//! Which UTC days are worth requesting.
//!
//! Two facts are knowable without a request: JPL has no maps before
//! [`COVERAGE_START`], and none for a day that has not happened. Everything
//! else is [`DayOutlook::Fetchable`], the current day included, where both
//! products return 404 until the day has been processed.

use chrono::NaiveDate;

use crate::IonexProduct;

/// First UTC day JPL published a final map for, day 324 of 2008.
///
/// Read off the earliest `JPLG` file in `IONEX_final/y2008/`, checked on
/// 2026-08-17.
pub const COVERAGE_START: NaiveDate = coverage_start(COVERAGE_START_YMD);

const COVERAGE_START_YMD: (i32, u32, u32) = (2008, 11, 19);

const fn coverage_start((year, month, day): (i32, u32, u32)) -> NaiveDate {
    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => date,
        // Dead arm: const evaluation cannot unwrap without panicking, and the
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
    /// Inside JPL's coverage and not in the future. Worth requesting, even if
    /// neither product turns out to have a file for it.
    Fetchable,
    /// Earlier than the first published day.
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

/// The products worth requesting for `day`, settled product first, or nothing
/// for a day outside coverage.
pub fn fetchable_products(day: NaiveDate, today_utc: NaiveDate) -> &'static [IonexProduct] {
    match day_outlook(day, today_utc) {
        DayOutlook::Fetchable => &IonexProduct::PREFERENCE_ORDER,
        DayOutlook::BeforeCoverage | DayOutlook::InFuture => &[],
    }
}

/// Every day in `from..=to` inside coverage, oldest first.
pub fn fetchable_days(from: NaiveDate, to: NaiveDate, today_utc: NaiveDate) -> Vec<NaiveDate> {
    // The lower bound keeps a range starting in the year 1 out of the walk.
    gt_types::utc_days::days_in_range(from.max(COVERAGE_START)..=to, |day| {
        !fetchable_products(day, today_utc).is_empty()
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
        date(2026, 8, 17)
    }

    #[rstest]
    #[case::before_coverage(date(2008, 11, 18), DayOutlook::BeforeCoverage)]
    #[case::the_first_published_day(date(2008, 11, 19), DayOutlook::Fetchable)]
    #[case::a_settled_day(date(2024, 5, 10), DayOutlook::Fetchable)]
    #[case::today(date(2026, 8, 17), DayOutlook::Fetchable)]
    #[case::tomorrow(date(2026, 8, 18), DayOutlook::InFuture)]
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
            day_outlook(date(1900, 1, 1), today()),
            day_outlook(today(), today()),
            day_outlook(date(2030, 1, 1), today()),
        ]
        .into_iter()
        .collect();
        assert_eq!(reached, DayOutlook::iter().collect::<HashSet<_>>());
    }

    /// A fetchable day requests the settled product first, and a day outside
    /// coverage is never requested at all.
    #[rstest]
    #[case::a_settled_day(date(2024, 5, 10), &[IonexProduct::Final, IonexProduct::Rapid])]
    #[case::before_coverage(date(1970, 1, 1), &[])]
    #[case::in_the_future(date(2026, 8, 18), &[])]
    fn fetchable_products_orders_the_settled_product_first(
        #[case] day: NaiveDate,
        #[case] expected: &[IonexProduct],
    ) {
        assert_eq!(fetchable_products(day, today()), expected);
    }

    #[rstest]
    #[case::a_week(date(2024, 5, 10), date(2024, 5, 16), 7)]
    #[case::one_day(date(2024, 5, 10), date(2024, 5, 10), 1)]
    #[case::reversed(date(2024, 5, 16), date(2024, 5, 10), 0)]
    #[case::clamped_to_coverage(date(1900, 1, 1), COVERAGE_START, 1)]
    #[case::stops_at_today(date(2026, 8, 15), date(2026, 8, 30), 3)]
    #[case::entirely_in_the_future(date(2026, 9, 1), date(2026, 9, 10), 0)]
    fn fetchable_days_covers_the_range_the_archive_can_serve(
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
