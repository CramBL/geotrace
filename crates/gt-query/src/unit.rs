//! Unit literals and their conversions to the evaluator's base units.
//!
//! Base units per quantity: angle/direction in degrees, length in meters,
//! speed in m/s, acceleration in m/s2, duration in seconds, ratio as a 0-1
//! fraction, rate in events per minute. Providers deliver values in these
//! bases and literals are converted to them at check time, so the evaluator
//! is plain f64 arithmetic.
//!
//! A unit as written is a [`BaseUnit`] optionally scaled by an [`SiPrefix`]
//! ([`Unit`] composes the two): `mg` is milli-gravities, `cm` centimeters,
//! `mm/s2` millimeter-per-second-squared. Each base accepts a curated prefix
//! set (see [`BaseUnit::accepts_prefix`]) so nonsense like `kg`
//! (kilo-gravities) stays a parse error. The familiar `km` and `ms` are
//! compositions too, not bases of their own.

use uom::si::acceleration::{meter_per_second_squared, standard_gravity};
use uom::si::f64::{Acceleration, Velocity};
use uom::si::velocity::{kilometer_per_hour, knot, meter_per_second};

use crate::dimension::Dimension;
use crate::metric::Quantity;

const S_PER_MIN: f64 = 60.0;
const MIN_PER_H: f64 = 60.0;
const PERCENT: f64 = 100.0;
/// One event per second is 60 per minute (the rate base unit). Same value as
/// [`S_PER_MIN`] by coincidence, but an independent domain fact.
const PER_S_TO_PER_MIN: f64 = 60.0;

/// An SI prefix scaling a [`BaseUnit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumCount)]
pub enum SiPrefix {
    Nano,
    Micro,
    Milli,
    Centi,
    Kilo,
}

impl SiPrefix {
    /// The scale this prefix applies to its base unit.
    pub fn factor(self) -> f64 {
        match self {
            SiPrefix::Nano => 1e-9,
            SiPrefix::Micro => 1e-6,
            SiPrefix::Milli => 1e-3,
            SiPrefix::Centi => 1e-2,
            SiPrefix::Kilo => 1e3,
        }
    }

    /// Canonical spelling, as written before the base unit.
    pub fn text(self) -> &'static str {
        match self {
            SiPrefix::Nano => "n",
            SiPrefix::Micro => "u",
            SiPrefix::Milli => "m",
            SiPrefix::Centi => "c",
            SiPrefix::Kilo => "k",
        }
    }

    /// The prefix an identifier starts with, with the remainder. Both `u`
    /// and `µ` spell micro. Longest spelling first, though no current
    /// spelling is a prefix of another.
    fn split(ident: &str) -> Option<(SiPrefix, &str)> {
        let micro = ident
            .strip_prefix('u')
            .or_else(|| ident.strip_prefix('µ'))
            .map(|rest| (SiPrefix::Micro, rest));
        micro.or_else(|| {
            let (first, rest) = ident.split_at_checked(1)?;
            let prefix = match first {
                "n" => SiPrefix::Nano,
                "m" => SiPrefix::Milli,
                "c" => SiPrefix::Centi,
                "k" => SiPrefix::Kilo,
                _ => return None,
            };
            Some((prefix, rest))
        })
    }
}

/// An unprefixed unit as written in a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumCount)]
pub enum BaseUnit {
    Deg,
    M,
    KmPerH,
    MPerS,
    Kn,
    MPerS2,
    /// Standard gravities, `g`.
    G,
    /// Kilometres per hour per second, `km/h/s`.
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
    pub fn quantity(self) -> Quantity {
        match self {
            BaseUnit::Deg => Quantity::Angle,
            BaseUnit::M => Quantity::Length,
            BaseUnit::KmPerH | BaseUnit::MPerS | BaseUnit::Kn => Quantity::Speed,
            BaseUnit::MPerS2 | BaseUnit::G | BaseUnit::KmPerHPerS => Quantity::Acceleration,
            BaseUnit::S | BaseUnit::Min | BaseUnit::H => Quantity::Duration,
            BaseUnit::Percent => Quantity::Ratio,
            BaseUnit::PerS | BaseUnit::PerMin | BaseUnit::PerH => Quantity::Rate,
        }
    }

