pub mod coordinates;
pub mod event_marker_visibility;
pub mod filter;
pub mod highlight;
pub mod markers;
pub(crate) mod mercator;
pub mod nav_point;
pub mod satellites;
pub use satellites::{Prn, SignalQuality, Snr};
pub mod time_types;
pub mod tpv;
pub mod track;
pub mod visibility;

pub use coordinates::{Latitude, Longitude};
pub use event_marker_visibility::EventMarkerVisibility;
pub use filter::{GlobalFilter, point_passes_time_filter, track_passes_filter};
pub use geo_types::{Coord, Rect};
pub use highlight::{
    DataCategory, DataPointRef, FileIdx, HighlightScope, MapHighlight, PointIdx, TrackIdx,
};
pub use markers::{
    CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker, GeneratedMarkerKind, MarkerIcon,
    event_marker_fallback_color,
};
pub use nav_point::NavPoint;
pub use time_types::{GpsTime, SysTime};
pub use tpv::TimePositionVelocity;
pub use tpv::TimePositionVelocityBuilder;
pub use track::{
    FileMetadata, LoadedFile, LoadedTrack, MarkerRequirement, MercBounds, SpatialPoint, TimeRange,
    TrackMetadata, merc_bounds_for_rect,
};
pub use visibility::{FileVisibility, TrackDataVisibility, TrackVisibility};
