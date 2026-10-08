use std::fmt;

/// Typed wrapper for a file index into `loaded_files[fi]`.
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

macro_rules! marker_index {
    ($name:ident, $element:ty) => {
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(usize);

        impl $name {
            pub fn new(n: usize) -> Self {
                Self(n)
            }

            pub fn as_usize(self) -> usize {
                self.0
            }

            pub fn get(self, slice: &[$element]) -> Option<&$element> {
                slice.get(self.0)
            }

            pub fn get_mut(self, slice: &mut [$element]) -> Option<&mut $element> {
                slice.get_mut(self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

marker_index!(CustomMarkerIdx, crate::markers::CustomMarker);
marker_index!(GeneratedMarkerIdx, crate::markers::GeneratedMarker);
marker_index!(EventMarkerIdx, crate::markers::EventMarker);

/// Canonical address of a single track: which file and which track within it.
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

/// Canonical address of a single recorded fix: which track and which of its
/// points.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FixRef {
    pub track: TrackRef,
    pub point: PointIdx,
}

impl FixRef {
    pub fn new(track: TrackRef, point: PointIdx) -> Self {
        Self { track, point }
    }
}

macro_rules! marker_ref {
    ($name:ident, $index:ident) => {
        #[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name {
            pub track: TrackRef,
            pub index: $index,
        }

        impl $name {
            pub fn new(track: TrackRef, index: $index) -> Self {
                Self { track, index }
            }
        }
    };
}

marker_ref!(CustomMarkerRef, CustomMarkerIdx);
marker_ref!(GeneratedMarkerRef, GeneratedMarkerIdx);
marker_ref!(EventMarkerRef, EventMarkerIdx);

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

crate::enum_bitset! {
    /// Per-track element-visibility set.
    pub struct DataCategorySet(u8) for DataCategory;
}
