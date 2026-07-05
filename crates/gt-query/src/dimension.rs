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
//! The zero-exponent (dimensionless) case still hides a distinction the
//! language cares about - a count is not a ratio is not a bare number - which a
//! later kind tag will carry alongside the exponents. This type is the exponent
//! arithmetic only; nothing consumes it yet (the checker is routed through it in
//! a following change).

use std::ops::{Div, Mul};

/// A physical dimension: exponents of length, time, and angle.
///
/// The fields are `i32`, which gives ample headroom for exponents that stay
/// within a handful of units. Guarding a pathological integer power is the
/// power operator's job (it bounds the exponent), not this type's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Dimension {
    pub length: i32,
    pub time: i32,
    pub angle: i32,
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

    const fn new(length: i32, time: i32, angle: i32) -> Dimension {
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
    /// A zeroth power is dimensionless; a negative power inverts.
    pub fn powi(self, n: i32) -> Dimension {
        Dimension {
            length: self.length * n,
            time: self.time * n,
            angle: self.angle * n,
        }
    }

    /// Square root: exponents halve, so `sqrt(L² T⁻²) = speed`. `None` when any
    /// exponent is odd, since a whole-number dimension has no square root then
    /// (e.g. `sqrt(length)` is not expressible).
    pub fn sqrt(self) -> Option<Dimension> {
        let even = |e: i32| e % 2 == 0;
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
            length: self.length + other.length,
            time: self.time + other.time,
            angle: self.angle + other.angle,
        }
    }
}

/// Quotient of two dimensions: their exponents subtract, so
/// `length / time = speed`.
impl Div for Dimension {
    type Output = Dimension;

    fn div(self, other: Dimension) -> Dimension {
        Dimension {
            length: self.length - other.length,
            time: self.time - other.time,
            angle: self.angle - other.angle,
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

    mod properties {
        use proptest::prelude::*;

        use super::super::Dimension;

        // Exponents kept small - real dimensions never leave this range, and it
        // keeps the doubled exponents from a square well clear of overflow.
        fn dimension() -> impl Strategy<Value = Dimension> {
            (-4..=4i32, -4..=4i32, -4..=4i32).prop_map(|(length, time, angle)| Dimension {
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
