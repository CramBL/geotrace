//! [`enum_bitset`]: generate a `Copy` set type backed by one bit per variant
//! of a fieldless `enum`.
//!
//! The storage integer is a parameter, so a 4- or 6-variant set can stay a `u8`
//! while a 33-variant set uses a `u64`.
//!
//! The macro is `#[macro_export]`ed on purpose - it is shared across crates
//! (e.g. gt-plot builds its metric set on it), unlike the otherwise-private
//! `bitset` module. Treat it as public API.

/// Define a bitset newtype `$set($storage)` holding one bit per variant of
/// `$elem`, a fieldless `enum` which must derive [`strum::EnumCount`] and
/// [`strum::EnumIter`]. A `const` assert checks the variants fit the storage.
///
/// `$elem` must keep the default sequential discriminants (`0, 1, 2, …`) that
/// match its declaration and iteration order: the bit for a variant is
/// `1 << (variant as $storage)`, so an explicit discriminant (`Foo = 4`) would
/// shift bits apart, waste storage, or - past the storage width - collide.
///
/// ```ignore
/// enum_bitset! {
///     /// Doc comment for the generated type.
///     pub struct ConstellationSet(u8) for Constellation;
/// }
/// ```
#[macro_export]
macro_rules! enum_bitset {
    (
        $(#[$meta:meta])*
        $vis:vis struct $set:ident($storage:ty) for $elem:ty;
    ) => {
        const _: () = assert!(
            <$elem as ::strum::EnumCount>::COUNT <= <$storage>::BITS as usize,
            concat!(
                stringify!($set),
                " stores one bit per variant of ",
                stringify!($elem),
                "; the storage integer is too small",
            ),
        );

        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        $vis struct $set($storage);

        impl $set {
            const fn bit(variant: $elem) -> $storage {
                1 << (variant as $storage)
            }

            pub const fn empty() -> Self {
                Self(0)
            }

            pub fn all() -> Self {
                use ::strum::IntoEnumIterator as _;
                <$elem>::iter().fold(Self::empty(), Self::with)
            }

            pub const fn single(variant: $elem) -> Self {
                Self(Self::bit(variant))
            }

            pub const fn with(self, variant: $elem) -> Self {
                Self(self.0 | Self::bit(variant))
            }

            pub const fn insert(&mut self, variant: $elem) {
                self.0 |= Self::bit(variant);
            }

            pub const fn remove(&mut self, variant: $elem) {
                self.0 &= !Self::bit(variant);
            }

            pub const fn set(&mut self, variant: $elem, present: bool) {
                if present {
                    self.insert(variant);
                } else {
                    self.remove(variant);
                }
            }

            pub const fn union(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            pub const fn contains(self, variant: $elem) -> bool {
                self.0 & Self::bit(variant) != 0
            }

            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }
        }

        impl ::core::iter::FromIterator<$elem> for $set {
            fn from_iter<I: ::core::iter::IntoIterator<Item = $elem>>(iter: I) -> Self {
                iter.into_iter().fold(Self::empty(), |mut set, variant| {
                    set.insert(variant);
                    set
                })
            }
        }
    };
}
