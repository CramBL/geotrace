//! The period an angular value repeats at, and the reductions that respect it.
//!
//! A heading and a longitude repeat every 360°. A `.gtd` channel declares its
//! own period. `spread`, `std` and `delta` reduce against that period.

use std::f64::consts::TAU;

use nalgebra::{Complex, UnitComplex};

const FULL_TURN_DEGREES: f64 = 360.0;

/// The period an angular value repeats at, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WrapPeriod(f64);

impl WrapPeriod {
    /// The period of a compass quantity: a heading, a bearing, a longitude.
    pub(crate) const FULL_TURN: WrapPeriod = WrapPeriod(FULL_TURN_DEGREES);

    /// `None` unless `degrees` is finite and positive: anything else declares
    /// no wrap.
    pub(crate) fn from_degrees(degrees: f64) -> Option<WrapPeriod> {
        (degrees.is_finite() && degrees > 0.0).then_some(WrapPeriod(degrees))
    }

    /// Radians per unit of the values this period wraps.
    fn radians_per_unit(self) -> f64 {
        TAU / self.0
    }

    /// Signed shortest difference from the first value to the last,
    /// approximately in `(-period / 2, period / 2]`.
    ///
    /// Expressed as the rotation carrying the first onto the last, which
    /// handles the wrap. At the exact antipode the sign is
    /// implementation-defined: the turn is equally short either way, and
    /// `angle()` resolves it by floating-point rounding.
    pub(crate) fn delta(self, values: &[f64]) -> f64 {
        let (Some(first), Some(last)) = (values.first().copied(), values.last().copied()) else {
            return 0.0;
        };
        let scale = self.radians_per_unit();
        let rotation = UnitComplex::new(last * scale) * UnitComplex::new(first * scale).inverse();
        rotation.angle() / scale
    }

