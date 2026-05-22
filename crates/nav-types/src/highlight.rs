#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataCategory {
    /// Rendered as a polyline through all TPV points; no individual point refs.
    TripTrack,
    Tpv,
    SatelliteReport,
    CustomMarker,
    GeneratedMarker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataPointRef {
    pub file_index: usize,
    pub trip_index: usize,
    pub category: DataCategory,
    pub point_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightScope {
    File {
        file_index: usize,
    },
    Trip {
        file_index: usize,
        trip_index: usize,
    },
    TripCategory {
        file_index: usize,
        trip_index: usize,
        category: DataCategory,
    },
    Point(DataPointRef),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MapHighlight {
    pub hover: Option<HighlightScope>,
    pub sticky: Option<DataPointRef>,
}
