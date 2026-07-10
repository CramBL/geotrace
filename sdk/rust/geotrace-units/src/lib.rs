//! Canonical units used by GeoTrace channels and queries.
//!
//! [`ChannelUnit`] separates units GeoTrace understands from deliberate custom
//! labels. Recognized units carry dimensional and scaling information. A
//! [`CustomUnit`] is preserved and displayed verbatim, but its values are
//! dimensionless because GeoTrace cannot safely infer conversions for it.

use std::{fmt, str::FromStr};

use uom::si::acceleration::{meter_per_second_squared, standard_gravity};
use uom::si::f64::{Acceleration, Velocity};
use uom::si::velocity::{kilometer_per_hour, knot, meter_per_second};

const S_PER_MIN: f64 = 60.0;
const MIN_PER_H: f64 = 60.0;
const PERCENT: f64 = 100.0;
const PER_S_TO_PER_MIN: f64 = 60.0;
const MAX_CUSTOM_UNIT_BYTES: usize = 63;

/// The physical quantity represented by a recognized [`Unit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhysicalQuantity {
    Angle,
    Length,
    Speed,
    Acceleration,
    Duration,
    Ratio,
    Rate,
}

/// An SI prefix scaling a [`BaseUnit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumIter, strum::EnumCount)]
pub enum SiPrefix {
    Nano,
    Micro,
    Milli,
    Centi,
    Kilo,
}

impl SiPrefix {
    /// The scale applied to the base unit.
    pub fn factor(self) -> f64 {
        match self {
            Self::Nano => 1e-9,
            Self::Micro => 1e-6,
            Self::Milli => 1e-3,
            Self::Centi => 1e-2,
            Self::Kilo => 1e3,
        }
    }

    /// Canonical spelling before the base unit.
    pub fn text(self) -> &'static str {
        match self {
            Self::Nano => "n",
            Self::Micro => "u",
            Self::Milli => "m",
            Self::Centi => "c",
            Self::Kilo => "k",
        }
    }

    fn split(ident: &str) -> Option<(Self, &str)> {
        let micro = ident
            .strip_prefix('u')
            .or_else(|| ident.strip_prefix('µ'))
            .or_else(|| ident.strip_prefix('μ'))
            .map(|rest| (Self::Micro, rest));
        micro.or_else(|| {
            let (first, rest) = ident.split_at_checked(1)?;
            let prefix = match first {
                "n" => Self::Nano,
                "m" => Self::Milli,
                "c" => Self::Centi,
                "k" => Self::Kilo,
                _ => return None,
            };
            Some((prefix, rest))
        })
    }
}

/// An unprefixed recognized unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumIter, strum::EnumCount)]
pub enum BaseUnit {
    Deg,
    M,
    KmPerH,
    MPerS,
    Kn,
    MPerS2,
    G,
    KmPerHPerS,
    S,
    Min,
    H,
    Percent,
    PerS,
    PerMin,
    PerH,
}

impl BaseUnit {
    /// The physical quantity this unit measures.
    pub fn quantity(self) -> PhysicalQuantity {
        match self {
            Self::Deg => PhysicalQuantity::Angle,
            Self::M => PhysicalQuantity::Length,
            Self::KmPerH | Self::MPerS | Self::Kn => PhysicalQuantity::Speed,
            Self::MPerS2 | Self::G | Self::KmPerHPerS => PhysicalQuantity::Acceleration,
            Self::S | Self::Min | Self::H => PhysicalQuantity::Duration,
            Self::Percent => PhysicalQuantity::Ratio,
            Self::PerS | Self::PerMin | Self::PerH => PhysicalQuantity::Rate,
        }
    }

