//! The GOES flare classification, and the radio blackout levels read off it.
//!
//! A flare is classified by its peak soft X-ray flux: a letter for the decade
//! and a magnitude within it, so `M1.8` is 1.8e-5 W/m². Each letter is ten
//! times the one before, and the X class has no ceiling.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// Peak flux, in W/m², where each class begins.
const A_CLASS_FLUX: f64 = 1e-8;
const B_CLASS_FLUX: f64 = 1e-7;
const C_CLASS_FLUX: f64 = 1e-6;
const M_CLASS_FLUX: f64 = 1e-5;
const X_CLASS_FLUX: f64 = 1e-4;

/// Peak flux, in W/m², where each NOAA radio blackout level begins.
const R1_FLUX: f64 = 1e-5;
const R2_FLUX: f64 = 5e-5;
const R3_FLUX: f64 = 1e-4;
const R4_FLUX: f64 = 1e-3;
const R5_FLUX: f64 = 2e-3;

/// The decade of a flare's peak soft X-ray flux, as the catalog letters it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    strum::Display,
    strum::EnumCount,
    strum::EnumIter,
)]
pub enum FlareClass {
    A,
    B,
    C,
    M,
    X,
}

impl FlareClass {
    /// The class this letter names, or [`None`] for a letter no class uses.
    pub const fn from_letter(letter: char) -> Option<Self> {
        match letter {
            'A' => Some(Self::A),
            'B' => Some(Self::B),
            'C' => Some(Self::C),
            'M' => Some(Self::M),
            'X' => Some(Self::X),
            _ => None,
        }
    }

    /// Peak flux, in W/m², of magnitude 1 in this class.
    pub const fn lowest_flux_watts_per_square_meter(self) -> f64 {
        match self {
            Self::A => A_CLASS_FLUX,
            Self::B => B_CLASS_FLUX,
            Self::C => C_CLASS_FLUX,
            Self::M => M_CLASS_FLUX,
            Self::X => X_CLASS_FLUX,
        }
    }
}

/// Lowest magnitude a class is published with. Below it the flux belongs to
/// the class below.
const MIN_MAGNITUDE: f64 = 1.0;

/// One flare's classification, as the catalog writes it: `M1.8`.
///
/// Constructed through [`FromStr`], so a classification no letter or
/// magnitude backs cannot reach a surface. Ordered by peak flux, which puts
/// an X1.8 above an M9.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlareClassification {
    class: FlareClass,
    magnitude: f64,
}

impl FlareClassification {
    /// The classification of `magnitude` within `class`, or [`None`] when the
    /// magnitude is below the class floor of 1 or is not a finite number.
    pub fn new(class: FlareClass, magnitude: f64) -> Option<Self> {
        (magnitude.is_finite() && magnitude >= MIN_MAGNITUDE).then_some(Self { class, magnitude })
    }

    pub const fn class(self) -> FlareClass {
        self.class
    }

    pub const fn magnitude(self) -> f64 {
        self.magnitude
    }

    /// Peak soft X-ray flux, in W/m².
    pub fn peak_flux_watts_per_square_meter(self) -> f64 {
        self.class.lowest_flux_watts_per_square_meter() * self.magnitude
    }

    /// Where the flare sits on the NOAA radio blackout scale, which starts at
    /// M1 and has no level for the classes below it.
    pub fn radio_blackout_class(self) -> Option<RadioBlackoutClass> {
        let flux = self.peak_flux_watts_per_square_meter();
        match flux {
            _ if flux >= R5_FLUX => Some(RadioBlackoutClass::Extreme),
            _ if flux >= R4_FLUX => Some(RadioBlackoutClass::Severe),
            _ if flux >= R3_FLUX => Some(RadioBlackoutClass::Strong),
            _ if flux >= R2_FLUX => Some(RadioBlackoutClass::Moderate),
            _ if flux >= R1_FLUX => Some(RadioBlackoutClass::Minor),
            _ => None,
        }
    }
}

