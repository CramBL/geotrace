use std::ptr;
use std::sync::{Arc, Weak};

/// The identity of an [`Arc`] allocation, for cache invalidation: two
/// [`ArcIdentity`] values are equal exactly when they were taken from
/// clones of the same allocation.
///
/// Comparisons are only meaningful while the source `Arc` is alive - a
/// freed allocation's address can be reused. The snap caches satisfy this
/// by construction: the identity is stored next to data derived from the
/// same `Arc`, and both are replaced together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ArcIdentity(usize);

impl ArcIdentity {
    pub fn of<T>(arc: &Arc<T>) -> Self {
        Self(Arc::as_ptr(arc) as usize)
    }
}

/// An allocation identity that remains valid after the value drops.
#[derive(Debug)]
pub struct WeakArcIdentity<T>(Weak<T>);

impl<T> WeakArcIdentity<T> {
    pub fn of(arc: &Arc<T>) -> Self {
        Self(Arc::downgrade(arc))
    }

    pub fn matches(&self, arc: &Arc<T>) -> bool {
        ptr::eq(self.0.as_ptr(), Arc::as_ptr(arc))
    }
}

impl<T> Clone for WeakArcIdentity<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> PartialEq for WeakArcIdentity<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl<T> Eq for WeakArcIdentity<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clones of one allocation share an identity. A distinct allocation with
    /// equal contents has its own.
    #[test]
    fn identity_follows_the_allocation_not_the_value() {
        let a = Arc::new(7);
        let clone = Arc::clone(&a);
        let b = Arc::new(7);
        assert_eq!(ArcIdentity::of(&a), ArcIdentity::of(&clone));
        assert_ne!(ArcIdentity::of(&a), ArcIdentity::of(&b));
    }
}
