//! Unit literals and their conversions to the evaluator's base units.
//!
//! Base units per quantity: angle/direction in degrees, length in meters,
//! speed in m/s, acceleration in m/s2, duration in seconds, ratio as a 0-1
//! fraction, rate in events per minute. Providers deliver values in these
//! bases and literals are converted to them at check time, so the evaluator
//! is plain f64 arithmetic.

use uom::si::acceleration::{meter_per_second_squared, standard_gravity};
use uom::si::f64::{Acceleration, Velocity};
use uom::si::velocity::{kilometer_per_hour, knot, meter_per_second};

use crate::dimension::Dimension;
use crate::metric::Quantity;

const M_PER_KM: f64 = 1_000.0;
const MS_PER_S: f64 = 1_000.0;
const S_PER_MIN: f64 = 60.0;
const MIN_PER_H: f64 = 60.0;
const PERCENT: f64 = 100.0;
/// One event per second is 60 per minute (the rate base unit). Same value as
/// [`S_PER_MIN`] by coincidence, but an independent domain fact.
const PER_S_TO_PER_MIN: f64 = 60.0;

/// A unit as written in a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumCount)]
pub enum Unit {
    Deg,
    M,
    Km,
    KmPerH,
    MPerS,
    Kn,
    MPerS2,
    /// Standard gravities, `g`.
    G,
    /// Kilometres per hour per second, `km/h/s`.
    KmPerHPerS,
    Ms,
    S,
    Min,
    H,
    Percent,
    PerS,
    PerMin,
    PerH,
}

impl Unit {
    pub fn quantity(self) -> Quantity {
        match self {
            Unit::Deg => Quantity::Angle,
            Unit::M | Unit::Km => Quantity::Length,
            Unit::KmPerH | Unit::MPerS | Unit::Kn => Quantity::Speed,
            Unit::MPerS2 | Unit::G | Unit::KmPerHPerS => Quantity::Acceleration,
            Unit::Ms | Unit::S | Unit::Min | Unit::H => Quantity::Duration,
            Unit::Percent => Quantity::Ratio,
            Unit::PerS | Unit::PerMin | Unit::PerH => Quantity::Rate,
        }
    }

    /// The physical dimension of a literal in this unit. Total, because every
    /// unit is dimensioned - none maps to a timestamp or a condition - so this
    /// agrees with `self.quantity().dimension()` on every variant (pinned by a
    /// test).
    pub fn dimension(self) -> Dimension {
        match self {
            Unit::Deg => Dimension::ANGLE,
            Unit::M | Unit::Km => Dimension::LENGTH,
            Unit::KmPerH | Unit::MPerS | Unit::Kn => Dimension::SPEED,
            Unit::MPerS2 | Unit::G | Unit::KmPerHPerS => Dimension::ACCELERATION,
            Unit::Ms | Unit::S | Unit::Min | Unit::H => Dimension::TIME,
            Unit::Percent => Dimension::DIMENSIONLESS,
            Unit::PerS | Unit::PerMin | Unit::PerH => Dimension::RATE,
        }
    }

    /// Factor converting a literal in this unit to the quantity's base unit.
    pub fn to_base(self) -> f64 {
        match self {
            Unit::Deg | Unit::M | Unit::MPerS | Unit::MPerS2 | Unit::S | Unit::PerMin => 1.0,
            Unit::Km => M_PER_KM,
            // km/h and km/h/s share this factor: both carry the "km/h"
            // magnitude into SI, and the trailing "/s" of km/h/s changes only
            // the dimension (speed to acceleration), not the number.
            Unit::KmPerH | Unit::KmPerHPerS => {
                Velocity::new::<kilometer_per_hour>(1.0).get::<meter_per_second>()
            }
            Unit::Kn => Velocity::new::<knot>(1.0).get::<meter_per_second>(),
            Unit::G => Acceleration::new::<standard_gravity>(1.0).get::<meter_per_second_squared>(),
            Unit::Ms => 1.0 / MS_PER_S,
            Unit::Min => S_PER_MIN,
            Unit::H => S_PER_MIN * MIN_PER_H,
            Unit::Percent => 1.0 / PERCENT,
            Unit::PerS => PER_S_TO_PER_MIN,
            Unit::PerH => 1.0 / MIN_PER_H,
        }
    }

