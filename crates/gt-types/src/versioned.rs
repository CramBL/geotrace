//! A value paired with a revision of itself, for caches over data too large to
//! compare on every frame.

use std::sync::atomic::{AtomicU64, Ordering};

/// A revision, taken from a process-wide counter that hands out every value
/// once. The counter is in gt-types so that a cache over the recording names
/// or over the environment series can key on the same counter as the one over
/// the loaded files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generation(u64);

impl Generation {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A value that takes a new [`Generation`] on every mutable borrow.
///
/// [`Versioned::get_mut`] is the only path to a `&mut T`, and it takes the new
/// generation on the call itself, before the caller writes through the
/// reference. A cache keyed on [`Versioned::generation`] therefore recomputes
/// after a mutable borrow that wrote nothing, and after every write. A write
/// through interior mutability inside `T` needs no mutable borrow and leaves
/// the generation as it was.
///
/// A clone keeps the generation of the value it copied. Two values with equal
/// generations have equal contents where `T` has no interior mutability.
#[derive(Debug, Clone)]
pub struct Versioned<T> {
    inner: T,
    generation: Generation,
}

impl<T> Versioned<T> {
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

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: Default> Default for Versioned<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::Versioned;

    #[test]
    fn a_read_leaves_the_generation_as_it_was() {
        let versioned = Versioned::new(vec![1, 2, 3]);
        let before = versioned.generation();

        assert_eq!(versioned.get().len(), 3);

        assert_eq!(versioned.generation(), before);
    }

    /// A mutable borrow that writes nothing takes a new generation too:
    /// [`Versioned::get_mut`] changes it on the call itself.
    #[test]
    fn every_mutable_borrow_takes_a_new_generation() {
        let mut versioned = Versioned::new(vec![1, 2, 3]);
        let before = versioned.generation();

        versioned.get_mut().push(4);
        let after_the_write = versioned.generation();
        versioned.get_mut();

        assert_ne!(after_the_write, before);
        assert_ne!(versioned.generation(), after_the_write);
    }

    #[test]
    fn a_clone_keeps_the_generation_of_the_value_it_copied() {
        let versioned = Versioned::new(vec![1, 2, 3]);

        assert_eq!(versioned.clone().generation(), versioned.generation());
    }

    #[test]
    fn two_default_values_have_different_generations() {
        assert_ne!(
            Versioned::<Vec<u8>>::default().generation(),
            Versioned::<Vec<u8>>::default().generation()
        );
    }
}
