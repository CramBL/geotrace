//! A change counter for cached derivations.
//!
//! The owner of the data keeps it in a [`Versioned<T>`]. A consumer that derives
//! a value from that data keeps the [`Generation<T>`] it read last and
//! recomputes when the owner's generation differs from it.

use std::{
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    sync::atomic::{AtomicU64, Ordering},
};

/// The inner type of a [`Versioned`].
///
/// Implement this for the type an owner keeps in a [`Versioned`], one type per
/// kind of cached derivation. Wrap a foreign type in a newtype first, so two
/// owners of the same data type have generations of different types.
pub trait Versionable {}

/// The change counter of a [`Versioned<T>`].
///
/// Two generations are equal only when one is a copy of the other: every value
/// comes from one process-wide counter.
///
/// `Generation<A>` and `Generation<B>` are distinct types, and comparing them is
/// a compile error.
pub struct Generation<T: Versionable> {
    count: u64,
    versioned_type: PhantomData<fn() -> T>,
}

impl<T: Versionable> Generation<T> {
    fn next() -> Self {
        static NEXT_COUNT: AtomicU64 = AtomicU64::new(0);
        Self {
            count: NEXT_COUNT.fetch_add(1, Ordering::Relaxed),
            versioned_type: PhantomData,
        }
    }
}

// Each of the following impls is written out because a derive would add a bound
// on `T` for the trait it implements, which none of them reads.
impl<T: Versionable> Clone for Generation<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Versionable> Copy for Generation<T> {}

impl<T: Versionable> PartialEq for Generation<T> {
    fn eq(&self, other: &Self) -> bool {
        self.count == other.count
    }
}

impl<T: Versionable> Eq for Generation<T> {}

impl<T: Versionable> Hash for Generation<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.count.hash(state);
    }
}

impl<T: Versionable> fmt::Debug for Generation<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Generation({})", self.count)
    }
}

/// A value that takes a new [`Generation`] on every mutable borrow.
///
/// [`Versioned::get_mut`] is the only path to a `&mut T`, and it takes the new
/// generation on the call itself. A mutable borrow that writes nothing changes
/// the generation too. A write through interior mutability inside `T` needs no
/// mutable borrow and leaves the generation as it was.
#[derive(Debug, Clone)]
pub struct Versioned<T: Versionable> {
    inner: T,
    generation: Generation<T>,
}

impl<T: Versionable> Versioned<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            generation: Generation::next(),
        }
    }

    pub fn get(&self) -> &T {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.generation = Generation::next();
        &mut self.inner
    }

    pub fn generation(&self) -> Generation<T> {
        self.generation
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: Versionable + Default> Default for Versioned<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[cfg(test)]
mod tests {
    use crate::versioned::{Versionable, Versioned};

    #[derive(Clone, Debug, Default)]
    struct Counter(u32);

    impl Versionable for Counter {}

    #[test]
    fn a_read_leaves_the_generation_as_it_was() {
        let versioned = Versioned::new(Counter(1));
        let before = versioned.generation();

        assert_eq!(versioned.get().0, 1);

        assert_eq!(versioned.generation(), before);
    }

    #[test]
    fn every_mutable_borrow_takes_a_new_generation() {
        let mut versioned = Versioned::new(Counter(1));
        let before = versioned.generation();

        versioned.get_mut().0 = 2;
        let after_the_write = versioned.generation();
        versioned.get_mut();

        assert_ne!(after_the_write, before);
        assert_ne!(versioned.generation(), after_the_write);
    }

    #[test]
    fn cloning_a_versioned_value_keeps_its_generation() {
        let versioned = Versioned::new(Counter(1));

        assert_eq!(versioned.clone().generation(), versioned.generation());
    }

    #[test]
    fn two_values_of_the_same_type_have_different_generations() {
        assert_ne!(
            Versioned::new(Counter(1)).generation(),
            Versioned::new(Counter(1)).generation()
        );
    }

    #[test]
    fn a_default_versioned_holds_the_default_of_its_inner_type() {
        assert_eq!(
            Versioned::<Counter>::default().get().0,
            Counter::default().0
        );
    }
}
