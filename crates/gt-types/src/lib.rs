pub mod event_marker_visibility;
pub mod filter;
pub mod highlight;
pub mod markers;
pub(crate) mod mercator;
pub mod nav_point;
pub mod satellites;
pub mod segment;
pub mod test_data;
pub mod time_types;
pub mod tpv;
pub mod trip;
pub mod visibility;

pub use event_marker_visibility::EventMarkerVisibility;
pub use filter::{GlobalFilter, point_passes_time_filter, trip_passes_filter};
pub use geo_types::{Coord, Rect};
pub use highlight::{
    DataCategory, DataPointRef, FileIdx, HighlightScope, MapHighlight, PointIdx, TripIdx,
};
pub use markers::{
    CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker, GeneratedMarkerKind, MarkerIcon,
    event_marker_fallback_color,
};
pub use nav_point::NavPoint;
pub use time_types::{GpsTime, SysTime};
pub use tpv::TimePositionVelocity;
pub use tpv::TimePositionVelocityBuilder;
pub use trip::{
    FileMetadata, LoadedFile, LoadedTrip, MarkerRequirement, MercBounds, TimeRange, TripMetadata,
    merc_bounds_for_rect,
};
pub use visibility::{FileVisibility, TripDataVisibility, TripVisibility};

pub use test_data::marker_test_data;
pub use test_data::nav_test_data;