    /// Factor converting one value to the evaluator's base unit.
    pub fn to_base(self) -> f64 {
        match self {
            Self::Deg | Self::M | Self::MPerS | Self::MPerS2 | Self::S | Self::PerMin => 1.0,
            Self::KmPerH | Self::KmPerHPerS => {
                Velocity::new::<kilometer_per_hour>(1.0).get::<meter_per_second>()
            }
            Self::Kn => Velocity::new::<knot>(1.0).get::<meter_per_second>(),
            Self::G => Acceleration::new::<standard_gravity>(1.0).get::<meter_per_second_squared>(),
            Self::Min => S_PER_MIN,
            Self::H => S_PER_MIN * MIN_PER_H,
            Self::Percent => 1.0 / PERCENT,
            Self::PerS => PER_S_TO_PER_MIN,
            Self::PerH => 1.0 / MIN_PER_H,
        }
    }

    /// Canonical wire spelling.
    pub fn text(self) -> &'static str {
        match self {
            Self::Deg => "deg",
            Self::M => "m",
            Self::KmPerH => "km/h",
            Self::MPerS => "m/s",
            Self::Kn => "kn",
            Self::MPerS2 => "m/s2",
            Self::G => "g",
            Self::KmPerHPerS => "km/h/s",
            Self::S => "s",
            Self::Min => "min",
            Self::H => "h",
            Self::Percent => "%",
            Self::PerS => "per s",
            Self::PerMin => "per min",
            Self::PerH => "per h",
        }
    }

    /// Whether this base accepts the given prefix.
    pub fn accepts_prefix(self, prefix: SiPrefix) -> bool {
        match self {
            Self::M => true,
            Self::S => matches!(prefix, SiPrefix::Nano | SiPrefix::Micro | SiPrefix::Milli),
            Self::G => matches!(prefix, SiPrefix::Micro | SiPrefix::Milli),
            Self::MPerS | Self::MPerS2 => matches!(prefix, SiPrefix::Milli | SiPrefix::Centi),
            Self::Deg
            | Self::KmPerH
            | Self::Kn
            | Self::KmPerHPerS
            | Self::Min
            | Self::H
            | Self::Percent
            | Self::PerS
            | Self::PerMin
            | Self::PerH => false,
        }
    }

    fn from_ident(ident: &str) -> Option<Self> {
        match ident {
            "deg" => Some(Self::Deg),
            "m" => Some(Self::M),
            "kmh" => Some(Self::KmPerH),
            "kn" => Some(Self::Kn),
            "g" => Some(Self::G),
            "s" => Some(Self::S),
            "min" => Some(Self::Min),
            "h" => Some(Self::H),
            _ => None,
        }
    }
}

/// A recognized unit, optionally scaled by an SI prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unit {
    prefix: Option<SiPrefix>,
    base: BaseUnit,
}

impl Unit {
    pub const DEG: Self = Self::base(BaseUnit::Deg);
    pub const M: Self = Self::base(BaseUnit::M);
    pub const NM: Self = Self::prefixed(SiPrefix::Nano, BaseUnit::M);
    pub const UM: Self = Self::prefixed(SiPrefix::Micro, BaseUnit::M);
    pub const MM: Self = Self::prefixed(SiPrefix::Milli, BaseUnit::M);
    pub const CM: Self = Self::prefixed(SiPrefix::Centi, BaseUnit::M);
    pub const KM: Self = Self::prefixed(SiPrefix::Kilo, BaseUnit::M);
    pub const KM_PER_H: Self = Self::base(BaseUnit::KmPerH);
    pub const M_PER_S: Self = Self::base(BaseUnit::MPerS);
    pub const MM_PER_S: Self = Self::prefixed(SiPrefix::Milli, BaseUnit::MPerS);
    pub const CM_PER_S: Self = Self::prefixed(SiPrefix::Centi, BaseUnit::MPerS);
    pub const KN: Self = Self::base(BaseUnit::Kn);
    pub const M_PER_S2: Self = Self::base(BaseUnit::MPerS2);
    pub const MM_PER_S2: Self = Self::prefixed(SiPrefix::Milli, BaseUnit::MPerS2);
    pub const CM_PER_S2: Self = Self::prefixed(SiPrefix::Centi, BaseUnit::MPerS2);
    pub const G: Self = Self::base(BaseUnit::G);
    pub const UG: Self = Self::prefixed(SiPrefix::Micro, BaseUnit::G);
    pub const MG: Self = Self::prefixed(SiPrefix::Milli, BaseUnit::G);
    pub const KM_PER_H_PER_S: Self = Self::base(BaseUnit::KmPerHPerS);
    pub const NS: Self = Self::prefixed(SiPrefix::Nano, BaseUnit::S);
    pub const US: Self = Self::prefixed(SiPrefix::Micro, BaseUnit::S);
    pub const MS: Self = Self::prefixed(SiPrefix::Milli, BaseUnit::S);
    pub const S: Self = Self::base(BaseUnit::S);
    pub const MIN: Self = Self::base(BaseUnit::Min);
    pub const H: Self = Self::base(BaseUnit::H);
    pub const PERCENT: Self = Self::base(BaseUnit::Percent);
    pub const PER_S: Self = Self::base(BaseUnit::PerS);
    pub const PER_MIN: Self = Self::base(BaseUnit::PerMin);
    pub const PER_H: Self = Self::base(BaseUnit::PerH);