    /// Factor converting a literal in this unit to the quantity's base unit.
    pub fn to_base(self) -> f64 {
        match self {
            BaseUnit::Deg
            | BaseUnit::M
            | BaseUnit::MPerS
            | BaseUnit::MPerS2
            | BaseUnit::S
            | BaseUnit::PerMin => 1.0,
            // km/h and km/h/s share this factor: both carry the "km/h"
            // magnitude into SI, and the trailing "/s" of km/h/s changes only
            // the dimension (speed to acceleration), not the number.
            BaseUnit::KmPerH | BaseUnit::KmPerHPerS => {
                Velocity::new::<kilometer_per_hour>(1.0).get::<meter_per_second>()
            }
            BaseUnit::Kn => Velocity::new::<knot>(1.0).get::<meter_per_second>(),
            BaseUnit::G => {
                Acceleration::new::<standard_gravity>(1.0).get::<meter_per_second_squared>()
            }
            BaseUnit::Min => S_PER_MIN,
            BaseUnit::H => S_PER_MIN * MIN_PER_H,
            BaseUnit::Percent => 1.0 / PERCENT,
            BaseUnit::PerS => PER_S_TO_PER_MIN,
            BaseUnit::PerH => 1.0 / MIN_PER_H,
        }
    }

    /// Source spelling, used by error messages and the canonical formatter.
    pub fn text(self) -> &'static str {
        match self {
            BaseUnit::Deg => "deg",
            BaseUnit::M => "m",
            BaseUnit::KmPerH => "km/h",
            BaseUnit::MPerS => "m/s",
            BaseUnit::Kn => "kn",
            BaseUnit::MPerS2 => "m/s2",
            BaseUnit::G => "g",
            BaseUnit::KmPerHPerS => "km/h/s",
            BaseUnit::S => "s",
            BaseUnit::Min => "min",
            BaseUnit::H => "h",
            BaseUnit::Percent => "%",
            BaseUnit::PerS => "per s",
            BaseUnit::PerMin => "per min",
            BaseUnit::PerH => "per h",
        }
    }

    /// Whether the base accepts `prefix`. Curated per base so a physically
    /// senseless spelling (`kg` as kilo-gravities, `ch` as centi-hours) stays
    /// a parse error with the usual "expected a … unit" diagnostics:
    /// lengths take the full nm..km run, durations the sub-second run,
    /// gravities the small IMU-spec scales, and the metric compounds the
    /// sub-meter numerators (`mm/s`, `cm/s2`). `km/h` needs no prefixing -
    /// its `km` is already spelled out - and the remaining bases take none.
    pub fn accepts_prefix(self, prefix: SiPrefix) -> bool {
        match self {
            BaseUnit::M => true,
            BaseUnit::S => matches!(prefix, SiPrefix::Nano | SiPrefix::Micro | SiPrefix::Milli),
            BaseUnit::G => matches!(prefix, SiPrefix::Micro | SiPrefix::Milli),
            BaseUnit::MPerS | BaseUnit::MPerS2 => {
                matches!(prefix, SiPrefix::Milli | SiPrefix::Centi)
            }
            BaseUnit::Deg
            | BaseUnit::KmPerH
            | BaseUnit::Kn
            | BaseUnit::KmPerHPerS
            | BaseUnit::Min
            | BaseUnit::H
            | BaseUnit::Percent
            | BaseUnit::PerS
            | BaseUnit::PerMin
            | BaseUnit::PerH => false,
        }
    }

    /// Single-word base unit for the given identifier, if any (`%` and
    /// `per …` forms have their own tokens and are handled by the parser).
    fn from_ident(ident: &str) -> Option<BaseUnit> {
        match ident {
            "deg" => Some(BaseUnit::Deg),
            "m" => Some(BaseUnit::M),
            // `kmh` desugars to the compound `km/h`.
            "kmh" => Some(BaseUnit::KmPerH),
            "kn" => Some(BaseUnit::Kn),
            "g" => Some(BaseUnit::G),
            "s" => Some(BaseUnit::S),
            "min" => Some(BaseUnit::Min),
            "h" => Some(BaseUnit::H),
            _ => None,
        }
    }
}

/// A unit as written in a query: a base, optionally scaled by an SI prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unit {
    pub prefix: Option<SiPrefix>,
    pub base: BaseUnit,
}

