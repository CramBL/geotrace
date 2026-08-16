//! Unit literals shared with the open GeoTrace SDK.

use geotrace_sdk_units::PhysicalQuantity;
pub use geotrace_sdk_units::Unit;

use crate::dimension::Dimension;
use crate::metric::Quantity;

pub fn quantity(unit: Unit) -> Quantity {
    match unit.quantity() {
        PhysicalQuantity::Angle => Quantity::Angle,
        PhysicalQuantity::Length => Quantity::Length,
        PhysicalQuantity::Speed => Quantity::Speed,
        PhysicalQuantity::Acceleration => Quantity::Acceleration,
        PhysicalQuantity::Duration => Quantity::Duration,
        PhysicalQuantity::Ratio => Quantity::Ratio,
        PhysicalQuantity::Rate => Quantity::Rate,
    }
}

pub fn dimension(unit: Unit) -> Dimension {
    match unit.quantity() {
        PhysicalQuantity::Angle => Dimension::ANGLE,
        PhysicalQuantity::Length => Dimension::LENGTH,
        PhysicalQuantity::Speed => Dimension::SPEED,
        PhysicalQuantity::Acceleration => Dimension::ACCELERATION,
        PhysicalQuantity::Duration => Dimension::TIME,
        PhysicalQuantity::Ratio => Dimension::DIMENSIONLESS,
        PhysicalQuantity::Rate => Dimension::RATE,
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
        Quantity::Timestamp | Quantity::Count | Quantity::Index | Quantity::Condition => None,
    }
}

/// Canonical units shown in diagnostics. Other accepted prefixes stay omitted
/// so the message remains short.
pub fn unit_list(quantity: Quantity) -> Option<&'static str> {
    match quantity {
        Quantity::Angle | Quantity::Direction => Some("deg"),
        Quantity::Speed => Some("km/h, m/s, kn"),
        Quantity::Acceleration => Some("m/s2, g, km/h/s"),
        Quantity::Length => Some("m, km"),
        Quantity::Duration => Some("ms, s, min, h"),
        Quantity::Ratio => Some("%"),
        Quantity::Rate => Some("per s, per min, per h"),
        Quantity::Timestamp | Quantity::Count | Quantity::Index | Quantity::Condition => None,
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use super::*;

    #[test]
    fn error_help_tables_cover_every_quantity() {
        for quantity in Quantity::iter() {
            let unitless = matches!(
                quantity,
                Quantity::Timestamp | Quantity::Count | Quantity::Index | Quantity::Condition
            );
            assert_eq!(unit_list(quantity).is_none(), unitless, "{quantity}");
            assert_eq!(example_literal(quantity).is_none(), unitless, "{quantity}");
        }
    }

    #[test]
    fn shared_units_map_to_query_dimensions() {
        assert_eq!(quantity(Unit::G), Quantity::Acceleration);
        assert_eq!(dimension(Unit::KM_PER_H), Dimension::SPEED);
        assert_eq!(dimension(Unit::PERCENT), Dimension::DIMENSIONLESS);
    }
}
