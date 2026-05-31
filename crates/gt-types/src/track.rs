use crate::highlight::{DataCategory, FileIdx, PointIdx, TrackIdx};
use crate::markers::{CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker};
use crate::nav_point::NavPoint;
use chrono::{DateTime, Duration, Utc};
use geo_types::Rect;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Normalised Web Mercator bounding box, with all values in `[0.0, 1.0]`.
///
/// Mercator Y increases south (0 = north pole, 1 = south pole), so `y_min`
/// corresponds to the northernmost latitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MercBounds {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl MercBounds {
    /// Returns `true` when `self` overlaps `viewport` in both axes.
    pub fn intersects(self, viewport: MercBounds) -> bool {
        self.x_max >= viewport.x_min
            && self.x_min <= viewport.x_max
            && self.y_max >= viewport.y_min
            && self.y_min <= viewport.y_max
    }
}

/// A closed time interval `[start, end]` with named fields.
///
/// Replaces raw `(DateTime<Utc>, DateTime<Utc>)` tuples so that `start`/`end`
/// are self-documenting and a swapped pair becomes a compile error.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self { start, end }
    }

    pub fn duration(&self) -> Duration {
        self.end - self.start
    }

    /// Returns `true` when `self` overlaps the optional `[window_start, window_end]` window.
    ///
    /// An absent bound is treated as unbounded (−∞ or +∞ respectively), so a
    /// fully absent window matches every range.
    pub fn overlaps_window(
        self,
        window_start: Option<DateTime<Utc>>,
        window_end: Option<DateTime<Utc>>,
    ) -> bool {
        if let Some(start) = window_start
            && self.end < start
        {
            return false;
        }
        if let Some(end) = window_end
            && self.start > end
        {
            return false;
        }
        true
    }
}

/// Which marker types a trip must have to pass the marker filter.
///
/// The three variants are mutually exclusive; `CustomMarker` is a strict
/// subset of `AnyMarker`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkerRequirement {
    /// No marker constraint — all trips pass.
    #[default]
    None,
    /// Trip must have at least one custom *or* generated marker.
    AnyMarker,
    /// Trip must have at least one *custom* marker.
    CustomMarker,
}

#[derive(Debug, Clone, Copy)]
pub struct TrackMetadata {
    pub index: usize,
    pub distance_km: f64,
    pub duration: Duration,
    pub time_range: TimeRange,
    /// Geographic bounding box in (lon, lat) coordinate order per geo-types convention.
    pub bounding_box: Rect<f64>,
    /// Normalised Web Mercator bounding box, pre-computed from `bounding_box`.
    /// Used by map renderers for O(1) viewport intersection tests without trigonometry.
    pub merc_bounds: MercBounds,
    pub point_set_diameter_m: f64,
    pub has_custom_markers: bool,
    pub tpv_count: usize,
    pub satellite_report_count: usize,
    pub custom_marker_count: usize,
    pub generated_marker_count: usize,
    pub event_marker_count: usize,
}

impl TrackMetadata {
    /// Returns `true` when the trip has at least one custom, event, or generated marker.
    pub fn has_any_marker(&self) -> bool {
        self.has_custom_markers || self.generated_marker_count > 0 || self.event_marker_count > 0
    }
}

/// Compute the normalised Web Mercator bounding box for a geographic rectangle.
///
/// The input `Rect` uses (lon, lat) coordinate order per `geo_types` convention.
///
/// Mercator Y increases south (0 = north pole, 1 = south pole), so the
/// northernmost latitude (`bb.max().y`) maps to `y_min`.
pub fn merc_bounds_for_rect(bb: Rect<f64>) -> MercBounds {
    let (x_min, y_min) = crate::mercator::normalize(bb.min().x, bb.max().y);
    let (x_max, y_max) = crate::mercator::normalize(bb.max().x, bb.min().y);
    MercBounds {
        x_min,
        x_max,
        y_min,
        y_max,
    }
}

/// A point in the global spatial index, covering TPV fixes and all marker categories.
///
/// Ghost TPV fixes (heading == `None`) are excluded — their position is interpolated
/// at render time rather than pre-computed.
#[derive(Debug, Clone, Copy)]
pub struct SpatialPoint {
    pub merc_x: f64,
    pub merc_y: f64,
    pub file_index: FileIdx,
    pub track_index: TrackIdx,
    pub point_index: PointIdx,
    pub category: DataCategory,
}

impl rstar::RTreeObject for SpatialPoint {
    type Envelope = rstar::AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        rstar::AABB::from_point([self.merc_x, self.merc_y])
    }
}

impl rstar::PointDistance for SpatialPoint {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        let dx = self.merc_x - point[0];
        let dy = self.merc_y - point[1];
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone)]
pub struct LoadedTrack {
    pub metadata: TrackMetadata,
    /// TPV points, each optionally paired with a satellite report.
    pub points: Vec<NavPoint>,
    pub custom_markers: Vec<CustomMarker>,
    pub generated_markers: Vec<GeneratedMarker>,
    pub event_markers: Vec<EventMarker>,
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub filename: String,
    pub total_distance_km: f64,
    pub total_duration: Duration,
    pub time_range: TimeRange,
}

/// Configuration for log-marker and satellite association.
///
/// Stored in `Settings` and persisted to the config file; also carried on
/// `LoadedFile` so that re-processing knows which window was last used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AssociationConfig {
    /// Max seconds between a log-file timestamp and the nearest GPS fix for the
    /// entry to be associated (placed on the map). Entries further away are
    /// listed as "unassociated." Default: 60 s.
    pub log_marker_window_s: u64,
}

impl Default for AssociationConfig {
    fn default() -> Self {
        Self {
            log_marker_window_s: 60,
        }
    }
}

/// Where the file content came from; stored on [`LoadedFile`] to enable
/// re-processing when association settings change.
#[derive(Debug, Clone)]
pub enum FileSource {
    /// Loaded from a path on disk (NVD file).
    NvdPath(PathBuf),
    /// Loaded from bytes delivered via drag-and-drop (NVD file).
    NvdBytes(Arc<[u8]>),
    /// Loaded from a log file on disk.
    LogPath(PathBuf),
    /// Loaded from in-memory log text (e.g. dropped bytes decoded to UTF-8).
    LogText(Arc<str>),
}

#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub metadata: FileMetadata,
    pub tracks: Vec<LoadedTrack>,
    /// Icon/color overrides keyed by variant path; file-level (shared across trips).
    pub event_marker_styles: HashMap<String, EventMarkerStyle>,
    /// Event markers whose timestamp did not fall within any trip's time window.
    pub orphaned_event_markers: Vec<EventMarker>,
    /// Where this file was loaded from; used to re-process when settings change.
    pub source: FileSource,
}
