pub mod coordinates;
pub mod highlight;
pub mod history;
pub mod markers;
pub mod mercator;
pub mod metrics;
pub mod nav_point;
pub mod satellites;
pub use satellites::{Prn, SignalQuality, Snr};
pub mod time_types;
pub mod tpv;
pub mod track;

pub use coordinates::{Latitude, Longitude};
pub use geo_types::{Coord, Rect};
pub use highlight::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};
pub use markers::{
    CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker, GeneratedMarkerKind, MarkerColor,
    MarkerIcon, event_marker_fallback_color,
};
pub use mercator::MercPoint;
pub use metrics::MetricKind;
pub use nav_point::NavPoint;
pub use time_types::{GpsTime, SysTime};
pub use tpv::TimePositionVelocity;
pub use tpv::TimePositionVelocityBuilder;
pub use track::{
    AssociationConfig, DatabaseRef, FileMetadata, FileSource, LoadWarning, LoadedFile, LoadedTrack,
    MarkerRequirement, MercBounds, SpatialPoint, TimeRange, TrackMetadata, merc_bounds_for_rect,
};
