use super::*;

/// `sqrt` halves the dimension, so its argument must be a perfect square (or
/// dimensionless): `sqrt(velocity²)` is a speed, and squaring components
/// then rooting the sum is a magnitude. A non-square is a pointed error.
#[rstest]
#[case("points | where sqrt(velocity²) > 30 km/h", None)]
#[case("points | where sqrt(lat² + lon²) > 0 deg", None)]
#[case("points | where sqrt(velocity² + velocity²) > 30 km/h", None)]
#[case("points | where sqrt(sats_fix) < 5", None)]
// `sqrt` nested inside an aggregate (it works in a window too).
#[case("points | window 3 | where avg(sqrt(velocity²)) > 30 km/h", None)]
// A squared ratio roots to a bare number, compared without a unit.
#[case("points | with mask 15 deg | where sqrt(util_gps) > 0.7", None)]
#[case(
    "points | with mask 15 deg | where sqrt(util_gps) > 50 %",
    Some("cannot compare number with ratio")
)]
#[case("points | where sqrt(velocity) > 0", Some("sqrt needs a square"))]
#[case(
    "points | where sqrt(time) > 5 s",
    Some("cannot take the square root of a timestamp")
)]
fn sqrt_needs_a_perfect_square(#[case] src: &str, #[case] error: Option<&str>) {
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
fn sqrt_on_a_non_square_suggests_squaring_first() {
    let err = test_util::chk(&parse("points | where sqrt(velocity) > 0").unwrap()).unwrap_err();
    assert_eq!(
        err.help.as_deref(),
        Some("square the values first, e.g. sqrt(x² + y²)")
    );
}

#[test]
fn a_squared_comparison_suggests_sqrt() {
    // The squared side has a matching root, so the fix is to take it.
    let err = test_util::chk(&parse("points | where velocity² > 30 km/h").unwrap()).unwrap_err();
    assert_eq!(err.message, "cannot compare speed² with speed");
    assert_eq!(err.help.as_deref(), Some("take its square root with sqrt"));
}

#[test]
fn sqrt_computes_a_magnitude() {
    // sqrt(lat² + lon²): sqrt(3² + 4²) = 5 clears 4.5, sqrt(0) does not.
    let provider = TestProvider::new(2)
        .with(QueryMetric::Lat, vec![Some(3.0), Some(0.0)])
        .with(QueryMetric::Lon, vec![Some(4.0), Some(0.0)]);
    let output = test_util::run_one("points | where sqrt(lat² + lon²) > 4.5 deg", &provider);
    assert_eq!(output.matches[0].ranges, vec![0..1]);
}

#[test]
fn sqrt_wraps_a_windowed_aggregate() {
    // sqrt(avg(velocity)²) over window 2: avg 15 m/s (54 km/h) misses the
    // 70 km/h bar, avg 25 m/s (90 km/h) clears it.
    let provider = TestProvider::new(3).with(
        QueryMetric::Velocity,
        vec![Some(10.0), Some(20.0), Some(30.0)],
    );
    let output = test_util::run_one(
        "points | window 2 | where sqrt(avg(velocity)²) > 70 km/h",
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![1..3]);
}

#[test]
fn sqrt_of_a_negative_poisons_the_point() {
    // sqrt(sats_fix - 10): 4 - 10 = -6 roots to NaN and is skipped. 20 - 10
    // roots to a finite value that matches.
    let provider = TestProvider::new(2).with(QueryMetric::SatsFix, vec![Some(4.0), Some(20.0)]);
    let output = test_util::run_one("points | where sqrt(sats_fix - 10) < 5", &provider);
    assert_eq!(output.matches[0].ranges, vec![1..2]);
    assert_eq!(output.summary.skipped_non_finite, 1);
}
