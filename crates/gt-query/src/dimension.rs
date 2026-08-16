//! Physical dimension as integer exponents over the base dimensions the query
//! language tracks: length, time, and angle.
//!
//! A dimension is `length^l · time^t · angle^a`, so speed is `L T⁻¹`,
//! acceleration `L T⁻²`, a rate `T⁻¹`, an angle `A`, and a dimensionless value
//! (a count, a ratio, a bare number) the zero exponents. Arithmetic is algebra
//! on the exponents: multiplication adds them, division subtracts, an integer
//! power scales them, and a square root halves them.
//!
//! Angle is a base dimension here, unlike SI (and [`uom`], which treats angle
//! as dimensionless). Tracking it keeps a heading spread in `deg` from ever
//! comparing against a percentage, which is the whole point of dimensional
//! checking in this language.
//!
//! This type is the exponent arithmetic only. The distinction between a count,
//! a ratio, and a bare number is carried by the checker's own kind tag.

use std::ops::{Div, Mul};

/// A physical dimension: exponents of length, time, and angle.
///
/// The exponents are `i8`, which stays compact where the dimension is embedded
/// (in the checker's value type) while giving ample headroom for exponents that
/// stay within a handful of units. Arithmetic saturates, and a saturated
/// dimension matches no named dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dimension {
    pub length: i8,
    pub time: i8,
    pub angle: i8,
}

impl Dimension {
    /// A dimensionless quantity: a count, a ratio, or a bare number.
    pub const DIMENSIONLESS: Dimension = Dimension::new(0, 0, 0);
    pub const LENGTH: Dimension = Dimension::new(1, 0, 0);
    pub const TIME: Dimension = Dimension::new(0, 1, 0);
    pub const ANGLE: Dimension = Dimension::new(0, 0, 1);
    pub const SPEED: Dimension = Dimension::new(1, -1, 0);
    pub const ACCELERATION: Dimension = Dimension::new(1, -2, 0);
    /// Events per unit time, `T⁻¹`.
    pub const RATE: Dimension = Dimension::new(0, -1, 0);

    const fn new(length: i8, time: i8, angle: i8) -> Dimension {
        Dimension {
            length,
            time,
            angle,
        }
    }

    pub fn is_dimensionless(self) -> bool {
        self == Dimension::DIMENSIONLESS
    }

    /// Raised to an integer power: exponents scale, so `speed² = L² T⁻²`.
    pub fn powi(self, n: i8) -> Dimension {
        Dimension {
            length: self.length.saturating_mul(n),
            time: self.time.saturating_mul(n),
            angle: self.angle.saturating_mul(n),
        }
    }

    /// Square root: exponents halve, so `sqrt(L² T⁻²) = speed`. `None` when any
    /// exponent is odd, since a whole-number dimension has no square root then
    /// (e.g. `sqrt(length)` is not expressible).
    pub fn sqrt(self) -> Option<Dimension> {
        let even = |e: i8| e % 2 == 0;
        (even(self.length) && even(self.time) && even(self.angle)).then_some(Dimension {
            length: self.length / 2,
            time: self.time / 2,
            angle: self.angle / 2,
        })
    }
}

/// Product of two dimensions: their exponents add, so `speed · time = length`.
impl Mul for Dimension {
    type Output = Dimension;

    fn mul(self, other: Dimension) -> Dimension {
        Dimension {
            length: self.length.saturating_add(other.length),
            time: self.time.saturating_add(other.time),
            angle: self.angle.saturating_add(other.angle),
        }
    }
}

/// Quotient of two dimensions: their exponents subtract, so
/// `length / time = speed`.
impl Div for Dimension {
    type Output = Dimension;