impl Unit {
    pub const DEG: Unit = Unit::base(BaseUnit::Deg);
    pub const M: Unit = Unit::base(BaseUnit::M);
    pub const KM: Unit = Unit::prefixed(SiPrefix::Kilo, BaseUnit::M);
    pub const KM_PER_H: Unit = Unit::base(BaseUnit::KmPerH);
    pub const M_PER_S: Unit = Unit::base(BaseUnit::MPerS);
    pub const KN: Unit = Unit::base(BaseUnit::Kn);
    pub const M_PER_S2: Unit = Unit::base(BaseUnit::MPerS2);
    pub const G: Unit = Unit::base(BaseUnit::G);
    pub const KM_PER_H_PER_S: Unit = Unit::base(BaseUnit::KmPerHPerS);
    pub const MS: Unit = Unit::prefixed(SiPrefix::Milli, BaseUnit::S);
    pub const S: Unit = Unit::base(BaseUnit::S);
    pub const MIN: Unit = Unit::base(BaseUnit::Min);
    pub const H: Unit = Unit::base(BaseUnit::H);
    pub const PERCENT: Unit = Unit::base(BaseUnit::Percent);
    pub const PER_S: Unit = Unit::base(BaseUnit::PerS);
    pub const PER_MIN: Unit = Unit::base(BaseUnit::PerMin);
    pub const PER_H: Unit = Unit::base(BaseUnit::PerH);

    /// The canonical units, offered by the editor's completion and listed in
    /// "expected a … unit" errors. Prefixed forms beyond the household `km`
    /// and `ms` parse but are not suggested - the cross-product would drown
    /// the list.
    pub const CANONICAL: [Unit; 17] = [
        Unit::DEG,
        Unit::M,
        Unit::KM,
        Unit::KM_PER_H,
        Unit::M_PER_S,
        Unit::KN,
        Unit::M_PER_S2,
        Unit::G,
        Unit::KM_PER_H_PER_S,
        Unit::MS,
        Unit::S,
        Unit::MIN,
        Unit::H,
        Unit::PERCENT,
        Unit::PER_S,
        Unit::PER_MIN,
        Unit::PER_H,
    ];

    const fn base(base: BaseUnit) -> Unit {
        Unit { prefix: None, base }
    }

    const fn prefixed(prefix: SiPrefix, base: BaseUnit) -> Unit {
        Unit {
            prefix: Some(prefix),
            base,
        }
    }

    pub fn quantity(self) -> Quantity {
        self.base.quantity()
    }

    /// The physical dimension of a literal in this unit (a prefix scales the
    /// magnitude, never the dimension). Total, because every unit is
    /// dimensioned - none maps to a timestamp or a condition - so this agrees
    /// with `self.quantity().dimension()` on every variant (pinned by a test).
    pub fn dimension(self) -> Dimension {
        match self.base {
            BaseUnit::Deg => Dimension::ANGLE,
            BaseUnit::M => Dimension::LENGTH,
            BaseUnit::KmPerH | BaseUnit::MPerS | BaseUnit::Kn => Dimension::SPEED,
            BaseUnit::MPerS2 | BaseUnit::G | BaseUnit::KmPerHPerS => Dimension::ACCELERATION,
            BaseUnit::S | BaseUnit::Min | BaseUnit::H => Dimension::TIME,
            BaseUnit::Percent => Dimension::DIMENSIONLESS,
            BaseUnit::PerS | BaseUnit::PerMin | BaseUnit::PerH => Dimension::RATE,
        }
    }

    /// Factor converting a literal in this unit to the quantity's base unit.
    pub fn to_base(self) -> f64 {
        self.prefix.map_or(1.0, SiPrefix::factor) * self.base.to_base()
    }

    /// The canonical spelling, for error messages and the query formatter.
    /// [`std::fmt::Display`] rather than a `&'static str`: a prefixed
    /// spelling is composed of its parts.
    pub fn text(self) -> UnitText {
        UnitText(self)
    }

