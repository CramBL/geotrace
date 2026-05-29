/// Typed wrapper for a file index into `loaded_files[fi]`.
///
/// Using a newtype prevents accidentally swapping a file index for a trip index.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FileIdx(pub usize);

/// Typed wrapper for a trip index into `loaded_files[fi].trips[ti]`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TripIdx(pub usize);

/// Typed wrapper for a point index into `loaded_files[fi].trips[ti].points[pi]`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PointIdx(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataCategory {
    /// Rendered as a polyline through all TPV points; no individual point refs.
    TripTrack,
    Tpv,
    SatelliteReport,
    CustomMarker,
    GeneratedMarker,
    EventMarker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataPointRef {
    pub file_index: FileIdx,
    pub trip_index: TripIdx,
    pub category: DataCategory,
    pub point_index: PointIdx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightScope {
    File {
        file_index: FileIdx,
    },
    Trip {
        file_index: FileIdx,
        trip_index: TripIdx,
    },
    TripCategory {
        file_index: FileIdx,
        trip_index: TripIdx,
        category: DataCategory,
    },
    Point(DataPointRef),
}

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, Default)]
pub struct MapHighlight {
    pub hover: Option<HighlightScope>,
    pub sticky: Option<DataPointRef>,
    /// Time currently hovered on the trip plot; used to cross-highlight the
    /// closest TPV point on the map.  `None` when the plot cursor is inactive.
    pub plot_hover_time: Option<DateTime<Utc>>,
    /// Pre-computed `(FileIdx, TripIdx, PointIdx)` of the TPV point closest to
    /// `plot_hover_time`, set by the app layer alongside that field.
    /// `TpvRenderer` reads this directly instead of re-scanning all points.
    /// `None` when `plot_hover_time` is `None`.
    pub plot_hover_point: Option<(FileIdx, TripIdx, PointIdx)>,
}
