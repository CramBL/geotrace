//! Which UTC days are worth requesting, and for which index.
//!
//! Two answers are knowable without asking: an index has no values before
//! [`GeomagneticIndex::coverage_start`], and none for a day that has not
//! happened. Everything else is [`DayOutlook::Fetchable`], the current day
//! included: GFZ publishes a nowcast value for the running period and replaces
//! it with a definitive one once every station has reported.

use chrono::NaiveDate;
use strum::IntoEnumIterator as _;

use crate::GeomagneticIndex;

/// Most UTC days one recording is allowed to pull in.
///
/// A recording spanning longer than this is left to an explicit backfill: a
/// recording should not silently turn into hundreds of requests.
pub const MAX_DAYS_PER_TRACK: usize = 7;

/// What the calendar alone says about one index on one day, before any
/// request is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::EnumIter)]
#[strum(serialize_all = "snake_case")]
pub enum DayOutlook {
    /// Inside the index's coverage and not in the future. Worth requesting,
    /// even if the service turns out to have no value for it.
    Fetchable,
    /// Earlier than the index's first published day.
    BeforeCoverage,
    /// Later than the current UTC day.
    InFuture,
}

/// The calendar's verdict on `index` for `day`.
pub fn day_outlook(index: GeomagneticIndex, day: NaiveDate, today_utc: NaiveDate) -> DayOutlook {
    if day < index.coverage_start() {
        DayOutlook::BeforeCoverage
    } else if day > today_utc {
        DayOutlook::InFuture
    } else {
        DayOutlook::Fetchable
    }
}

/// The indices worth requesting for `day`, in [`GeomagneticIndex`] declaration
/// order.
///
/// The two indices can differ per day, not only per recording: a receiver
/// without time lock timestamps its fixes in 1970, which is inside Kp's
/// coverage but before Hp30's.
pub fn fetchable_indices(
    day: NaiveDate,
    today_utc: NaiveDate,
) -> impl Iterator<Item = GeomagneticIndex> {
    GeomagneticIndex::iter()
        .filter(move |index| day_outlook(*index, day, today_utc) == DayOutlook::Fetchable)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn today() -> NaiveDate {
        date(2026, 7, 29)
    }

    #[rstest]
    #[case::before_kp(GeomagneticIndex::Kp, date(1931, 12, 31), DayOutlook::BeforeCoverage)]
    #[case::the_first_kp_day(GeomagneticIndex::Kp, date(1932, 1, 1), DayOutlook::Fetchable)]
    #[case::before_hp30(GeomagneticIndex::Hp30, date(1984, 12, 31), DayOutlook::BeforeCoverage)]
    #[case::the_first_hp30_day(GeomagneticIndex::Hp30, date(1985, 1, 1), DayOutlook::Fetchable)]
    #[case::a_settled_day(GeomagneticIndex::Hp30, date(2026, 7, 20), DayOutlook::Fetchable)]
    #[case::today(GeomagneticIndex::Kp, date(2026, 7, 29), DayOutlook::Fetchable)]
    #[case::tomorrow(GeomagneticIndex::Kp, date(2026, 7, 30), DayOutlook::InFuture)]
    fn day_outlook_covers_every_calendar_boundary(
        #[case] index: GeomagneticIndex,
        #[case] day: NaiveDate,
        #[case] expected: DayOutlook,
    ) {
        assert_eq!(day_outlook(index, day, today()), expected);
    }

    /// A variant cannot be added without a day that reaches it.
    #[test]
    fn every_outlook_is_reachable() {
        let reached: HashSet<DayOutlook> = [
            day_outlook(GeomagneticIndex::Kp, date(1900, 1, 1), today()),
            day_outlook(GeomagneticIndex::Kp, today(), today()),
            day_outlook(GeomagneticIndex::Kp, date(2030, 1, 1), today()),
        ]
        .into_iter()
        .collect();
        assert_eq!(reached, DayOutlook::iter().collect::<HashSet<_>>());
    }

    /// A 1970 day, as a receiver without time lock reports, has Kp and no
    /// Hp30.
    #[rstest]
    #[case::both_indices(date(2026, 7, 20), vec![GeomagneticIndex::Kp, GeomagneticIndex::Hp30])]
    #[case::kp_only(date(1970, 1, 1), vec![GeomagneticIndex::Kp])]
    #[case::in_the_future(date(2026, 7, 30), vec![])]
    #[case::before_every_index(date(1900, 1, 1), vec![])]
    fn fetchable_indices_drops_what_the_service_cannot_have(
        #[case] day: NaiveDate,
        #[case] expected: Vec<GeomagneticIndex>,
    ) {
        assert_eq!(
            fetchable_indices(day, today()).collect::<Vec<_>>(),
            expected
        );
    }
}