    fn div(self, other: Dimension) -> Dimension {
        Dimension {
            length: self.length.saturating_sub(other.length),
            time: self.time.saturating_sub(other.time),
            angle: self.angle.saturating_sub(other.angle),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplication_and_division_move_exponents() {
        assert_eq!(Dimension::SPEED * Dimension::TIME, Dimension::LENGTH);
        assert_eq!(Dimension::LENGTH / Dimension::TIME, Dimension::SPEED);
        assert_eq!(Dimension::SPEED / Dimension::TIME, Dimension::ACCELERATION);
        // Dimensionless is the identity for both.
        assert_eq!(
            Dimension::SPEED * Dimension::DIMENSIONLESS,
            Dimension::SPEED
        );
        assert_eq!(
            Dimension::SPEED / Dimension::DIMENSIONLESS,
            Dimension::SPEED
        );
    }

    #[test]
    fn a_quantity_divided_by_itself_is_dimensionless() {
        assert!((Dimension::SPEED / Dimension::SPEED).is_dimensionless());
        assert!((Dimension::ANGLE / Dimension::ANGLE).is_dimensionless());
    }

    #[test]
    fn integer_powers_scale_exponents() {
        assert_eq!(
            Dimension::SPEED.powi(2),
            Dimension::new(2, -2, 0),
            "speed squared is L² T⁻²"
        );
        assert_eq!(Dimension::ANGLE.powi(3), Dimension::new(0, 0, 3));
        assert!(Dimension::SPEED.powi(0).is_dimensionless());
        assert_eq!(
            Dimension::SPEED.powi(-1),
            Dimension::new(-1, 1, 0),
            "a negative power inverts"
        );
    }

    #[test]
    fn square_root_halves_even_exponents() {
        assert_eq!(Dimension::SPEED.powi(2).sqrt(), Some(Dimension::SPEED));
        // The magnitude of a vector: each component squared, summed (same
        // dimension), then rooted back to the component's dimension.
        assert_eq!(
            Dimension::ACCELERATION.powi(2).sqrt(),
            Some(Dimension::ACCELERATION)
        );
        assert!(Dimension::DIMENSIONLESS.sqrt().is_some());
    }

    #[test]
    fn square_root_rejects_odd_exponents() {
        // length is L¹, acceleration is L¹ T⁻²: both carry an odd exponent.
        assert_eq!(Dimension::LENGTH.sqrt(), None);
        assert_eq!(Dimension::ACCELERATION.sqrt(), None);
        assert_eq!(Dimension::ANGLE.sqrt(), None);
    }

    #[test]
    fn arithmetic_saturates_instead_of_overflowing() {
        // A very long product folds many exponent additions. Saturation keeps
        // the exponents at `i8::MAX`.
        let huge = (0..200).fold(Dimension::LENGTH, |acc, _| acc * Dimension::LENGTH);
        assert_eq!(huge.length, i8::MAX);
        assert!(!huge.is_dimensionless());
        // powi and division saturate the same way.
        assert_eq!(Dimension::LENGTH.powi(i8::MAX).length, i8::MAX);
        let tiny = (0..200).fold(Dimension::DIMENSIONLESS, |acc, _| acc / Dimension::LENGTH);
        assert_eq!(tiny.length, i8::MIN);
    }

    mod properties {
        use proptest::prelude::*;

        use super::super::Dimension;

        // Exponents kept small - real dimensions never leave this range, and it
        // keeps the doubled exponents from a square well clear of overflow.
        fn dimension() -> impl Strategy<Value = Dimension> {
            (-4..=4i8, -4..=4i8, -4..=4i8).prop_map(|(length, time, angle)| Dimension {
                length,
                time,
                angle,
            })
        }

        proptest! {
            #[test]
            fn multiplication_commutes(a in dimension(), b in dimension()) {
                prop_assert_eq!(a * b, b * a);
            }

            #[test]
            fn dimensionless_is_the_identity(d in dimension()) {
                prop_assert_eq!(d * Dimension::DIMENSIONLESS, d);
                prop_assert_eq!(d / Dimension::DIMENSIONLESS, d);
            }

            #[test]
            fn division_inverts_multiplication(a in dimension(), b in dimension()) {
                prop_assert_eq!((a * b) / b, a);
            }

            #[test]
            fn square_root_undoes_squaring(d in dimension()) {
                prop_assert_eq!(d.powi(2).sqrt(), Some(d));
            }
        }
    }
}