/// A finite magnitude at or above the class floor, so every two
/// classifications compare.
impl Eq for FlareClassification {}

impl Ord for FlareClassification {
    fn cmp(&self, other: &Self) -> Ordering {
        self.class
            .cmp(&other.class)
            .then_with(|| self.magnitude.total_cmp(&other.magnitude))
    }
}

impl PartialOrd for FlareClassification {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Written as the catalog publishes it, to one decimal: `M1.8`, `C5.0`.
impl fmt::Display for FlareClassification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{:.1}", self.class, self.magnitude)
    }
}

/// Why a `classType` could not be read as a classification.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ClassificationParseError {
    #[error("{class:?} names no flare class, which is one of A, B, C, M or X")]
    UnknownClass { class: String },

    #[error("{magnitude:?} is not a magnitude: {detail}")]
    Magnitude { magnitude: String, detail: String },

    #[error(
        "magnitude {magnitude} is outside what a class publishes, which starts at {MIN_MAGNITUDE}"
    )]
    MagnitudeOutsideClass { magnitude: f64 },
}

impl FromStr for FlareClassification {
    type Err = ClassificationParseError;

    fn from_str(class_type: &str) -> Result<Self, Self::Err> {
        let mut characters = class_type.chars();
        let class = characters
            .next()
            .and_then(FlareClass::from_letter)
            .ok_or_else(|| ClassificationParseError::UnknownClass {
                class: class_type.to_owned(),
            })?;
        let magnitude = characters.as_str();
        let magnitude: f64 = magnitude
            .parse()
            .map_err(
                |err: std::num::ParseFloatError| ClassificationParseError::Magnitude {
                    magnitude: magnitude.to_owned(),
                    detail: err.to_string(),
                },
            )?;
        Self::new(class, magnitude)
            .ok_or(ClassificationParseError::MagnitudeOutsideClass { magnitude })
    }
}

/// A NOAA radio blackout level, R1 to R5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::EnumCount, strum::EnumIter)]
pub enum RadioBlackoutClass {
    Minor,
    Moderate,
    Strong,
    Severe,
    Extreme,
}

