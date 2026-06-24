use std::fmt;

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
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
}
