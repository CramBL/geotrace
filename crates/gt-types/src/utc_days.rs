//! Walking whole UTC days, the granularity every day-keyed dataset is
//! published and archived at.

use std::ops::RangeInclusive;

use chrono::{Days, NaiveDate};

/// The days in `range` that `keep` accepts, oldest first. A range that runs
/// backwards holds no days.
pub fn days_in_range(
    range: RangeInclusive<NaiveDate>,
    mut keep: impl FnMut(NaiveDate) -> bool,
) -> Vec<NaiveDate> {
    let mut days = Vec::new();
    let mut day = *range.start();
    while day <= *range.end() {
        if keep(day) {
            days.push(day);
        }
        let Some(next) = day.checked_add_days(Days::new(1)) else {
            break;
        };
        day = next;
    }
    days
}

#[cfg(test)]
mod tests {
    use chrono::Datelike as _;
    use rstest::rstest;

    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    #[rstest]
    #[case::one_day(date(2026, 7, 20), date(2026, 7, 20), 1)]
    #[case::a_week(date(2026, 7, 20), date(2026, 7, 26), 7)]
    #[case::across_a_leap_day(date(2024, 2, 28), date(2024, 3, 1), 3)]
    #[case::reversed(date(2026, 7, 26), date(2026, 7, 20), 0)]
    fn every_day_of_the_range_is_walked(
        #[case] from: NaiveDate,
        #[case] to: NaiveDate,
        #[case] expected: usize,
    ) {
        let days = days_in_range(from..=to, |_| true);
        assert_eq!(days.len(), expected);
        assert_eq!(days.first().copied(), (expected > 0).then_some(from));
        assert_eq!(days.last().copied(), (expected > 0).then_some(to));
    }

    #[test]
    fn a_rejected_day_is_left_out() {
        let days = days_in_range(date(2026, 7, 20)..=date(2026, 7, 26), |day| {
            day.day() % 2 == 0
        });
        assert_eq!(
            days,
            [
                date(2026, 7, 20),
                date(2026, 7, 22),
                date(2026, 7, 24),
                date(2026, 7, 26)
            ]
        );
    }

    #[test]
    fn the_walk_stops_at_the_end_of_the_calendar() {
        assert_eq!(
            days_in_range(NaiveDate::MAX..=NaiveDate::MAX, |_| true),
            [NaiveDate::MAX]
        );
    }
}
