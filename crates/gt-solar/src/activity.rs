//! The Kp scale, and the storm classes read off it.
//!
//! Kp and Hp30 share one scale, so one value type serves both: quasi
//! logarithmic, published in thirds of a unit (2.667, 3.0, 3.333). Kp is
//! defined up to 9, while Hp30 keeps climbing past it during an extreme
//! storm.

use std::fmt;

use strum::IntoEnumIterator as _;

use crate::GeomagneticIndex;

/// Lowest value the service publishes for either index.
pub const MIN_VALUE: f64 = 0.0;

/// Highest value Kp is defined for. Hp30 has no ceiling.
pub const KP_MAX_VALUE: f64 = 9.0;

const UNSETTLED_LOWEST_VALUE: f64 = 3.0;
const ACTIVE_LOWEST_VALUE: f64 = 4.0;

/// One index value on the Kp scale.
///
/// Constructed through [`from_published_value`](Self::from_published_value),
/// so a value outside what its index publishes cannot reach a surface.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct GeomagneticActivity(f64);

impl GeomagneticActivity {
    /// The value as `index` publishes it, or [`None`] when it falls outside
    /// that index's range.
    pub fn from_published_value(index: GeomagneticIndex, value: f64) -> Option<Self> {
        let published = value.is_finite()
            && value >= MIN_VALUE
            && match index {
                GeomagneticIndex::Kp => value <= KP_MAX_VALUE,
                GeomagneticIndex::Hp30 => true,
            };
        published.then_some(Self(value))
    }

    pub const fn value(self) -> f64 {
        self.0
    }

    /// Where the value sits on the NOAA G-scale, which counts Kp 5 as the
    /// first storm level and Kp 9 as the last.
    pub fn class(self) -> GeomagneticActivityClass {
        let storm = GeomagneticStormClass::iter()
            .rev()
            .find(|storm| self.0 >= storm.lowest_value());
        match storm {
            Some(storm) => GeomagneticActivityClass::Storm(storm),
            None if self.0 >= ACTIVE_LOWEST_VALUE => GeomagneticActivityClass::Active,
            None if self.0 >= UNSETTLED_LOWEST_VALUE => GeomagneticActivityClass::Unsettled,
            None => GeomagneticActivityClass::Quiet,
        }
    }

    /// The storm class, for the values that have one.
    pub fn storm_class(self) -> Option<GeomagneticStormClass> {
        match self.class() {
            GeomagneticActivityClass::Storm(storm) => Some(storm),
            GeomagneticActivityClass::Quiet
            | GeomagneticActivityClass::Unsettled
            | GeomagneticActivityClass::Active => None,
        }
    }
}

/// Written as the service publishes it: `2.667`, `9`, `11.333`.
impl fmt::Display for GeomagneticActivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A NOAA geomagnetic storm level, G1 to G5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::EnumCount, strum::EnumIter)]
pub enum GeomagneticStormClass {
    Minor,
    Moderate,
    Strong,
    Severe,
    Extreme,
}

impl GeomagneticStormClass {
    /// Lowest index value that reaches this storm level, on the Kp scale both
    /// indices are published on.
    pub const fn lowest_value(self) -> f64 {
        match self {
            Self::Minor => 5.0,
            Self::Moderate => 6.0,
            Self::Strong => 7.0,
            Self::Severe => 8.0,
            Self::Extreme => 9.0,
        }
    }

    /// The G-scale designation on its own, for a compact chip or legend.
    pub const fn scale_name(self) -> &'static str {
        match self {
            Self::Minor => "G1",
            Self::Moderate => "G2",
            Self::Strong => "G3",
            Self::Severe => "G4",
            Self::Extreme => "G5",
        }
    }

    /// Canonical human-readable name shown in the UI.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Minor => "G1 minor storm",
            Self::Moderate => "G2 moderate storm",
            Self::Strong => "G3 strong storm",
            Self::Severe => "G4 severe storm",
            Self::Extreme => "G5 extreme storm",
        }
    }
}

/// What one value says about the field: a storm level, or one of the three
/// levels below the G-scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount)]
pub enum GeomagneticActivityClass {
    Quiet,
    Unsettled,
    Active,
    Storm(GeomagneticStormClass),
}