    /// Compact unit catalog suitable for query suggestions.
    pub const CANONICAL: [Self; 17] = [
        Self::DEG,
        Self::M,
        Self::KM,
        Self::KM_PER_H,
        Self::M_PER_S,
        Self::KN,
        Self::M_PER_S2,
        Self::G,
        Self::KM_PER_H_PER_S,
        Self::MS,
        Self::S,
        Self::MIN,
        Self::H,
        Self::PERCENT,
        Self::PER_S,
        Self::PER_MIN,
        Self::PER_H,
    ];

    /// Every unit accepted as recognized channel metadata.
    pub const RECOGNIZED: [Self; 29] = [
        Self::DEG,
        Self::M,
        Self::NM,
        Self::UM,
        Self::MM,
        Self::CM,
        Self::KM,
        Self::KM_PER_H,
        Self::M_PER_S,
        Self::MM_PER_S,
        Self::CM_PER_S,
        Self::KN,
        Self::M_PER_S2,
        Self::MM_PER_S2,
        Self::CM_PER_S2,
        Self::G,
        Self::UG,
        Self::MG,
        Self::KM_PER_H_PER_S,
        Self::NS,
        Self::US,
        Self::MS,
        Self::S,
        Self::MIN,
        Self::H,
        Self::PERCENT,
        Self::PER_S,
        Self::PER_MIN,
        Self::PER_H,
    ];

    const fn base(base: BaseUnit) -> Self {
        Self { prefix: None, base }
    }

    const fn prefixed(prefix: SiPrefix, base: BaseUnit) -> Self {
        Self {
            prefix: Some(prefix),
            base,
        }
    }

    pub fn quantity(self) -> PhysicalQuantity {
        self.base.quantity()
    }

    pub fn to_base(self) -> f64 {
        self.prefix.map_or(1.0, SiPrefix::factor) * self.base.to_base()
    }

    pub fn from_base(self) -> f64 {
        1.0 / self.to_base()
    }

    /// Canonical spelling as a lightweight display value.
    pub fn text(self) -> UnitText {
        UnitText(self)
    }

