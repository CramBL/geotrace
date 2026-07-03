//! Unit literals and their conversions to the evaluator's base units.
//!
//! Base units per quantity: angle/direction in degrees, length in meters,
//! speed in m/s, acceleration in m/s2, duration in seconds, ratio as a 0-1
//! fraction, rate in events per minute. Providers deliver values in these
//! bases and literals are converted to them at check time, so the evaluator
//! is plain f64 arithmetic.

use uom::si::f64::Velocity;
use uom::si::velocity::{kilometer_per_hour, knot, meter_per_second};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Deg,
    M,
    Km,
    KmPerH,
    MPerS,
    Kn,
    MPerS2,
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
            Unit::MPerS2 => Quantity::Acceleration,
            Unit::Ms | Unit::S | Unit::Min | Unit::H => Quantity::Duration,
            Unit::Percent => Quantity::Ratio,
            Unit::PerS | Unit::PerMin | Unit::PerH => Quantity::Rate,
        }
    }

    /// Factor converting a literal in this unit to the quantity's base unit.
    pub fn to_base(self) -> f64 {
        match self {
            Unit::Deg | Unit::M | Unit::MPerS | Unit::MPerS2 | Unit::S | Unit::PerMin => 1.0,
            Unit::Km => M_PER_KM,
            Unit::KmPerH => Velocity::new::<kilometer_per_hour>(1.0).get::<meter_per_second>(),
            Unit::Kn => Velocity::new::<knot>(1.0).get::<meter_per_second>(),
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
            "kn" => Some(Unit::Kn),
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
        Quantity::Acceleration => Some("m/s2"),
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
}