    /// The `&'static` spelling of a [`Unit::CANONICAL`] unit - what the
    /// completion catalog inserts. `None` for the other prefixed forms,
    /// whose spellings only exist composed.
    pub fn canonical_text(self) -> Option<&'static str> {
        match (self.prefix, self.base) {
            (None, base) => Some(base.text()),
            (Some(SiPrefix::Kilo), BaseUnit::M) => Some("km"),
            (Some(SiPrefix::Milli), BaseUnit::S) => Some("ms"),
            _ => None,
        }
    }

    /// Single-word unit for the given identifier, if any: an exact base
    /// spelling first (`min` is minutes, never milli-anything), else an SI
    /// prefix followed by a base that accepts it (`mg`, `cm`, `km`, `us`).
    pub fn from_ident(ident: &str) -> Option<Unit> {
        if let Some(base) = BaseUnit::from_ident(ident) {
            return Some(Unit::base(base));
        }
        let (prefix, rest) = SiPrefix::split(ident)?;
        let base = BaseUnit::from_ident(rest)?;
        base.accepts_prefix(prefix)
            .then_some(Unit::prefixed(prefix, base))
    }

    /// Compound `first/second` unit, if the pair is one (`km/h`, `m/s`,
    /// `m/s2`). The metric compounds also take a numerator prefix
    /// (`mm/s`, `cm/s2`).
    pub fn from_pair(first: &str, second: &str) -> Option<Unit> {
        let exact = match (first, second) {
            ("km", "h") => Some(BaseUnit::KmPerH),
            ("m", "s") => Some(BaseUnit::MPerS),
            ("m", "s2") => Some(BaseUnit::MPerS2),
            _ => None,
        };
        if let Some(base) = exact {
            return Some(Unit::base(base));
        }
        let (prefix, rest) = SiPrefix::split(first)?;
        let base = match (rest, second) {
            ("m", "s") => BaseUnit::MPerS,
            ("m", "s2") => BaseUnit::MPerS2,
            _ => return None,
        };
        base.accepts_prefix(prefix)
            .then_some(Unit::prefixed(prefix, base))
    }

    /// Compound `first/second/third` unit, if the triple is one (`km/h/s`).
    pub fn from_triple(first: &str, second: &str, third: &str) -> Option<Unit> {
        match (first, second, third) {
            ("km", "h", "s") => Some(Unit::base(BaseUnit::KmPerHPerS)),
            _ => None,
        }
    }

    /// A unit from a producer's free-form unit label, as carried by a
    /// channel's metadata: a single identifier (`g`, `mg`, `deg`), `%`, or a
    /// compound slash form (`km/h`, `mm/s2`, `km/h/s`). The `per …` rate
    /// spellings are query syntax, not labels, and are not accepted here.
    pub fn from_label(label: &str) -> Option<Unit> {
        if label == "%" {
            return Some(Unit::PERCENT);
        }
        let parts: Vec<&str> = label.split('/').collect();
        match parts.as_slice() {
            [a] => Unit::from_ident(a),
            [a, b] => Unit::from_pair(a, b),
            [a, b, c] => Unit::from_triple(a, b, c),
            _ => None,
        }
    }
}

/// [`Unit`]'s spelling as a `Display` value, from [`Unit::text`].
#[derive(Clone, Copy)]
pub struct UnitText(Unit);

impl std::fmt::Display for UnitText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(prefix) = self.0.prefix {
            write!(f, "{}", prefix.text())?;
        }
        write!(f, "{}", self.0.base.text())
    }
}

/// Example literal used in "needs a unit" error help, per quantity.
pub fn example_literal(quantity: Quantity) -> Option<&'static str> {
    match quantity {
        Quantity::Angle | Quantity::Direction => Some("10 deg"),
        Quantity::Speed => Some("30 km/h"),
        Quantity::Acceleration => Some("0.3 m/s2"),
        Quantity::Length => Some("20 m"),
        Quantity::Duration => Some("15 s"),
        Quantity::Ratio => Some("50 %"),
        Quantity::Rate => Some("2 per min"),
        Quantity::Timestamp | Quantity::Count | Quantity::Condition => None,
    }
}

