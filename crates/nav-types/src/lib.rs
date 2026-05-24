pub mod filter;
pub mod highlight;
pub mod markers;
pub(crate) mod mercator;
pub mod nav_point;
pub mod satellites;
pub mod segment;
pub mod test_data;
pub mod tpv;
pub mod trip;
pub mod visibility;

pub use filter::{GlobalFilter, point_passes_time_filter, trip_passes_filter};
pub use geo_types::{Coord, Rect};
pub use highlight::{DataCategory, DataPointRef, HighlightScope, MapHighlight};
pub use markers::{CustomMarker, GeneratedMarker, GeneratedMarkerKind, MarkerIcon};
pub use nav_point::NavPoint;
pub use tpv::TimePositionVelocity;
pub use tpv::TimePositionVelocityBuilder;
pub use trip::{FileMetadata, LoadedFile, LoadedTrip, TripMetadata};
pub use visibility::{FileVisibility, TripDataVisibility, TripVisibility};

pub use test_data::marker_test_data;
pub use test_data::nav_test_data;
