use super::*;

/// Arithmetic is dimensional algebra: `*` adds dimensions, `/` subtracts,
/// and a dimensionless result is a bare number. A product or quotient of
/// dimensioned values is always well-formed - a wrong combination surfaces
/// at the comparison, not the arithmetic (see the rejected cases below).
#[rstest]
#[case("points | where velocity + 3 km/h > 30 km/h")]
#[case("points | where eph - 3 m > 10 m")]
#[case("points | where velocity * 2 > 30 km/h")]
#[case("points | where 2 * velocity > 30 km/h")]
#[case("points | where sats_fix * 2 > 6")]
#[case("points | where velocity / 2 > 15 km/h")]
// length / length and speed / speed are dimensionless bare numbers.
#[case("points | where eph / eph > 0.5")]
#[case("points | where eph / clock_delta > 1 m/s")]
#[case("points | where velocity / clock_delta > 0.1 m/s2")]
// speed * duration is a length. speed / length is a rate.
#[case("points | where velocity * clock_delta > eph")]
#[case("points | where velocity / eph > 2 per min")]
fn arithmetic_accepts_well_formed_dimensions(#[case] src: &str) {
    test_util::chk(&parse(src).expect(src)).expect(src);
}

/// The rejected side of the algebra. A product or quotient with an exotic
/// dimension type-checks but cannot compare to a bare number. Addition needs a
/// shared dimension. Timestamps, wrapping angles, and conditions reject
/// arithmetic outright.
#[rstest]
#[case(
    "points | where velocity * eph > 3",
    "cannot compare length²/time with number"
)]
#[case(
    "points | where sats_fix / velocity > 3",
    "cannot compare time/length with number"
)]
#[case(
    "points | where velocity + eph > 3 m",
    "unsupported arithmetic between speed and length"
)]
#[case(
    "points | where time - clock_delta > 3 s",
    "timestamps do not support + and -"
)]
#[case(
    "points | where heading + 10 deg < 30 deg",
    "wrapping angles do not support + and -"
)]
#[case(
    "points | where (sats_fix == 1) + 1 > 1",
    "conditions do not support arithmetic"
)]
fn arithmetic_rejects_with_message(#[case] src: &str, #[case] expected: &str) {
    let message = test_util::chk(&parse(src).expect(src))
        .expect_err(src)
        .message;
    assert_eq!(message, expected, "for {src}");
}

#[test]
fn min_unit_and_min_aggregate_coexist() {
    // Position disambiguates: after a number `min` is the minute unit,
    // before `(` it is the aggregate.
    test_util::checked(
        "points | window 3 | where delta(time) <= 15 min and min(velocity) > 5 km/h",
    );
}

#[test]
fn division_by_a_call_named_like_a_unit_round_trips() {
    // `min` names both the minute unit and the aggregate. After a unit,
    // `/ min(...)` is division by a call, not a `deg/min` compound unit -
    // the shape the printer emits for `<length> / min(x)`. Regression for
    // a format/reparse round-trip the property test surfaced.
    let query =
        parse("points | where avg(1 deg / min(heading)) > 0").expect("division by a call parses");
    let printed = query.to_string();
    assert_eq!(
        parse(&printed).expect("re-parses").to_string(),
        printed,
        "the canonical form round-trips"
    );
}

#[test]
fn negative_thresholds_parse_and_check() {
    let provider = TestProvider::new(3)
        .with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(5.0), Some(1.0)],
        )
        .indexed_time();
    let output = test_util::run_one("points | where accel < -2 m/s2", &provider);
    assert_eq!(output.matches[0].ranges, vec![1..3]);
}

/// `==`/`!=` accept a discrete count (`sats_fix == 6`) but not a continuous
/// quantity (`velocity == 30 km/h`), which would be a float-equality trap.
#[rstest]
#[case("points | where sats_fix == 6", true)]
#[case("points | where velocity == 30 km/h", false)]
fn equality_is_allowed_only_on_counts(#[case] src: &str, #[case] accepted: bool) {
    assert_eq!(
        test_util::chk(&parse(src).unwrap()).is_ok(),
        accepted,
        "for {src}"
    );
}

#[test]
fn long_arithmetic_chain_checks_without_panicking() {
    // A long `*` chain folds many exponent additions in the checker. The
    // dimension arithmetic saturates at the `i8` bounds. The exotic dimension
    // has no matching literal, so the checker rejects the query at the
    // comparison.
    let chain = std::iter::repeat_n("velocity", 200)
        .collect::<Vec<_>>()
        .join(" * ");
    let src = format!("points | where {chain} > 0");
    let result = test_util::chk(&parse(&src).expect("a long product chain parses"));
    assert!(
        result.is_err(),
        "an exotic dimension cannot compare to a bare number"
    );
}
