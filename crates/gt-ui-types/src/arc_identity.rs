use std::sync::Arc;

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
