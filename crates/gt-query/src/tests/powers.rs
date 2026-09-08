use super::*;

/// The exponent is a whole number in `i8` range. A fractional or oversized
/// power is rejected while parsing.
#[rstest]
#[case("points | where velocity^2.5 > 0", "a power must be a whole number")]
#[case(
    "points | where velocity^999 > 0",
    "a power must be a whole number between -128 and 127"
)]
#[case(
    "points | where velocity⁹⁹⁹ > 0",
    "a power must be a whole number between -128 and 127"
)]
#[case("points | where velocity⁻ > 0", "a power must be a whole number")]
fn power_rejects_non_integer_and_out_of_range(#[case] src: &str, #[case] expected: &str) {
    assert_eq!(parse(src).expect_err(src).message, expected, "for {src}");
}

/// A power scales the base's dimension: `velocity²` is a squared speed
/// (comparable only to another squared speed), while any power of a
/// dimensionless value is a bare number.
#[rstest]
#[case("points | where sats_fix² < 100", None)]
#[case("points | where sats_fix⁻¹ < 0.5", None)]
#[case("points | where velocity² > velocity²", None)]
#[case(
    "points | where velocity² > 30 km/h",
    Some("cannot compare speed² with speed")
)]
fn power_scales_the_dimension(#[case] src: &str, #[case] error: Option<&str>) {
    match error {
        None => {
            test_util::chk(&parse(src).expect(src)).expect(src);
        }
        Some(message) => {
            assert_eq!(
                test_util::chk(&parse(src).unwrap()).unwrap_err().message,
                message,
                "for {src}"
            );
        }
    }
}

#[test]
fn power_squares_a_point_value() {
    // `sats_fix` squared: 3² = 9 < 16 matches, 5² = 25 does not.
    let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(3.0), Some(5.0)]);
    let output = test_util::run_one("points | where sats_fix² < 16", &provider);
    assert_eq!(output.matches[0].ranges, vec![0..1]);
}

#[test]
fn power_with_a_negative_exponent_inverts() {
    // sats_fix⁻¹: 1/2 = 0.5 > 0.4 matches, 1/4 = 0.25 does not.
    let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(2.0), Some(4.0)]);
    let output = test_util::run_one("points | where sats_fix⁻¹ > 0.4", &provider);
    assert_eq!(output.matches[0].ranges, vec![0..1]);
}

#[test]
fn power_with_a_zero_exponent_is_one() {
    // Every value to the zeroth power is 1, so all points clear the bar.
    let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(3.0), Some(7.0)]);
    let output = test_util::run_one("points | where sats_fix⁰ > 0.5", &provider);
    assert_eq!(output.matches[0].ranges, vec![0..2]);
}

#[test]
fn a_negative_power_of_zero_poisons_the_point() {
    // 0⁻¹ is infinite, so that point is skipped like any undefined
    // arithmetic. The finite inverse still matches.
    let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(0.0), Some(2.0)]);
    let output = test_util::run_one("points | where sats_fix⁻¹ < 1", &provider);
    assert_eq!(output.matches[0].ranges, vec![1..2]);
    assert_eq!(output.summary.skipped_non_finite, 1);
}