    pub fn canonical_text(self) -> Option<&'static str> {
        match (self.prefix, self.base) {
            (None, base) => Some(base.text()),
            (Some(SiPrefix::Kilo), BaseUnit::M) => Some("km"),
            (Some(SiPrefix::Milli), BaseUnit::S) => Some("ms"),
            _ => None,
        }
    }

    pub fn from_ident(ident: &str) -> Option<Self> {
        if let Some(base) = BaseUnit::from_ident(ident) {
            return Some(Self::base(base));
        }
        let (prefix, rest) = SiPrefix::split(ident)?;
        let base = BaseUnit::from_ident(rest)?;
        base.accepts_prefix(prefix)
            .then_some(Self::prefixed(prefix, base))
    }

    pub fn from_pair(first: &str, second: &str) -> Option<Self> {
        let exact = match (first, second) {
            ("km", "h") => Some(BaseUnit::KmPerH),
            ("m", "s") => Some(BaseUnit::MPerS),
            ("m", "s2") => Some(BaseUnit::MPerS2),
            _ => None,
        };
        if let Some(base) = exact {
            return Some(Self::base(base));
        }
        let (prefix, rest) = SiPrefix::split(first)?;
        let base = match (rest, second) {
            ("m", "s") => BaseUnit::MPerS,
            ("m", "s2") => BaseUnit::MPerS2,
            _ => return None,
        };
        base.accepts_prefix(prefix)
            .then_some(Self::prefixed(prefix, base))
    }

    pub fn from_triple(first: &str, second: &str, third: &str) -> Option<Self> {
        match (first, second, third) {
            ("km", "h", "s") => Some(Self::base(BaseUnit::KmPerHPerS)),
            _ => None,
        }
    }

    /// Parse a canonical channel label or a supported legacy spelling.
    pub fn from_label(label: &str) -> Option<Self> {
        let normalized = normalize_label(label);
        let exact = match normalized.as_str() {
            "%" => Some(Self::PERCENT),
            "per s" => Some(Self::PER_S),
            "per min" => Some(Self::PER_MIN),
            "per h" => Some(Self::PER_H),
            _ => None,
        };
        if exact.is_some() {
            return exact;
        }
        let parts: Vec<&str> = normalized.split('/').collect();
        match parts.as_slice() {
            [a] => Self::from_ident(a),
            [a, b] => Self::from_pair(a, b),
            [a, b, c] => Self::from_triple(a, b, c),
            _ => None,
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = self.prefix {
            write!(f, "{}", prefix.text())?;
            match self.base {
                BaseUnit::MPerS => return f.write_str("m/s"),
                BaseUnit::MPerS2 => return f.write_str("m/s2"),
                _ => {}
            }
        }
        f.write_str(self.base.text())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UnitText(Unit);

impl fmt::Display for UnitText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Unit {
    type Err = UnitParseError;

    fn from_str(label: &str) -> Result<Self, Self::Err> {
        Self::from_label(label).ok_or_else(|| UnitParseError::Unrecognized {
            label: label.to_owned(),
        })
    }
}

/// A deliberate display-only unit label GeoTrace does not understand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CustomUnit(String);

impl CustomUnit {
    pub fn new(label: impl Into<String>) -> Result<Self, UnitParseError> {
        let label = label.into();
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(UnitParseError::EmptyCustom);
        }
        if trimmed.len() > MAX_CUSTOM_UNIT_BYTES {
            return Err(UnitParseError::CustomTooLong {
                len: trimmed.len(),
                max: MAX_CUSTOM_UNIT_BYTES,
            });
        }
        if trimmed.chars().any(char::is_control) {
            return Err(UnitParseError::CustomControlCharacter);
        }
        if Unit::from_label(trimmed).is_some() {
            return Err(UnitParseError::CustomIsRecognized {
                label: trimmed.to_owned(),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CustomUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A recognized, convertible unit or an explicit display-only custom label.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ChannelUnit {
    Recognized(Unit),
    Custom(CustomUnit),
}

impl ChannelUnit {
    pub fn recognized(unit: Unit) -> Self {
        Self::Recognized(unit)
    }

    pub fn custom(label: impl Into<String>) -> Result<Self, UnitParseError> {
        CustomUnit::new(label).map(Self::Custom)
    }

    /// Parse existing file metadata as far as possible, preserving unsupported
    /// labels as custom units.
    pub fn from_file_label(label: impl Into<String>) -> Self {
        let label = label.into();
        match Unit::from_label(&label) {
            Some(unit) => Self::Recognized(unit),
            None => Self::Custom(CustomUnit(label)),
        }
    }

    pub fn as_recognized(&self) -> Option<Unit> {
        match self {
            Self::Recognized(unit) => Some(*unit),
            Self::Custom(_) => None,
        }
    }
}

impl fmt::Display for ChannelUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recognized(unit) => unit.fmt(f),
            Self::Custom(unit) => unit.fmt(f),
        }
    }
}

impl From<Unit> for ChannelUnit {
    fn from(unit: Unit) -> Self {
        Self::Recognized(unit)
    }
}

impl FromStr for ChannelUnit {
    type Err = UnitParseError;

    fn from_str(label: &str) -> Result<Self, Self::Err> {
        label.parse::<Unit>().map(Self::Recognized)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnitParseError {
    #[error("unrecognized channel unit {label:?}; use ChannelUnit::custom for a display-only unit")]
    Unrecognized { label: String },
    #[error("a custom channel unit cannot be empty")]
    EmptyCustom,
    #[error("custom channel unit is {len} bytes; the maximum is {max}")]
    CustomTooLong { len: usize, max: usize },
    #[error("a custom channel unit cannot contain control characters")]
    CustomControlCharacter,
    #[error("channel unit {label:?} is recognized; use ChannelUnit::recognized instead")]
    CustomIsRecognized { label: String },
}

fn normalize_label(label: &str) -> String {
    match label.trim() {
        "°" | "degree" | "degrees" => "deg".to_owned(),
        "meter" | "meters" | "metre" | "metres" => "m".to_owned(),
        "second" | "seconds" | "sec" => "s".to_owned(),
        "hour" | "hours" | "hr" => "h".to_owned(),
        "percent" => "%".to_owned(),
        "kph" | "kmph" => "km/h".to_owned(),
        "mps" => "m/s".to_owned(),
        "mps2" | "m/s/s" => "m/s2".to_owned(),
        "G" => "g".to_owned(),
        "mG" => "mg".to_owned(),
        value => value.replace("^2", "2").replace('²', "2"),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    #[test]
    fn recognized_channel_units_scale_to_base() {
        let mg = Unit::from_label("mg").expect("mg is recognized");
        assert_eq!(mg.quantity(), PhysicalQuantity::Acceleration);
        assert!((mg.to_base() - 0.009_806_65).abs() < 1e-12);
        assert_eq!(mg.to_string(), "mg");
    }

    #[test]
    fn file_labels_parse_aliases_before_using_the_escape_hatch() {
        for (label, expected) in [
            ("µg", "ug"),
            ("μg", "ug"),
            ("m/s²", "m/s2"),
            ("degrees", "deg"),
            ("metres", "m"),
            ("m/s^2", "m/s2"),
            ("m/s/s", "m/s2"),
            ("mG", "mg"),
            ("kph", "km/h"),
        ] {
            let parsed = ChannelUnit::from_file_label(label);
            assert_eq!(parsed.to_string(), expected);
            assert!(matches!(parsed, ChannelUnit::Recognized(_)));
        }
    }

    #[test]
    fn custom_units_are_explicit_and_dimensionless() {
        assert!(matches!(
            "rpm".parse::<ChannelUnit>(),
            Err(UnitParseError::Unrecognized { .. })
        ));
        let custom = ChannelUnit::custom("rpm").expect("valid custom label");
        assert_eq!(custom.to_string(), "rpm");
        assert_eq!(custom.as_recognized(), None);
        assert_eq!(ChannelUnit::from_file_label("rpm"), custom);
    }

    #[test]
    fn file_reader_preserves_unsupported_labels_losslessly() {
        let label = " future\nunit ";
        let parsed = ChannelUnit::from_file_label(label);
        assert_eq!(parsed.to_string(), label);
        assert!(matches!(parsed, ChannelUnit::Custom(_)));
    }

    #[test]
    fn invalid_custom_labels_are_rejected() {
        assert!(matches!(
            ChannelUnit::custom("  "),
            Err(UnitParseError::EmptyCustom)
        ));
        assert!(matches!(
            ChannelUnit::custom("bad\nunit"),
            Err(UnitParseError::CustomControlCharacter)
        ));
        assert!(matches!(
            ChannelUnit::custom("x".repeat(MAX_CUSTOM_UNIT_BYTES + 1)),
            Err(UnitParseError::CustomTooLong { .. })
        ));
        assert!(matches!(
            ChannelUnit::custom("m/s²"),
            Err(UnitParseError::CustomIsRecognized { .. })
        ));
    }

    #[test]
    fn every_base_has_a_canonical_round_trip() {
        let prefixed_count = BaseUnit::iter()
            .map(|base| {
                SiPrefix::iter()
                    .filter(|prefix| base.accepts_prefix(*prefix))
                    .count()
            })
            .sum::<usize>();
        assert_eq!(Unit::RECOGNIZED.len(), BaseUnit::COUNT + prefixed_count);
        for unit in Unit::RECOGNIZED {
            let text = unit.to_string();
            assert_eq!(Unit::from_label(&text), Some(unit), "{text}");
        }
    }

    #[test]
    fn conversions_cover_speed_acceleration_duration_ratio_and_rate() {
        assert!((Unit::KM_PER_H.to_base() - 1.0 / 3.6).abs() < 1e-12);
        assert!((Unit::KN.to_base() - 0.514_444_444_444).abs() < 1e-9);
        assert!((Unit::G.to_base() - 9.806_65).abs() < 1e-9);
        assert!((Unit::MIN.to_base() - 60.0).abs() < f64::EPSILON);
        assert!((Unit::H.to_base() - 3600.0).abs() < f64::EPSILON);
        assert!((Unit::PERCENT.to_base() - 0.01).abs() < f64::EPSILON);
        assert!((Unit::PER_S.to_base() - 60.0).abs() < f64::EPSILON);
        assert!((Unit::PER_MIN.to_base() - 1.0).abs() < f64::EPSILON);
        assert!((Unit::PER_H.to_base() - 1.0 / 60.0).abs() < 1e-12);
    }

    #[test]
    fn curated_prefixes_accept_sensor_scales_and_reject_ambiguous_nonsense() {
        for (label, factor) in [
            ("nm", 1e-9),
            ("um", 1e-6),
            ("µm", 1e-6),
            ("mm", 1e-3),
            ("cm", 1e-2),
            ("km", 1e3),
            ("ns", 1e-9),
            ("us", 1e-6),
            ("ms", 1e-3),
            ("ug", 1e-6 * 9.806_65),
            ("mg", 1e-3 * 9.806_65),
            ("mm/s", 1e-3),
            ("cm/s2", 1e-2),
        ] {
            let parsed = Unit::from_label(label).expect("curated unit");
            assert!((parsed.to_base() - factor).abs() < 1e-12, "{label}");
        }
        for label in ["kg", "cs", "ks", "ng", "cg", "km/s", "nm/s2"] {
            assert_eq!(Unit::from_label(label), None, "{label}");
        }
    }

    #[test]
    fn recognized_unit_catalog_is_stable() {
        let mut catalog = String::new();
        for base in BaseUnit::iter() {
            let bare = Unit::base(base);
            writeln!(
                &mut catalog,
                "{} | {:?} | {:.12}",
                bare,
                bare.quantity(),
                bare.to_base()
            )
            .expect("writing to a String cannot fail");
            for prefix in SiPrefix::iter().filter(|prefix| base.accepts_prefix(*prefix)) {
                let unit = Unit::prefixed(prefix, base);
                writeln!(
                    &mut catalog,
                    "{} | {:?} | {:.12}",
                    unit,
                    unit.quantity(),
                    unit.to_base()
                )
                .expect("writing to a String cannot fail");
            }
        }
        insta::assert_snapshot!(catalog);
    }
}