    /// Size of the smallest arc containing every value: the period minus the
    /// largest gap between neighbouring values on the circle.
    pub(crate) fn spread(self, values: &mut [f64]) -> f64 {
        for value in values.iter_mut() {
            *value = value.rem_euclid(self.0);
        }
        values.sort_unstable_by(f64::total_cmp);
        let (Some(first), Some(last)) = (values.first().copied(), values.last().copied()) else {
            return 0.0;
        };
        let wrap_gap = first + self.0 - last;
        let max_gap = values
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some(b - a),
                _ => None,
            })
            .fold(wrap_gap, f64::max);
        self.0 - max_gap
    }

    /// Circular (population) standard deviation of `values`, in their own
    /// units.
    ///
    /// Built from the mean resultant length R of the values as unit vectors:
    /// `sqrt(-2 ln R)` (Mardia), which holds across the wrap where a linear
    /// standard deviation does not. Identical values give 0. As they spread
    /// toward uniform, R falls to 0 and the deviation grows without bound.
    /// At the R = 0 singularity the value is non-finite, which the evaluator
    /// turns into a reported skip.
    pub(crate) fn std(self, values: &[f64]) -> f64 {
        let scale = self.radians_per_unit();
        let n = values.len() as f64;
        let resultant = values.iter().fold(Complex::new(0.0, 0.0), |acc, value| {
            acc + UnitComplex::new(value * scale).into_inner()
        });
        // Clamp guards a floating-point overshoot above 1 when every value is
        // identical, which would make the log positive and the `sqrt` NaN.
        let mean_resultant = (resultant.norm() / n).min(1.0);
        (-2.0 * mean_resultant.ln()).sqrt() / scale
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(0.0, "zero is no period")]
    #[case(-90.0, "a negative period is no period")]
    #[case(f64::NAN, "NaN is no period")]
    #[case(f64::INFINITY, "an infinite period is no period")]
    fn a_period_that_is_not_finite_and_positive_is_rejected(
        #[case] degrees: f64,
        #[case] reason: &str,
    ) {
        assert_eq!(WrapPeriod::from_degrees(degrees), None, "{reason}");
    }

    #[test]
    fn delta_takes_the_short_way() {
        let full_turn = WrapPeriod::FULL_TURN;
        assert!((full_turn.delta(&[350.0, 0.0, 10.0]) - 20.0).abs() < 1e-12);
        assert!((full_turn.delta(&[10.0, 0.0, 350.0]) + 20.0).abs() < 1e-12);
        assert!((full_turn.delta(&[0.0, 180.0]) - 180.0).abs() < 1e-12);
    }

    /// On a period of 180°, 170 and 10 are 20 apart the short way.
    #[test]
    fn delta_takes_the_short_way_on_a_half_turn() {
        let half_turn = WrapPeriod::from_degrees(180.0).expect("a positive period");
        assert!((half_turn.delta(&[170.0, 10.0]) - 20.0).abs() < 1e-12);
    }

    #[test]
    fn spread_measures_across_the_wrap() {
        let full_turn = WrapPeriod::FULL_TURN;
        let mut wrapped = vec![350.0, 10.0, 0.0];
        assert!((full_turn.spread(&mut wrapped) - 20.0).abs() < 1e-12);
        let mut plain = vec![10.0, 40.0];
        assert!((full_turn.spread(&mut plain) - 30.0).abs() < 1e-12);
        let mut single = vec![123.0];
        assert!((full_turn.spread(&mut single)).abs() < 1e-12);
    }

    /// On a period of 180°, 179 and 1 are two apart across the wrap.
    #[test]
    fn spread_measures_across_the_wrap_of_a_half_turn() {
        let half_turn = WrapPeriod::from_degrees(180.0).expect("a positive period");
        let mut values = vec![179.0, 1.0];
        assert!((half_turn.spread(&mut values) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn std_stays_small_across_the_wrap() {
        let full_turn = WrapPeriod::FULL_TURN;
        // Headings clustered around north stay small.
        assert!(full_turn.std(&[359.0, 0.0, 1.0]) < 2.0);
        // Identical directions collapse to zero (within float noise).
        assert!(full_turn.std(&[123.0, 123.0, 123.0]) < 1e-6);
        assert!(full_turn.std(&[42.0]) < 1e-6);
        // A wide scatter is a large deviation.
        assert!(full_turn.std(&[0.0, 90.0, 180.0]) > 45.0);
    }

    /// The reduction runs against the declared period. Doubling the period
    /// along with the values doubles the deviation. 179 and 358 sit across the
    /// wrap of their respective periods.
    #[test]
    fn std_reduces_against_the_declared_period() {
        let half_turn = WrapPeriod::from_degrees(180.0).expect("a positive period");
        let on_a_half_turn = half_turn.std(&[1.0, 3.0, 179.0]);
        let on_a_full_turn = WrapPeriod::FULL_TURN.std(&[2.0, 6.0, 358.0]);
        assert!((on_a_full_turn - 2.0 * on_a_half_turn).abs() < 1e-12);
    }

    mod properties {
        use proptest::prelude::*;

        use super::super::WrapPeriod;

        proptest! {
            /// Circular std is invariant under a rigid rotation of all the
            /// values, including across the wrap - the property that justifies
            /// the whole helper. Tested on tight clusters (within a twelfth of
            /// the period) where the statistic is well conditioned, so the
            /// invariant holds to a fine tolerance.
            #[test]
            fn std_is_rotation_invariant(
                period_deg in 1.0f64..720.0,
                fractions in proptest::collection::vec(-1.0f64 / 12.0..1.0 / 12.0, 1..20),
                center in 0.0f64..1.0,
                offset in 0.0f64..1.0,
            ) {
                let period = WrapPeriod::from_degrees(period_deg).expect("a positive period");
                let cluster = |base: f64| -> Vec<f64> {
                    fractions
                        .iter()
                        .map(|f| ((base + f) * period_deg).rem_euclid(period_deg))
                        .collect()
                };
                let here = period.std(&cluster(center));
                let there = period.std(&cluster(center + offset));
                prop_assert!(here.is_finite() && here >= 0.0);
                prop_assert!(
                    (here - there).abs() < 1e-6 * period_deg,
                    "here {here} there {there}"
                );
            }

            /// The R clamp keeps the statistic real over arbitrary values:
            /// never NaN, never negative.
            #[test]
            fn std_is_never_nan(
                period_deg in 1.0f64..720.0,
                fractions in proptest::collection::vec(0.0f64..1.0, 1..50),
            ) {
                let period = WrapPeriod::from_degrees(period_deg).expect("a positive period");
                let values: Vec<f64> = fractions.iter().map(|f| f * period_deg).collect();
                let std = period.std(&values);
                prop_assert!(!std.is_nan() && std >= 0.0);
            }
        }
    }
}
