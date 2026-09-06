use geotrace_sdk::{Angle, DateTime, Error, Timestamp, Utc};
use proptest::prelude::*;
use rstest::rstest;

#[test]
#[expect(
    clippy::float_cmp,
    reason = "whole-degree inputs make every arc exact in f64"
)]
fn signed_arc_crosses_the_compass_wraparound() {
    let arc = |from: f64, to: f64| Angle::degrees(from).signed_arc_to(Angle::degrees(to));
    assert_eq!(arc(359.0, 1.0).as_degrees(), 2.0);
    assert_eq!(arc(1.0, 359.0).as_degrees(), -2.0);
    assert_eq!(
        arc(0.0, 180.0).as_degrees(),
        -180.0,
        "the tie lands at -180"
    );
    assert_eq!(arc(90.0, 45.0).as_degrees(), -45.0);
    assert_eq!(arc(45.0, 45.0).as_degrees(), 0.0);
}

proptest! {
    /// The arc always lands in [-180, 180) and, applied to the start,
    /// reproduces the target heading (mod 360).
    #[test]
    fn signed_arc_stays_in_range_and_recomposes(from in -720.0_f64..720.0, to in -720.0_f64..720.0) {
        let arc = Angle::degrees(from).signed_arc_to(Angle::degrees(to)).as_degrees();
        prop_assert!((-180.0..180.0).contains(&arc));
        let recomposed = (from + arc).rem_euclid(360.0);
        prop_assert!((recomposed - to.rem_euclid(360.0)).abs() < 1e-9);
    }
}

#[rstest]
#[case(Timestamp::try_from_unix_seconds(1_700_000_000), 1_700_000_000_000_000)]
#[case(
    Timestamp::try_from_unix_millis(1_700_000_000_123),
    1_700_000_000_123_000
)]
#[case(
    Timestamp::try_from_unix_micros(1_700_000_000_123_456),
    1_700_000_000_123_456
)]
#[case(
    Timestamp::try_from_unix_nanos(1_700_000_000_123_456_789),
    1_700_000_000_123_456
)]
#[case(Timestamp::try_from_unix_seconds(-1), -1_000_000)]
#[case(Timestamp::try_from_unix_millis(-1), -1_000)]
#[case(Timestamp::try_from_unix_micros(-1), -1)]
#[case(Timestamp::try_from_unix_nanos(-1_000), -1)]
fn a_unit_constructor_converts_a_count_to_its_microseconds(
    #[case] converted: Result<Timestamp, Error>,
    #[case] expected_micros: i64,
) {
    let timestamp = converted.expect("the count is inside the range a UTC timestamp covers");
    assert_eq!(
        DateTime::<Utc>::from(timestamp).timestamp_micros(),
        expected_micros
    );
}

#[rstest]
#[case(
    Timestamp::try_from_unix_seconds(i64::MAX),
    "9223372036854775807 seconds since the Unix epoch is past the range a UTC timestamp covers"
)]
#[case(
    Timestamp::try_from_unix_millis(i64::MAX),
    "9223372036854775807 milliseconds since the Unix epoch is past the range a UTC timestamp covers"
)]
#[case(
    Timestamp::try_from_unix_micros(i64::MAX),
    "9223372036854775807 microseconds since the Unix epoch is past the range a UTC timestamp covers"
)]
fn a_unit_constructor_reports_a_count_past_the_range_with_its_unit(
    #[case] converted: Result<Timestamp, Error>,
    #[case] expected_message: &str,
) {
    let error = converted.expect_err("the count is past the range a UTC timestamp covers");
    assert_eq!(error.to_string(), expected_message);
}

#[test]
fn try_from_unix_nanos_truncates_towards_zero() {
    let unix_micros_from_nanos = |nanos| {
        let timestamp = Timestamp::try_from_unix_nanos(nanos).expect("well inside the range");
        DateTime::<Utc>::from(timestamp).timestamp_micros()
    };
    assert_eq!(unix_micros_from_nanos(999), 0);
    assert_eq!(unix_micros_from_nanos(-999), 0);
}

#[test]
fn try_from_unix_nanos_converts_the_largest_i64_count() {
    let timestamp = Timestamp::try_from_unix_nanos(i64::MAX).expect("2262 is inside the range");
    assert_eq!(
        DateTime::<Utc>::from(timestamp).timestamp_micros(),
        9_223_372_036_854_775
    );
}
