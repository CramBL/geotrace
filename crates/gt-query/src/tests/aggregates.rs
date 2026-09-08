use super::*;

#[test]
fn circular_spread_matches_across_north() {
    let provider = TestProvider::new(3).with(
        QueryMetric::Heading,
        vec![Some(350.0), Some(0.0), Some(10.0)],
    );
    let output = test_util::run_one(
        "points | window 3 | where spread(heading) <= 25 deg",
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

#[test]
fn std_over_a_window_uses_population_deviation() {
    // Steady speed has zero std. The last window jumps, so only the steady
    // stretch matches. 2 km/h is 0.556 m/s, well above the 0 of a flat run.
    let provider = TestProvider::new(4).with(
        QueryMetric::Velocity,
        vec![Some(10.0), Some(10.0), Some(10.0), Some(20.0)],
    );
    let output = test_util::run_one(
        "points | window 2 | where std(velocity) < 2 km/h",
        &provider,
    );
    // Windows [0,2) and [1,3) are flat. [2,4) has a 5 m/s std and fails.
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

#[test]
fn circular_std_flags_a_steady_heading() {
    // A heading tight around north stays under the threshold. The scattered
    // windows do not, so only the steady stretch matches.
    let provider = TestProvider::new(6).with(
        QueryMetric::Heading,
        vec![
            Some(359.0),
            Some(1.0),
            Some(0.0),
            Some(90.0),
            Some(200.0),
            Some(300.0),
        ],
    );
    let output = test_util::run_one("points | window 3 | where std(heading) <= 5 deg", &provider);
    // Only the first window [0,3) around north is steady.
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

/// `var` squares the argument's dimension: `var(velocity)` is a squared
/// speed with no matching literal, while `var(sats_fix)` is a plain number.
#[rstest]
#[case("points | window 3 | where var(sats_fix) < 4", None)]
// Two squared speeds share a dimension, so they compare.
#[case("points | window 3 | where var(velocity) > var(velocity)", None)]
// A squared ratio is a bare number. A squared timestamp is a squared
// duration. A squared angle is exotic. None has a matching literal.
#[case(
    "points | with mask 15 deg | window 3 | where var(util_all) < 0.1",
    None
)]
#[case(
    "points | window 3 | where var(velocity) > 30 km/h",
    Some("cannot compare speed² with speed")
)]
#[case(
    "points | window 3 | where var(velocity) > var(eph)",
    Some("cannot compare speed² with length²")
)]
#[case(
    "points | window 3 | where var(time) > 5 s",
    Some("cannot compare duration² with duration")
)]
#[case(
    "points | window 3 | where var(lat) > 3 deg",
    Some("cannot compare angle² with angle")
)]
fn var_squares_the_dimension(#[case] src: &str, #[case] error: Option<&str>) {
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
fn var_on_a_wrapping_angle_suggests_std() {
    let err = test_util::chk(&parse("points | window 3 | where var(heading) < 1 deg").unwrap())
        .unwrap_err();
    assert_eq!(err.message, "var is not defined for a wrapping angle");
    assert_eq!(
        err.help.as_deref(),
        Some("circular variance is unitless, not a squared angle - use std")
    );
}

#[test]
fn min_of_longitude_is_rejected_as_ambiguous() {
    let err =
        test_util::chk(&parse("points | window 3 | where min(lon) < 10 deg").unwrap()).unwrap_err();
    assert_eq!(err.message, "min on a wrapping angle is ambiguous");
    assert_eq!(
        err.help.as_deref(),
        Some("use spread, std, first, last, or delta")
    );
}

#[test]
fn var_matches_low_variance_windows() {
    // window 2 var(sats_fix) over [6,6,6,9]: windows [0,2) and [1,3) have
    // variance 0. Window [2,4) has variance 2.25, so only the steady points
    // match.
    let provider = TestProvider::new(4).with(
        QueryMetric::SatsFix,
        vec![Some(6.0), Some(6.0), Some(6.0), Some(9.0)],
    );
    let output = test_util::run_one("points | window 2 | where var(sats_fix) < 1", &provider);
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}
