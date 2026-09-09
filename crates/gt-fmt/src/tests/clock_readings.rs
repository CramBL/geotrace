use chrono::TimeDelta;
use rstest::rstest;

use crate::{DurationClockFormat, MINUS_SIGN};

#[rstest]
#[case::zero(0, "0s")]
#[case::below_a_minute(42, "42s")]
#[case::the_last_second_below_a_minute(59, "59s")]
#[case::a_minute(60, "1:00min")]
#[case::minutes_and_seconds(754, "12:34min")]
fn a_match_duration_reads_in_seconds_below_a_minute(#[case] secs: i64, #[case] text: &str) {
    assert_eq!(crate::format_match_duration(secs), text);
}

#[rstest]
#[case::zero(DurationClockFormat::MinutesSeconds, 0, "0:00")]
#[case::below_a_minute(DurationClockFormat::MinutesSeconds, 42, "0:42")]
#[case::a_minute(DurationClockFormat::MinutesSeconds, 60, "1:00")]
#[case::minutes_count_past_an_hour(DurationClockFormat::MinutesSeconds, 3_600, "60:00")]
#[case::negative(DurationClockFormat::MinutesSeconds, -61, &format!("{MINUS_SIGN}1:01"))]
#[case::widened_zero(DurationClockFormat::HoursMinutesSeconds, 0, "0:00:00")]
#[case::widened_below_an_hour(DurationClockFormat::HoursMinutesSeconds, 3_599, "0:59:59")]
#[case::an_hour(DurationClockFormat::HoursMinutesSeconds, 3_600, "1:00:00")]
#[case::hours(DurationClockFormat::HoursMinutesSeconds, 45_296, "12:34:56")]
#[case::hours_count_past_a_day(DurationClockFormat::HoursMinutesSeconds, 94_205, "26:10:05")]
fn a_clock_reading_prints_the_fields_of_its_format(
    #[case] format: DurationClockFormat,
    #[case] secs: i64,
    #[case] text: &str,
) {
    assert_eq!(format.format_seconds(secs), text);
}

#[rstest]
#[case::zero(0, DurationClockFormat::MinutesSeconds)]
#[case::the_last_second_below_an_hour(3_599, DurationClockFormat::MinutesSeconds)]
#[case::an_hour(3_600, DurationClockFormat::HoursMinutesSeconds)]
fn the_longest_duration_decides_the_clock_format(
    #[case] longest_secs: i64,
    #[case] expected: DurationClockFormat,
) {
    assert_eq!(
        DurationClockFormat::fitting_longest_duration(longest_secs),
        expected
    );
}

#[rstest]
#[case::zero(0, "0:00")]
#[case::under_a_minute(64, "1:04")]
#[case::past_an_hour(3725, "1:02:05")]
#[case::negative_clamps_to_zero(-5, "0:00")]
fn a_timeline_offset_reads_like_a_scrubber(#[case] secs: i64, #[case] expected: &str) {
    assert_eq!(
        crate::format_timeline_offset(TimeDelta::seconds(secs)),
        expected
    );
}