    /// Source spelling, used by error messages and the canonical formatter.
    pub fn text(self) -> &'static str {
        match self {
            Unit::Deg => "deg",
            Unit::M => "m",
            Unit::Km => "km",
            Unit::KmPerH => "km/h",
            Unit::MPerS => "m/s",
            Unit::Kn => "kn",
            Unit::MPerS2 => "m/s2",
            Unit::G => "g",
            Unit::KmPerHPerS => "km/h/s",
            Unit::Ms => "ms",
            Unit::S => "s",
            Unit::Min => "min",
            Unit::H => "h",
            Unit::Percent => "%",
            Unit::PerS => "per s",
            Unit::PerMin => "per min",
            Unit::PerH => "per h",
        }
    }

    /// Single-word unit for the given identifier, if any (`%` and `per …`
    /// forms have their own tokens and are handled by the parser).
    pub fn from_ident(ident: &str) -> Option<Unit> {
        match ident {
            "deg" => Some(Unit::Deg),
            "m" => Some(Unit::M),
            "km" => Some(Unit::Km),
            // `kmh` desugars to the compound `km/h`.
            "kmh" => Some(Unit::KmPerH),
            "kn" => Some(Unit::Kn),
            "g" => Some(Unit::G),
            "ms" => Some(Unit::Ms),
            "s" => Some(Unit::S),
            "min" => Some(Unit::Min),
            "h" => Some(Unit::H),
            _ => None,
        }
    }

    /// Compound `first/second` unit, if the pair is one (`km/h`, `m/s`, `m/s2`).
    pub fn from_pair(first: &str, second: &str) -> Option<Unit> {
        match (first, second) {
            ("km", "h") => Some(Unit::KmPerH),
            ("m", "s") => Some(Unit::MPerS),
            ("m", "s2") => Some(Unit::MPerS2),
            _ => None,
        }
    }

    /// Compound `first/second/third` unit, if the triple is one (`km/h/s`).
    pub fn from_triple(first: &str, second: &str, third: &str) -> Option<Unit> {
        match (first, second, third) {
            ("km", "h", "s") => Some(Unit::KmPerHPerS),
            _ => None,
        }
    }

    /// A unit from a producer's free-form unit label, as carried by a channel's
    /// metadata: a single identifier (`g`, `deg`), `%`, or a compound slash
    /// form (`km/h`, `m/s2`, `km/h/s`). The `per …` rate spellings are query
    /// syntax, not labels, and are not accepted here.
    pub fn from_label(label: &str) -> Option<Unit> {
        if label == "%" {
            return Some(Unit::Percent);
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

/// The accepted units for a quantity, for "expected a … unit" errors.
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
    use super::*;

    #[test]
    fn speed_conversions_are_exact_enough() {
        let kmh = Unit::KmPerH.to_base();
        assert!((kmh - 1.0 / 3.6).abs() < 1e-12, "km/h factor {kmh}");
        let kn = Unit::Kn.to_base();
        assert!((kn - 0.514_444_444_444).abs() < 1e-9, "kn factor {kn}");
    }

    #[test]
    fn rate_base_is_per_minute() {
        assert!((Unit::PerS.to_base() - 60.0).abs() < 1e-12);
        assert!((Unit::PerMin.to_base() - 1.0).abs() < 1e-12);
        assert!((Unit::PerH.to_base() - 1.0 / 60.0).abs() < 1e-12);
    }

    #[test]
    fn acceleration_units_convert_to_m_per_s2() {
        assert_eq!(Unit::MPerS2.quantity(), Quantity::Acceleration);
        assert_eq!(Unit::G.quantity(), Quantity::Acceleration);
        assert_eq!(Unit::KmPerHPerS.quantity(), Quantity::Acceleration);
        assert!((Unit::MPerS2.to_base() - 1.0).abs() < 1e-12);
        assert!((Unit::G.to_base() - 9.806_65).abs() < 1e-9, "1 g in m/s2");
        // km/h/s shares km/h's numeric factor.
        assert!((Unit::KmPerHPerS.to_base() - 1.0 / 3.6).abs() < 1e-12);
    }

    /// Each unit's dimension agrees with its quantity's dimension, tying the
    /// two hand-written tables together. Exhaustive over the enum, so a new
    /// unit must appear in both.
    #[test]
    fn unit_dimensions_match_their_quantity() {
        use strum::IntoEnumIterator as _;
        for unit in Unit::iter() {
            assert_eq!(
                Some(unit.dimension()),
                unit.quantity().dimension(),
                "{} dimension must agree with its quantity",
                unit.text()
            );
        }
        // A few pinned outright.
        assert_eq!(Unit::Deg.dimension(), Dimension::ANGLE);
        assert_eq!(Unit::KmPerH.dimension(), Dimension::SPEED);
        assert_eq!(Unit::G.dimension(), Dimension::ACCELERATION);
        assert_eq!(Unit::Percent.dimension(), Dimension::DIMENSIONLESS);
        assert_eq!(Unit::PerMin.dimension(), Dimension::RATE);
    }

    #[test]
    fn unit_ident_alias_and_triple_lookups() {
        // `kmh` is an accepted spelling of the compound `km/h`.
        assert_eq!(Unit::from_ident("kmh"), Some(Unit::KmPerH));
        assert_eq!(Unit::from_ident("g"), Some(Unit::G));
        assert_eq!(Unit::from_triple("km", "h", "s"), Some(Unit::KmPerHPerS));
        assert_eq!(Unit::from_triple("m", "s", "s"), None);
    }

    /// Every unit's `text()` spelling round-trips: single-ident and `kmh`
    /// forms through `from_ident`, and `km/h/s` through `from_triple`. The
    /// `%`/`per` forms are parser-level tokens and are covered there.
    #[test]
    fn unit_text_round_trips() {
        use strum::IntoEnumIterator as _;
        for unit in Unit::iter() {
            let text = unit.text();
            let round_tripped = Unit::from_ident(text).or_else(|| {
                match text.split('/').collect::<Vec<_>>().as_slice() {
                    [a, b] => Unit::from_pair(a, b),
                    [a, b, c] => Unit::from_triple(a, b, c),
                    _ => None,
                }
            });
            // `%` and `per …` are lexed as dedicated tokens, not idents.
            if matches!(unit, Unit::Percent | Unit::PerS | Unit::PerMin | Unit::PerH) {
                continue;
            }
            assert_eq!(round_tripped, Some(unit), "text {text:?} must round-trip");
        }
    }
}
