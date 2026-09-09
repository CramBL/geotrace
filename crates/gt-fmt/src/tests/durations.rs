use chrono::TimeDelta;
use rstest::rstest;

use crate::MINUS_SIGN;

#[rstest]
#[case::zero(TimeDelta::zero(), "0s")]
#[case::one_millisecond(TimeDelta::milliseconds(1), "0.1s")]
#[case::a_third_of_a_second(TimeDelta::milliseconds(333), "0.4s")]
#[case::just_under_a_second(TimeDelta::milliseconds(999), "0.9s")]
#[case::exactly_one_second(TimeDelta::milliseconds(1_000), "1s")]
#[case::negative_under_a_second(TimeDelta::milliseconds(-500), "0s")]
#[case::seconds_below_two_minutes(TimeDelta::seconds(45), "45s")]
#[case::minutes_and_seconds(TimeDelta::minutes(1) + TimeDelta::seconds(30), "1m30s")]
#[case::just_under_two_minutes(TimeDelta::minutes(1) + TimeDelta::seconds(59), "1m59s")]
#[case::exactly_two_minutes(TimeDelta::minutes(2), "2m")]
#[case::minutes_alone(TimeDelta::minutes(20), "20m")]
#[case::hours_and_minutes(
    TimeDelta::hours(1) + TimeDelta::minutes(28) + TimeDelta::seconds(15),
    "1h28m"
)]
#[case::hours_alone(TimeDelta::hours(2), "2h")]
#[case::three_hours_drop_the_minutes(
    TimeDelta::hours(3) + TimeDelta::minutes(45) + TimeDelta::seconds(10),
    "3h"
)]
#[case::just_under_two_days(TimeDelta::hours(47), "47h")]
#[case::exactly_two_days(TimeDelta::hours(48), "2d")]
#[case::days_and_hours(TimeDelta::hours(53), "2d5h")]
#[case::nearly_five_days(TimeDelta::hours(119), "4d23h")]
fn the_terse_reading_keeps_every_scale(#[case] duration: TimeDelta, #[case] expected: &str) {
    assert_eq!(crate::format_human_terse_duration(duration), expected);
}

#[rstest]
#[case::zero(TimeDelta::zero(), "0s")]
#[case::one_microsecond(TimeDelta::microseconds(1), "1µs")]
#[case::sub_millisecond(TimeDelta::microseconds(900), "900µs")]
#[case::whole_milliseconds(TimeDelta::milliseconds(4), "4ms")]
#[case::milliseconds_with_a_fraction(TimeDelta::microseconds(4_500), "4.5ms")]
#[case::milliseconds_to_the_microsecond(TimeDelta::microseconds(250_125), "250.125ms")]
#[case::just_under_a_second(TimeDelta::microseconds(999_999), "999.999ms")]
#[case::whole_seconds(TimeDelta::seconds(4), "4s")]
#[case::seconds_with_a_fraction(TimeDelta::milliseconds(1_500), "1.5s")]
#[case::seconds_cut_at_milliseconds(TimeDelta::microseconds(1_500_400), "1.5s")]
#[case::just_under_a_minute(TimeDelta::microseconds(59_999_999), "59.999s")]
#[case::a_minute(TimeDelta::seconds(60), "1m")]
#[case::past_a_minute(TimeDelta::seconds(90), "1m30s")]
#[case::negative_microseconds(TimeDelta::microseconds(-900), &format!("{MINUS_SIGN}900µs"))]
#[case::negative_past_a_minute(TimeDelta::seconds(-90), &format!("{MINUS_SIGN}1m30s"))]
fn the_microsecond_reading_keeps_every_scale(#[case] duration: TimeDelta, #[case] expected: &str) {
    assert_eq!(
        crate::format_human_terse_duration_with_microseconds(duration),
        expected
    );
}

/// [`chrono::TimeDelta::num_microseconds`] overflows past about 292 000
/// years, where the terse reading stands on its own.
#[test]
fn a_duration_past_the_microsecond_range_reads_in_days() {
    assert_eq!(
        crate::format_human_terse_duration_with_microseconds(TimeDelta::MAX),
        "106751991167d7h"
    );
}

#[rstest]
#[case::milliseconds(250, "+250ms")]
#[case::negative_milliseconds(-50, &format!("{MINUS_SIGN}50ms"))]
#[case::just_under_two_seconds(1_999, "+1999ms")]
#[case::exactly_two_seconds(2_000, "+2s")]
#[case::one_decimal_place(2_100, "+2.1s")]
#[case::two_decimal_places(2_140, "+2.14s")]
#[case::seconds_with_a_fraction(9_230, "+9.23s")]
#[case::negative_two_decimal_places(-2_140, &format!("{MINUS_SIGN}2.14s"))]
#[case::just_under_a_minute(59_990, "+59.99s")]
#[case::a_minute(60_000, "+1m")]
#[case::minutes_and_seconds(69_000, "+1m9s")]
#[case::hours_minutes_and_seconds(3_661_000, "+1h1m1s")]
fn the_signed_delta_reading_keeps_every_scale(#[case] delta_ms: i64, #[case] expected: &str) {
    assert_eq!(crate::format_signed_delta(delta_ms), expected);
}
