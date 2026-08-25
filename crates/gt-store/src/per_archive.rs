//! One value per day archive, indexed by [`EnvironmentArchive`].

use std::ops::{Index, IndexMut};

use strum::EnumCount as _;

use crate::{ArchiveUsage, EnvironmentArchive};

/// One `T` for each of the four archives, indexed by [`EnvironmentArchive`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PerArchive<T>([T; EnvironmentArchive::COUNT]);

impl<T: Copy> PerArchive<T> {
    /// `value` for every archive.
    pub const fn filled_with(value: T) -> Self {
        Self([value; EnvironmentArchive::COUNT])
    }
}

impl<T> PerArchive<T> {
    /// The values in the order [`EnvironmentArchive`] declares its variants,
    /// which is the order the settings rows list the archives in.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.0.iter()
    }
}

impl PerArchive<usize> {
    pub fn total(&self) -> usize {
        self.values().sum()
    }
}

impl PerArchive<Option<ArchiveUsage>> {
    /// Every archive that opened added up, or [`None`] where none of them
    /// opened.
    pub fn total(&self) -> Option<ArchiveUsage> {
        let opened: Vec<ArchiveUsage> = self.values().copied().flatten().collect();
        (!opened.is_empty()).then(|| ArchiveUsage::total(opened))
    }
}

#[expect(
    clippy::indexing_slicing,
    reason = "the array holds one element per variant, and the discriminants of a fieldless enum \
              run from 0 to COUNT - 1"
)]
impl<T> Index<EnvironmentArchive> for PerArchive<T> {
    type Output = T;

    fn index(&self, archive: EnvironmentArchive) -> &T {
        &self.0[archive as usize]
    }
}

#[expect(
    clippy::indexing_slicing,
    reason = "the array holds one element per variant, and the discriminants of a fieldless enum \
              run from 0 to COUNT - 1"
)]
impl<T> IndexMut<EnvironmentArchive> for PerArchive<T> {
    fn index_mut(&mut self, archive: EnvironmentArchive) -> &mut T {
        &mut self.0[archive as usize]
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator as _;

    use super::*;

    #[test]
    fn each_archive_indexes_a_slot_of_its_own() {
        let mut counts = PerArchive::<usize>::default();
        for (position, archive) in EnvironmentArchive::iter().enumerate() {
            counts[archive] = position + 1;
        }

        assert_eq!(
            EnvironmentArchive::iter()
                .map(|archive| counts[archive])
                .collect::<Vec<usize>>(),
            (1..=EnvironmentArchive::COUNT).collect::<Vec<usize>>()
        );
        assert_eq!(counts.total(), 10);
    }

    #[test]
    fn values_follow_the_order_the_variants_are_declared_in() {
        let mut plans = PerArchive::filled_with("unasked");
        plans[EnvironmentArchive::IonosphericTec] = "asked";

        assert_eq!(
            plans.values().copied().collect::<Vec<&str>>(),
            ["unasked", "unasked", "asked", "unasked"]
        );
    }
}
