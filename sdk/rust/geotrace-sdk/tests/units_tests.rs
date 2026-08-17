use geotrace_sdk::Angle;
use proptest::prelude::*;

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