/// The accepted units for a quantity, for "expected a … unit" errors. SI
/// prefixes on m, s, g, and the metric compounds parse too; the lists stay
/// canonical so the message stays short.
pub fn unit_list(quantity: Quantity) -> Option<&'static str> {
    match quantity {
        Quantity::Angle | Quantity::Direction => Some("deg"),
        Quantity::Speed => Some("km/h, m/s, kn"),
        Quantity::Acceleration => Some("m/s2, g, km/h/s"),
        Quantity::Length => Some("m, km"),
        Quantity::Duration => Some("ms, s, min, h"),
        Quantity::Ratio => Some("%"),
        Quantity::Rate => Some("per s, per min, per h"),
        Quantity::Timestamp | Quantity::Count | Quantity::Condition => None,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    /// The error-help tables answer for every dimensioned quantity: a
    /// quantity with units lists them and shows an example literal, the
    /// unitless three (timestamp, count, condition) answer `None` in both.
    /// Iterating the enum, so a new `Quantity` variant fails here until both
    /// tables cover it.
    #[test]
    fn error_help_tables_cover_every_quantity() {
        for quantity in Quantity::iter() {
            let unitless = matches!(
                quantity,
                Quantity::Timestamp | Quantity::Count | Quantity::Condition
            );
            assert_eq!(
                unit_list(quantity).is_none(),
                unitless,
                "{quantity} unit list"
            );
            assert_eq!(
                example_literal(quantity).is_none(),
                unitless,
                "{quantity} example literal"
            );
        }
    }

    #[test]
    fn speed_conversions_are_exact_enough() {
        let kmh = Unit::KM_PER_H.to_base();
        assert!((kmh - 1.0 / 3.6).abs() < 1e-12, "km/h factor {kmh}");
        let kn = Unit::KN.to_base();
        assert!((kn - 0.514_444_444_444).abs() < 1e-9, "kn factor {kn}");
    }

    #[test]
    fn rate_base_is_per_minute() {
        assert!((Unit::PER_S.to_base() - 60.0).abs() < 1e-12);
        assert!((Unit::PER_MIN.to_base() - 1.0).abs() < 1e-12);
        assert!((Unit::PER_H.to_base() - 1.0 / 60.0).abs() < 1e-12);
    }

    #[test]
    fn acceleration_units_convert_to_m_per_s2() {
        assert_eq!(Unit::M_PER_S2.quantity(), Quantity::Acceleration);
        assert_eq!(Unit::G.quantity(), Quantity::Acceleration);
        assert_eq!(Unit::KM_PER_H_PER_S.quantity(), Quantity::Acceleration);
        assert!((Unit::M_PER_S2.to_base() - 1.0).abs() < 1e-12);
        assert!((Unit::G.to_base() - 9.806_65).abs() < 1e-9, "1 g in m/s2");
        // km/h/s shares km/h's numeric factor.
        assert!((Unit::KM_PER_H_PER_S.to_base() - 1.0 / 3.6).abs() < 1e-12);
    }

    /// Each unit's dimension agrees with its quantity's dimension, tying the
    /// two hand-written tables together. Exhaustive over the bases (a prefix
    /// never changes the dimension), so a new base must appear in both.
    #[test]
    fn unit_dimensions_match_their_quantity() {
        for base in BaseUnit::iter() {
            let unit = Unit { prefix: None, base };
            assert_eq!(
                Some(unit.dimension()),
                unit.quantity().dimension(),
                "{} dimension must agree with its quantity",
                unit.text()
            );
        }
        // A few pinned outright.
        assert_eq!(Unit::DEG.dimension(), Dimension::ANGLE);
        assert_eq!(Unit::KM_PER_H.dimension(), Dimension::SPEED);
        assert_eq!(Unit::G.dimension(), Dimension::ACCELERATION);
        assert_eq!(Unit::PERCENT.dimension(), Dimension::DIMENSIONLESS);
        assert_eq!(Unit::PER_MIN.dimension(), Dimension::RATE);
        // A prefix scales the factor, not the dimension.
        assert_eq!(Unit::KM.dimension(), Unit::M.dimension());
        assert_eq!(Unit::MS.dimension(), Unit::S.dimension());
    }

    /// The canonical list covers every base exactly once (plus the two
    /// household prefixed spellings), so a new base must be listed to be
    /// offered by the completion.
    #[test]
    fn canonical_covers_every_base() {
        let bases: Vec<BaseUnit> = Unit::CANONICAL
            .iter()
            .filter(|u| u.prefix.is_none())
            .map(|u| u.base)
            .collect();
        assert_eq!(bases.len(), BaseUnit::COUNT);
        for base in BaseUnit::iter() {
            assert!(bases.contains(&base), "{} missing", base.text());
        }
        assert_eq!(
            Unit::CANONICAL.len(),
            BaseUnit::COUNT + 2,
            "the canonical extras are km and ms"
        );
        assert!(Unit::CANONICAL.contains(&Unit::KM));
        assert!(Unit::CANONICAL.contains(&Unit::MS));
    }

    #[rstest]
    // The household prefixed spellings, unchanged from their variant days.
    #[case("km", Some(1_000.0))]
    #[case("ms", Some(1e-3))]
    // Length runs nm..km.
    #[case("nm", Some(1e-9))]
    #[case("um", Some(1e-6))]
    #[case("µm", Some(1e-6))]
    #[case("mm", Some(1e-3))]
    #[case("cm", Some(1e-2))]
    // Sub-second durations.
    #[case("ns", Some(1e-9))]
    #[case("us", Some(1e-6))]
    // IMU-spec accelerations.
    #[case("ug", Some(1e-6 * 9.806_65))]
    #[case("mg", Some(1e-3 * 9.806_65))]
    // Exact bases win over decomposition.
    #[case("min", Some(60.0))]
    #[case("m", Some(1.0))]
    #[case("g", Some(9.806_65))]
    // Curated rejections: senseless prefix-base combinations.
    #[case("kg", None)]
    #[case("cs", None)]
    #[case("ks", None)]
    #[case("cg", None)]
    #[case("mmin", None)]
    #[case("cdeg", None)]
    #[case("kkn", None)]
    #[case("mh", None)]
    fn prefixed_ident_converts_or_rejects(#[case] ident: &str, #[case] factor: Option<f64>) {
        let unit = Unit::from_ident(ident);
        match factor {
            Some(expected) => {
                let unit = unit.unwrap_or_else(|| panic!("{ident} must parse"));
                let got = unit.to_base();
                assert!(
                    (got - expected).abs() < 1e-15,
                    "{ident}: {got} vs {expected}"
                );
            }
            None => assert_eq!(unit, None, "{ident} must not parse"),
        }
    }

    #[rstest]
    // The metric compounds take sub-meter numerator prefixes.
    #[case("mm", "s", Some(1e-3))]
    #[case("cm", "s", Some(1e-2))]
    #[case("mm", "s2", Some(1e-3))]
    #[case("cm", "s2", Some(1e-2))]
    // km/h spells its own km; a prefixed numerator is not a unit.
    #[case("mkm", "h", None)]
    #[case("km", "s", None)]
    #[case("um", "s", None)]
    fn prefixed_compound_converts_or_rejects(
        #[case] first: &str,
        #[case] second: &str,
        #[case] factor: Option<f64>,
    ) {
        let unit = Unit::from_pair(first, second);
        match factor {
            Some(expected) => {
                let unit = unit.unwrap_or_else(|| panic!("{first}/{second} must parse"));
                let got = unit.to_base();
                assert!(
                    (got - expected).abs() < 1e-15,
                    "{first}/{second}: {got} vs {expected}"
                );
            }
            None => assert_eq!(unit, None, "{first}/{second} must not parse"),
        }
    }

    #[test]
    fn channel_labels_resolve_prefixed_units() {
        assert_eq!(
            Unit::from_label("mg"),
            Some(Unit::from_ident("mg").unwrap())
        );
        assert_eq!(
            Unit::from_label("cm"),
            Some(Unit::from_ident("cm").unwrap())
        );
        assert_eq!(
            Unit::from_label("mm/s2").map(Unit::to_base),
            Some(1e-3),
            "a channel spec'd in mm/s2 converts"
        );
        assert_eq!(Unit::from_label("kg"), None);
        assert_eq!(
            Unit::from_label("mg").map(Unit::quantity),
            Some(Quantity::Acceleration)
        );
    }

    /// Every canonical unit's spelling round-trips: single-ident and `kmh`
    /// forms through `from_ident`, compounds through `from_pair` /
    /// `from_triple`. The `%`/`per …` forms are parser-level tokens and are
    /// covered there. Every *accepted* prefixed spelling round-trips too.
    #[test]
    fn unit_text_round_trips() {
        let reparse = |text: &str| {
            Unit::from_ident(text).or_else(|| {
                match text.split('/').collect::<Vec<_>>().as_slice() {
                    [a, b] => Unit::from_pair(a, b),
                    [a, b, c] => Unit::from_triple(a, b, c),
                    _ => None,
                }
            })
        };
        for unit in Unit::CANONICAL {
            let text = unit.text().to_string();
            // `%` and `per …` are lexed as dedicated tokens, not idents.
            if matches!(
                unit.base,
                BaseUnit::Percent | BaseUnit::PerS | BaseUnit::PerMin | BaseUnit::PerH
            ) {
                continue;
            }
            assert_eq!(reparse(&text), Some(unit), "text {text:?} must round-trip");
        }
        for base in BaseUnit::iter() {
            for prefix in SiPrefix::iter() {
                if !base.accepts_prefix(prefix) {
                    continue;
                }
                let unit = Unit {
                    prefix: Some(prefix),
                    base,
                };
                let text = unit.text().to_string();
                assert_eq!(reparse(&text), Some(unit), "text {text:?} must round-trip");
            }
        }
    }
}