impl RadioBlackoutClass {
    /// The R-scale designation on its own, for a compact chip or legend.
    pub const fn scale_name(self) -> &'static str {
        match self {
            Self::Minor => "R1",
            Self::Moderate => "R2",
            Self::Strong => "R3",
            Self::Severe => "R4",
            Self::Extreme => "R5",
        }
    }

    /// Canonical human-readable name shown in the UI.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Minor => "R1 minor radio blackout",
            Self::Moderate => "R2 moderate radio blackout",
            Self::Strong => "R3 strong radio blackout",
            Self::Severe => "R4 severe radio blackout",
            Self::Extreme => "R5 extreme radio blackout",
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    fn classification(class_type: &str) -> FlareClassification {
        class_type.parse().expect("a published classification")
    }

    #[rstest]
    #[case::the_may_2024_peak("X2.2", FlareClass::X, 2.2)]
    #[case::mid_m("M1.8", FlareClass::M, 1.8)]
    #[case::a_whole_magnitude("C5.0", FlareClass::C, 5.0)]
    #[case::without_a_decimal("B7", FlareClass::B, 7.0)]
    #[case::the_weakest_class("A1.0", FlareClass::A, 1.0)]
    #[case::past_the_x_decade("X28.0", FlareClass::X, 28.0)]
    fn a_published_class_reads_as_its_letter_and_magnitude(
        #[case] class_type: &str,
        #[case] class: FlareClass,
        #[case] magnitude: f64,
    ) {
        assert_eq!(
            Some(classification(class_type)),
            FlareClassification::new(class, magnitude)
        );
    }

    #[rstest]
    #[case::unknown_letter(
        "Z1.0",
        "\"Z1.0\" names no flare class, which is one of A, B, C, M or X"
    )]
    #[case::empty("", "\"\" names no flare class, which is one of A, B, C, M or X")]
    #[case::lowercase(
        "m1.8",
        "\"m1.8\" names no flare class, which is one of A, B, C, M or X"
    )]
    #[case::worded_magnitude("Mstrong", "\"strong\" is not a magnitude: invalid float literal")]
    #[case::no_magnitude("M", "\"\" is not a magnitude: cannot parse float from empty string")]
    #[case::below_the_class_floor(
        "M0.5",
        "magnitude 0.5 is outside what a class publishes, which starts at 1"
    )]
    #[case::not_a_number(
        "MNaN",
        "magnitude NaN is outside what a class publishes, which starts at 1"
    )]
    fn a_class_the_scale_does_not_define_names_what_is_wrong(
        #[case] class_type: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(
            class_type
                .parse::<FlareClassification>()
                .expect_err("rejected")
                .to_string(),
            expected
        );
    }

    #[test]
    fn a_classification_prints_as_the_catalog_publishes_it() {
        assert_eq!(classification("M1.8").to_string(), "M1.8");
        assert_eq!(classification("B7").to_string(), "B7.0");
    }

    #[rstest]
    #[case::a1("A1.0", 1e-8)]
    #[case::c5("C5.0", 5e-6)]
    #[case::m1("M1.0", 1e-5)]
    #[case::x1("X1.0", 1e-4)]
    #[case::x10("X10.0", 1e-3)]
    fn the_peak_flux_steps_a_decade_per_class(
        #[case] class_type: &str,
        #[case] expected_watts_per_square_meter: f64,
    ) {
        let flux = classification(class_type).peak_flux_watts_per_square_meter();
        assert!(
            (flux - expected_watts_per_square_meter).abs() < expected_watts_per_square_meter * 1e-9,
            "{class_type} is {flux} W/m²"
        );
    }

    /// The class outranks the magnitude, which is the whole point of ordering
    /// on flux rather than on the number.
    #[test]
    fn a_weak_x_outranks_the_strongest_m() {
        assert!(classification("X1.0") > classification("M9.9"));
        assert!(classification("M1.8") > classification("M1.7"));
        assert_eq!(classification("M1.8"), classification("M1.8"));
    }

    #[rstest]
    #[case::below_the_scale("C9.9", None)]
    #[case::r1_floor("M1.0", Some(RadioBlackoutClass::Minor))]
    #[case::r2_floor("M5.0", Some(RadioBlackoutClass::Moderate))]
    #[case::r3_floor("X1.0", Some(RadioBlackoutClass::Strong))]
    #[case::r4_floor("X10.0", Some(RadioBlackoutClass::Severe))]
    #[case::r5_floor("X20.0", Some(RadioBlackoutClass::Extreme))]
    #[case::the_may_2024_peak("X2.2", Some(RadioBlackoutClass::Strong))]
    fn a_classification_sits_on_the_noaa_blackout_scale(
        #[case] class_type: &str,
        #[case] expected: Option<RadioBlackoutClass>,
    ) {
        assert_eq!(classification(class_type).radio_blackout_class(), expected);
    }

    /// Every level is reachable from a published class, and each names its own
    /// R number.
    #[test]
    fn every_blackout_level_is_reachable_and_named() {
        let reached: Vec<RadioBlackoutClass> = ["M1.0", "M5.0", "X1.0", "X10.0", "X20.0"]
            .into_iter()
            .filter_map(|class_type| classification(class_type).radio_blackout_class())
            .collect();
        assert_eq!(reached, RadioBlackoutClass::iter().collect::<Vec<_>>());
        assert_eq!(reached.len(), RadioBlackoutClass::COUNT);
        for level in RadioBlackoutClass::iter() {
            assert!(level.display_name().starts_with(level.scale_name()));
        }
    }

    #[test]
    fn every_class_letter_reads_back_as_itself() {
        for class in FlareClass::iter() {
            let letter = class.to_string();
            assert_eq!(
                letter.chars().next().and_then(FlareClass::from_letter),
                Some(class)
            );
        }
        assert_eq!(FlareClass::COUNT, 5);
    }
}
