use std::fmt;

use strum::{EnumCount as _, IntoEnumIterator as _};

/// Typed wrapper for a file index into `loaded_files[fi]`.
///
/// Using a newtype prevents accidentally swapping a file index for a track index.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FileIdx(usize);

impl FileIdx {
    pub fn new(n: usize) -> Self {
        Self(n)
    }

    pub fn as_usize(self) -> usize {
        self.0
    }

    pub fn get<T>(self, slice: &[T]) -> Option<&T> {
        slice.get(self.0)
    }

    pub fn get_mut<T>(self, slice: &mut [T]) -> Option<&mut T> {
        slice.get_mut(self.0)
    }
}

impl fmt::Display for FileIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Typed wrapper for a track index into `loaded_files[fi].tracks[ti]`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TrackIdx(usize);

impl TrackIdx {
    pub fn new(n: usize) -> Self {
        Self(n)
    }

    pub fn as_usize(self) -> usize {
        self.0
    }

    pub fn get<T>(self, slice: &[T]) -> Option<&T> {
        slice.get(self.0)
    }

    pub fn get_mut<T>(self, slice: &mut [T]) -> Option<&mut T> {
        slice.get_mut(self.0)
    }
}

impl fmt::Display for TrackIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Typed wrapper for a point index into `loaded_files[fi].tracks[ti].points[pi]`.
///
/// Serializes as the bare index (`transparent`): point indices appear in
/// persisted snap results, where a wrapper object would only add noise.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct PointIdx(usize);

impl PointIdx {
    pub fn new(n: usize) -> Self {
        Self(n)
    }

    pub fn as_usize(self) -> usize {
        self.0
    }

    pub fn get<T>(self, slice: &[T]) -> Option<&T> {
        slice.get(self.0)
    }

    pub fn get_mut<T>(self, slice: &mut [T]) -> Option<&mut T> {
        slice.get_mut(self.0)
    }
}

impl fmt::Display for PointIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Canonical address of a single track: which file and which track within it.
///
/// Replaces the loose `(fi: FileIdx, ti: TrackIdx)` pair that appeared in
/// every function signature that needed to identify a track.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TrackRef {
    pub fi: FileIdx,
    pub index: TrackIdx,
}

impl TrackRef {
    pub fn new(fi: FileIdx, index: TrackIdx) -> Self {
        Self { fi, index }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, strum::EnumIter, strum::EnumCount,
)]
pub enum DataCategory {
    /// Rendered as a polyline through all TPV points. No individual point refs.
    Track,
    Tpv,
    SatelliteReport,
    CustomMarker,
    GeneratedMarker,
    EventMarker,
}

impl DataCategory {
    /// Index into `MapHighlight::hover_candidates` for this category.
    /// Returns `None` for categories that don't participate in multi-hover.
    pub fn hover_slot(self) -> Option<usize> {
        match self {
            Self::Tpv | Self::SatelliteReport => Some(0),
            Self::EventMarker => Some(1),
            Self::CustomMarker => Some(2),
            Self::GeneratedMarker => Some(3),
            Self::Track => None,
        }
    }

    /// The category's bit in a [`DataCategorySet`].
    const fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

const _: () = assert!(
    DataCategory::COUNT <= u8::BITS as usize,
    "DataCategorySet stores one bit per category in a u8"
);

/// A set of [`DataCategory`]s, one bit each - a `Copy` stand-in for a set of
/// per-category booleans, used for per-track element visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DataCategorySet(u8);

impl DataCategorySet {
    /// The empty set.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Every category.
    pub fn all() -> Self {
        DataCategory::iter().fold(Self::empty(), |set, c| set.with(c))
    }

    /// `self` plus `category`.
    pub const fn with(self, category: DataCategory) -> Self {
        Self(self.0 | category.bit())
    }

    /// Adds or removes `category` per `present`.
    pub const fn set(&mut self, category: DataCategory, present: bool) {
        if present {
            self.0 |= category.bit();
        } else {
            self.0 &= !category.bit();
        }
    }

    /// Whether `category` is in the set.
    pub const fn contains(self, category: DataCategory) -> bool {
        self.0 & category.bit() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::{DataCategory, DataCategorySet};
    use strum::{EnumCount as _, IntoEnumIterator as _};

    #[test]
    fn all_contains_every_category_empty_contains_none() {
        let all = DataCategorySet::all();
        let empty = DataCategorySet::empty();
        assert!(DataCategory::iter().all(|c| all.contains(c)));
        assert!(DataCategory::iter().all(|c| !empty.contains(c)));
    }

    #[test]
    fn set_toggles_exactly_one_category() {
        let mut set = DataCategorySet::all();
        set.set(DataCategory::Tpv, false);
        assert!(!set.contains(DataCategory::Tpv));
        assert!(
            DataCategory::iter()
                .filter(|&c| c != DataCategory::Tpv)
                .all(|c| set.contains(c))
        );
        set.set(DataCategory::Tpv, true);
        assert!(set.contains(DataCategory::Tpv));
    }

    #[test]
    fn every_category_has_a_distinct_bit() {
        let mut seen = 0u8;
        for category in DataCategory::iter() {
            let bit = category.bit();
            assert_eq!(bit & seen, 0, "bit collision for {category:?}");
            seen |= bit;
        }
        assert_eq!(seen.count_ones() as usize, DataCategory::COUNT);
    }
}
