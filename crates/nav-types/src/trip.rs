use crate::markers::{CustomMarker, GeneratedMarker};
use crate::nav_point::NavPoint;
use chrono::{DateTime, Duration, Utc};
use geo_types::Rect;

#[derive(Debug, Clone, Copy)]
pub struct TripMetadata {
    pub index: usize,
    pub distance_km: f64,
    pub duration: Duration,
    pub time_range: (DateTime<Utc>, DateTime<Utc>),
    /// Geographic bounding box in (lon, lat) coordinate order per geo-types convention.
    pub bounding_box: Rect<f64>,
    pub point_set_diameter_m: f64,
    pub has_custom_markers: bool,
    pub tpv_count: usize,
    pub satellite_report_count: usize,
    pub custom_marker_count: usize,
    pub generated_marker_count: usize,
}

#[derive(Debug, Clone)]
pub struct LoadedTrip {
    pub metadata: TripMetadata,
    /// TPV points, each optionally paired with a satellite report.
    pub points: Vec<NavPoint>,
    pub custom_markers: Vec<CustomMarker>,
    pub generated_markers: Vec<GeneratedMarker>,
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub filename: String,
    pub total_distance_km: f64,
    pub total_duration: Duration,
    pub time_range: (DateTime<Utc>, DateTime<Utc>),
}

#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub metadata: FileMetadata,
    pub trips: Vec<LoadedTrip>,
}
