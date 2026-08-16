//! Total electron content values and the L1 delay they cause.

use std::ops::RangeInclusive;

/// First-order ionospheric delay coefficient in m^3/s^2, from the standard
/// relation `delay = 40.3 / f^2 * TEC` (IONEX 1.0 specification, section 2).
const DELAY_COEFFICIENT_M3_PER_S2: f64 = 40.3;

/// Electrons per square meter in one TEC unit.
const ELECTRONS_PER_SQUARE_METER_PER_TECU: f64 = 1e16;

/// GPS L1 carrier frequency.
const L1_FREQUENCY_HZ: f64 = 1_575.42e6;

/// Range one TEC unit adds to an L1 pseudorange: 0.162 m.
pub const L1_DELAY_METERS_PER_TECU: f64 = DELAY_COEFFICIENT_M3_PER_S2
    * ELECTRONS_PER_SQUARE_METER_PER_TECU
    / (L1_FREQUENCY_HZ * L1_FREQUENCY_HZ);

/// A vertical total electron content value in TEC units, one of which is
/// 10^16 electrons per square meter.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct TotalElectronContent(f64);

impl TotalElectronContent {
    pub const fn from_tecu(tecu: f64) -> Self {
        Self(tecu)
    }

    pub const fn tecu(self) -> f64 {
        self.0
    }

    /// The extra range this value adds on L1, by
    /// [`L1_DELAY_METERS_PER_TECU`].
    pub fn l1_delay_meters(self) -> f64 {
        self.0 * L1_DELAY_METERS_PER_TECU
    }
}

/// Exponents accepted from a file, wide enough for every published product
/// and narrow enough to keep a scaled value finite.
const EXPONENT_RANGE: RangeInclusive<i32> = -10..=10;

/// The power of ten a file stores its TEC values scaled by, from its
/// `EXPONENT` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalingExponent(i32);

impl Default for ScalingExponent {
    /// The exponent IONEX defines for a file that declares none.
    fn default() -> Self {
        Self(-1)
    }
}

impl ScalingExponent {
    /// [`None`] for an exponent no published product writes.
    pub fn new(exponent: i32) -> Option<Self> {
        EXPONENT_RANGE.contains(&exponent).then_some(Self(exponent))
    }

    pub const fn value(self) -> i32 {
        self.0
    }

    /// The value a stored integer stands for.
    pub fn scale(self, stored: i32) -> TotalElectronContent {
        let stored = f64::from(stored);
        let magnitude = 10f64.powi(self.0.abs());
        TotalElectronContent(if self.0.is_negative() {
            stored / magnitude
        } else {
            stored * magnitude
        })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// The published first-order relation, to the three digits every GNSS
    /// text quotes it with.
    #[test]
    fn one_tec_unit_delays_l1_by_sixteen_centimeters() {
        assert!(
            (L1_DELAY_METERS_PER_TECU - 0.162).abs() < 5e-4,
            "{L1_DELAY_METERS_PER_TECU} m per TECU"
        );
        let delay = TotalElectronContent::from_tecu(50.0).l1_delay_meters();
        assert!((delay - 8.12).abs() < 5e-3, "{delay} m at 50 TECU");
    }

    #[rstest]
    #[case::the_ionex_default(-1, 263, 26.3)]
    #[case::two_decimals(-2, 2630, 26.3)]
    #[case::whole_units(0, 26, 26.0)]
    #[case::tens_of_units(1, 26, 260.0)]
    #[case::a_negative_value(-1, -5, -0.5)]
    fn a_stored_integer_scales_by_its_exponent(
        #[case] exponent: i32,
        #[case] stored: i32,
        #[case] expected: f64,
    ) {
        assert_eq!(
            ScalingExponent::new(exponent).map(|exponent| exponent.scale(stored)),
            Some(TotalElectronContent::from_tecu(expected))
        );
    }

    #[rstest]
    #[case::far_below_the_range(-11)]
    #[case::far_above_the_range(11)]
    #[case::the_smallest_integer(i32::MIN)]
    fn an_exponent_outside_the_published_range_is_refused(#[case] exponent: i32) {
        assert_eq!(ScalingExponent::new(exponent), None);
    }

    #[test]
    fn a_file_without_an_exponent_record_scales_by_a_tenth() {
        assert_eq!(
            ScalingExponent::default().scale(263),
            TotalElectronContent::from_tecu(26.3)
        );
        assert_eq!(ScalingExponent::default().value(), -1);
    }
}