impl GeomagneticActivityClass {
    /// Canonical human-readable name shown in the UI.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Quiet => "Quiet",
            Self::Unsettled => "Unsettled",
            Self::Active => "Active",
            Self::Storm(storm) => storm.display_name(),
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    fn kp(value: f64) -> GeomagneticActivity {
        GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, value).unwrap()
    }

    #[rstest]
    #[case::floor(0.0, GeomagneticActivityClass::Quiet)]
    #[case::quiet_top(2.667, GeomagneticActivityClass::Quiet)]
    #[case::unsettled_floor(3.0, GeomagneticActivityClass::Unsettled)]
    #[case::active_floor(4.0, GeomagneticActivityClass::Active)]
    #[case::below_the_first_storm(4.667, GeomagneticActivityClass::Active)]
    #[case::g1(5.0, GeomagneticActivityClass::Storm(GeomagneticStormClass::Minor))]
    #[case::g2(
        6.333,
        GeomagneticActivityClass::Storm(GeomagneticStormClass::Moderate)
    )]
    #[case::g3(7.0, GeomagneticActivityClass::Storm(GeomagneticStormClass::Strong))]
    #[case::g4(8.667, GeomagneticActivityClass::Storm(GeomagneticStormClass::Severe))]
    #[case::g5(9.0, GeomagneticActivityClass::Storm(GeomagneticStormClass::Extreme))]
    fn a_value_classifies_on_the_g_scale(
        #[case] value: f64,
        #[case] expected: GeomagneticActivityClass,
    ) {
        assert_eq!(kp(value).class(), expected);
    }

    /// Hp30 climbs past Kp's ceiling in an extreme storm, and stays G5.
    #[test]
    fn an_hp30_value_above_nine_is_still_the_top_class() {
        let activity =
            GeomagneticActivity::from_published_value(GeomagneticIndex::Hp30, 11.333).unwrap();
        assert_eq!(activity.storm_class(), Some(GeomagneticStormClass::Extreme));
        assert_eq!(activity.to_string(), "11.333");
    }

    #[rstest]
    #[case::above_the_kp_ceiling(GeomagneticIndex::Kp, 9.333)]
    #[case::negative_kp(GeomagneticIndex::Kp, -0.333)]
    #[case::negative_hp30(GeomagneticIndex::Hp30, -1.0)]
    #[case::not_a_number(GeomagneticIndex::Kp, f64::NAN)]
    #[case::infinite(GeomagneticIndex::Hp30, f64::INFINITY)]
    fn a_value_outside_the_published_range_is_refused(
        #[case] index: GeomagneticIndex,
        #[case] value: f64,
    ) {
        assert_eq!(
            GeomagneticActivity::from_published_value(index, value),
            None
        );
    }

    #[test]
    fn hp30_accepts_what_kp_refuses() {
        let value = 11.333;
        assert!(GeomagneticActivity::from_published_value(GeomagneticIndex::Hp30, value).is_some());
        assert_eq!(
            GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, value),
            None
        );
    }

    #[test]
    fn a_value_below_the_g_scale_has_no_storm_class() {
        assert_eq!(kp(4.667).storm_class(), None);
    }

    /// Every storm level is reachable from its own lowest value, and each
    /// names its own G number.
    #[test]
    fn every_storm_class_is_reachable_and_named() {
        let storms: Vec<GeomagneticStormClass> = GeomagneticStormClass::iter().collect();
        assert_eq!(storms.len(), GeomagneticStormClass::COUNT);
        for storm in storms {
            assert_eq!(
                GeomagneticActivity::from_published_value(
                    GeomagneticIndex::Hp30,
                    storm.lowest_value()
                )
                .and_then(GeomagneticActivity::storm_class),
                Some(storm)
            );
            assert!(storm.display_name().starts_with(storm.scale_name()));
        }
    }

    #[test]
    fn a_value_prints_as_the_service_publishes_it() {
        assert_eq!(kp(2.667).to_string(), "2.667");
        assert_eq!(kp(9.0).to_string(), "9");
    }
}
