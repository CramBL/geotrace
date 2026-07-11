use crate::channel::Channel;
use crate::highlight::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};
use crate::markers::{CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker};
use crate::mercator::MercPoint;
use crate::nav_point::NavPoint;
use crate::sat_label::SatLabelAnchor;
use chrono::{DateTime, Duration, Utc};
use geo_types::{Coord, Rect};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

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

/// Which marker types a track must have to pass the marker filter.
///
/// The three variants are mutually exclusive. `CustomMarker` is a strict
/// subset of `AnyMarker`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkerRequirement {
    /// No marker constraint - all tracks pass.
    #[default]
    None,
    /// Trip must have at least one custom *or* generated marker.
    AnyMarker,
    /// Trip must have at least one *custom* marker.
    CustomMarker,
}

/// Aggregate GNSS fix-quality statistics computed from satellite reports.
///
/// Covers only intervals between consecutive satellite-report points. Periods
/// without satellite data do not contribute to any field.
/// `None` on a track or file means no satellite reports were present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixStats {
    /// Total elapsed time while `fix_count > 0` in satellite reports.
    pub time_with_fix: Duration,
    /// Total elapsed time while `fix_count == 0` in satellite reports.
    pub time_without_fix: Duration,
    /// Number of transitions from fix to no-fix.
    pub fix_loss_count: u32,
    /// Longest contiguous stretch with `fix_count == 0`.
    pub max_continuous_no_fix: Duration,
}

/// Great-circle length range of a track's consecutive-fix segments, in the
/// order the fixes were recorded. Lets renderers reason O(1) about the whole
/// track's on-screen fix spacing: at a given map scale, every spacing lies
/// between `min` and `max` scaled by pixels-per-metre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentLengthRange {
    pub min: Length,
    pub max: Length,
}

/// Mercator-space radial tolerance of a track LOD's finest stored level
/// before the per-track level offset is applied: ≈ 0.6 m at the equator.
/// Stored level `i` of a [`TrackLod`] with offset `e` has tolerance
/// `LOD_BASE_TOLERANCE_MERC × 2^(e + i)`.
pub const LOD_BASE_TOLERANCE_MERC: f64 = 1.0 / (1u64 << 26) as f64;

/// Multi-resolution decimation of a track's points for rendering.
///
/// Each level holds the indices (into `LoadedTrack::points`) that survive a
/// Mercator-space radial-distance filter: a point is kept when it is at
/// least the level's tolerance away from the previously kept point, or when
/// its [`NavPoint::render_class`] differs (so ghost stretches and
/// fix-quality transitions can never be erased by downsampling). The first
/// and last point always survive.
///
/// Renderers call [`TrackLod::select`] with the current map scale to get
/// the coarsest level whose tolerance is still sub-pixel, bounding per-frame
/// iteration by on-screen detail instead of recording size. Mercator space
/// is used because screen space is a pure scaling of it: a tolerance checked
/// in Mercator units holds exactly in pixels at every latitude.
#[derive(Debug, Clone, Default)]
pub struct TrackLod {
    /// Tolerance exponent of `levels[0]`. Level `i` has tolerance
    /// `LOD_BASE_TOLERANCE_MERC × 2^(first_level_exp + i)`. Lets sparse
    /// recordings skip storing fine levels that would not drop any points.
    first_level_exp: u32,
    levels: Vec<Vec<u32>>,
}

impl TrackLod {
    pub fn new(first_level_exp: u32, levels: Vec<Vec<u32>>) -> Self {
        Self {
            first_level_exp,
            levels,
        }
    }

    /// Tolerance, in Mercator units, of stored level `i`. An exponent
    /// overflow (impossible for the level counts the builder produces)
    /// yields an infinite tolerance, which `select` never accepts.
    fn level_tolerance_merc(&self, i: usize) -> f64 {
        u32::try_from(i)
            .ok()
            .and_then(|i| self.first_level_exp.checked_add(i))
            .and_then(|exp| i32::try_from(exp).ok())
            .map_or(f64::INFINITY, |exp| {
                LOD_BASE_TOLERANCE_MERC * 2_f64.powi(exp)
            })
    }

    /// The coarsest level whose decimation error stays below `max_error_px`
    /// at the given scale (`px_per_merc` = pixels per Mercator unit, i.e.
    /// the world's width in pixels). `None` when no stored level is fine
    /// enough - render from the full point list.
    ///
    /// A level built from its predecessor accumulates at most twice its own
    /// tolerance of error (a geometric series of halving tolerances), so the
    /// bound uses `2 × tolerance`.
    pub fn select(&self, px_per_merc: f64, max_error_px: f32) -> Option<&[u32]> {
        self.select_level(px_per_merc, max_error_px)
            .and_then(|i| self.level(i))
    }

    /// Index of the level [`TrackLod::select`] would return.
    pub fn select_level(&self, px_per_merc: f64, max_error_px: f32) -> Option<usize> {
        (0..self.levels.len())
            .rev()
            .find(|&i| 2.0 * self.level_tolerance_merc(i) * px_per_merc <= f64::from(max_error_px))
    }

    /// The point indices of stored level `i`.
    pub fn level(&self, i: usize) -> Option<&[u32]> {
        self.levels.get(i).map(Vec::as_slice)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TrackMetadata {
    pub index: usize,
    pub distance_km: Length,
    pub duration: Duration,
    pub time_range: TimeRange,
    /// Geographic bounding box in (lon, lat) coordinate order per geo-types convention.
    pub bounding_box: Rect<f64>,
    /// Normalised Web Mercator bounding box, pre-computed from `bounding_box`.
    /// Used by map renderers for O(1) viewport intersection tests without trigonometry.
    pub merc_bounds: MercBounds,
    pub point_set_diameter_m: Length,
    /// `None` when the track has fewer than two points (no segments).
    pub segment_length_range: Option<SegmentLengthRange>,
    pub has_custom_markers: bool,
    pub tpv_count: usize,
    pub satellite_report_count: usize,
    pub custom_marker_count: usize,
    pub generated_marker_count: usize,
    pub event_marker_count: usize,
    /// `None` when the track has no satellite reports.
    pub fix_stats: Option<FixStats>,
}

impl TrackMetadata {
    /// Returns `true` when the track has at least one custom, event, or generated marker.
    pub fn has_any_marker(&self) -> bool {
        self.has_custom_markers || self.generated_marker_count > 0 || self.event_marker_count > 0
    }
}

/// Sentinel default for use in test helpers via struct-update syntax (`..TrackMetadata::default()`).
///
/// All numeric fields are zero, bounding box is a point at the origin, and
/// `fix_stats` is `None`. Not intended for production construction. Callers
/// should set all semantically meaningful fields explicitly.
impl Default for TrackMetadata {
    fn default() -> Self {
        Self {
            index: 0,
            distance_km: Length::new::<kilometer>(0.0),
            duration: Duration::zero(),
            time_range: TimeRange::new(DateTime::<Utc>::UNIX_EPOCH, DateTime::<Utc>::UNIX_EPOCH),
            bounding_box: Rect::new(Coord { x: 0.0, y: 0.0 }, Coord { x: 0.0, y: 0.0 }),
            merc_bounds: MercBounds {
                x_min: 0.0,
                x_max: 0.0,
                y_min: 0.0,
                y_max: 0.0,
            },
            point_set_diameter_m: Length::new::<meter>(0.0),
            segment_length_range: None,
            has_custom_markers: false,
            tpv_count: 0,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 0,
            event_marker_count: 0,
            fix_stats: None,
        }
    }
}

/// Compute the normalised Web Mercator bounding box for a geographic rectangle.
///
/// The input `Rect` uses (lon, lat) coordinate order per `geo_types` convention.
///
/// Mercator Y increases south (0 = north pole, 1 = south pole), so the
/// northernmost latitude (`bb.max().y`) maps to `y_min`.
pub fn merc_bounds_for_rect(bb: Rect<f64>) -> MercBounds {
    use crate::coordinates::{Latitude, Longitude};
    let sw = crate::mercator::normalize(Latitude::new(bb.max().y), Longitude::new(bb.min().x));
    let ne = crate::mercator::normalize(Latitude::new(bb.min().y), Longitude::new(bb.max().x));
    MercBounds {
        x_min: sw.x,
        x_max: ne.x,
        y_min: sw.y,
        y_max: ne.y,
    }
}

/// A point in the global spatial index, covering TPV fixes and all marker categories.
///
/// Ghost TPV fixes (heading == `None`) are excluded - their position is interpolated
/// at render time rather than pre-computed.
#[derive(Debug, Clone, Copy)]
pub struct SpatialPoint {
    pub merc: MercPoint,
    pub file_index: FileIdx,
    pub track_index: TrackIdx,
    pub point_index: PointIdx,
    pub category: DataCategory,
}

impl SpatialPoint {
    pub fn track_ref(&self) -> crate::highlight::TrackRef {
        crate::highlight::TrackRef::new(self.file_index, self.track_index)
    }
}

impl rstar::RTreeObject for SpatialPoint {
    type Envelope = rstar::AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        rstar::AABB::from_point([self.merc.x, self.merc.y])
    }
}

impl rstar::PointDistance for SpatialPoint {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        let dx = self.merc.x - point[0];
        let dy = self.merc.y - point[1];
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone)]
pub struct LoadedTrack {
    pub metadata: TrackMetadata,
    /// TPV points, each optionally paired with a satellite report.
    pub points: Vec<NavPoint>,
    /// Multi-resolution decimation of `points` for rendering. Empty (the
    /// default) makes renderers fall back to the full point list.
    pub lod: TrackLod,
    /// Satellite-label anchor candidates, ascending by point index. Empty
    /// when the track has no satellite reports.
    pub sat_label_anchors: Vec<SatLabelAnchor>,
    pub custom_markers: Vec<CustomMarker>,
    pub generated_markers: Vec<GeneratedMarker>,
    pub event_markers: Vec<EventMarker>,
    /// Ad-hoc sensor channels, each holding the samples whose timestamp falls in
    /// this track's time range. File-level on load, partitioned here by time.
    pub channels: Vec<Channel>,
}

impl TrackRef {
    /// The track this ref addresses, `None` when either index is stale.
    ///
    /// The canonical form of the `fi.get(files)` + `index.get(&f.tracks)`
    /// chain that otherwise gets re-spelled at every lookup site.
    pub fn resolve(self, files: &[LoadedFile]) -> Option<&LoadedTrack> {
        self.fi.get(files).and_then(|f| self.index.get(&f.tracks))
    }
}

/// Platform a recording was made on, declared by the recorder via the SDK's
/// `travel_mode` metadata field.
///
/// Mirrors `geotrace_sdk::TravelMode` (the structurally-identical wire-format
/// type); `gt-loader` converts between the two. Keep the variant sets in sync.
/// Wire values outside the known set are preserved in [`TravelMode::Unknown`],
/// never dropped - unexpected declarations are signal, not noise.
///
/// `Display`/`FromStr` (via `strum`) give the lower snake_case wire form used
/// by [`TravelMode::from_wire`].
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    strum::EnumIter,
)]
#[strum(serialize_all = "snake_case")]
pub enum TravelMode {
    Car,
    Motorcycle,
    Bicycle,
    Pedestrian,
    Boat,
    Rail,
    Aircraft,
    /// A wire value not in the known set, preserved verbatim.
    #[strum(default)]
    Unknown(String),
}

impl TravelMode {
    /// Parses the lower snake_case wire form (the `meta_travel_mode` attribute
    /// value).
    ///
    /// Never fails: values outside the known set become
    /// [`TravelMode::Unknown`], preserving the input verbatim.
    pub fn from_wire(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        match s.parse() {
            Ok(mode) => mode,
            // `#[strum(default)]` makes parsing infallible; keep the explicit
            // fallback so removing the default cannot introduce a panic here.
            Err(_) => TravelMode::Unknown(s.to_owned()),
        }
    }

    /// Canonical human-readable name, e.g. `TravelMode::Car.display_name() == "Car"`.
    ///
    /// Single source of truth for this type's display spelling - call sites
    /// should format through this rather than re-typing the name. Unknown
    /// values display their preserved wire form verbatim.
    pub fn display_name(&self) -> &str {
        match self {
            TravelMode::Car => "Car",
            TravelMode::Motorcycle => "Motorcycle",
            TravelMode::Bicycle => "Bicycle",
            TravelMode::Pedestrian => "Pedestrian",
            TravelMode::Boat => "Boat",
            TravelMode::Rail => "Rail",
            TravelMode::Aircraft => "Aircraft",
            TravelMode::Unknown(raw) => raw,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub filename: String,
    pub total_distance_km: Length,
    pub total_duration: Duration,
    pub time_range: TimeRange,
    /// Aggregated fix stats across all tracks. `None` when no track has satellite reports.
    pub fix_stats: Option<FixStats>,
    /// Optional file title from the recording's SDK metadata.
    pub title: Option<String>,
    /// Optional producing device/sensor from the recording's SDK metadata.
    pub device: Option<String>,
    /// Optional free-text notes from the recording's SDK metadata.
    pub notes: Option<String>,
    /// Optional declared travel mode from the recording's SDK metadata.
    pub travel_mode: Option<TravelMode>,
}

/// Sentinel default for use in test helpers via struct-update syntax (`..FileMetadata::default()`).
///
/// Filename is empty, all durations and distances are zero, and `fix_stats` is `None`.
/// Not intended for production construction.
impl Default for FileMetadata {
    fn default() -> Self {
        Self {
            filename: String::new(),
            total_distance_km: Length::new::<kilometer>(0.0),
            total_duration: Duration::zero(),
            time_range: TimeRange::new(DateTime::<Utc>::UNIX_EPOCH, DateTime::<Utc>::UNIX_EPOCH),
            fix_stats: None,
            title: None,
            device: None,
            notes: None,
            travel_mode: None,
        }
    }
}

/// Configuration for log-marker and satellite association.
///
/// Stored in `Settings` and persisted to the config file. Also carried on
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

/// Where the file content came from. Stored on [`LoadedFile`] to enable
/// re-processing when association settings change.
#[derive(Debug, Clone)]
pub enum FileSource {
    /// Loaded from a path on disk (GTD file).
    GtdPath(PathBuf),
    /// Loaded from bytes delivered via drag-and-drop (GTD file).
    GtdBytes(Arc<[u8]>),
    /// Loaded from a log file on disk.
    LogPath(PathBuf),
    /// Loaded from in-memory log text (e.g. dropped bytes decoded to UTF-8).
    LogText(Arc<str>),
}

/// A structured data quality warning produced when loading a recording file.
#[derive(Debug, Clone)]
pub struct LoadWarning {
    /// Number of instances of this issue in the file.
    pub count: u32,
    /// Short description of the issue (e.g. "satellite(s) with PRN 0").
    pub issue: String,
    /// Explanation of why the issue matters and how to resolve it.
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub metadata: FileMetadata,
    pub tracks: Vec<LoadedTrack>,
    /// Icon/color overrides keyed by variant path. File-level (shared across tracks).
    pub event_marker_styles: HashMap<String, EventMarkerStyle>,
    /// Event markers whose timestamp did not fall within any track's time window.
    pub orphaned_event_markers: Vec<EventMarker>,
    /// Where this file was loaded from. Used to re-process when settings change.
    pub source: FileSource,
    /// Data quality warnings detected when the file was loaded (empty when clean).
    pub load_warnings: Vec<LoadWarning>,
}

#[cfg(test)]
mod travel_mode_tests {
    use strum::{EnumCount, IntoEnumIterator};

    use super::*;

    /// The wire form is the `meta_travel_mode` attribute value produced by the
    /// SDK. Every variant must round-trip through it, and unknown values must
    /// be preserved verbatim rather than dropped.
    #[test]
    fn from_wire_round_trips_every_variant() {
        for mode in TravelMode::iter() {
            assert_eq!(TravelMode::from_wire(mode.to_string()), mode);
        }
        assert_eq!(
            TravelMode::from_wire("hovercraft"),
            TravelMode::Unknown("hovercraft".into())
        );
    }

    /// Pin the display spellings so a variant rename cannot silently change
    /// what the metadata rows show. The table length is asserted against
    /// `EnumCount` so a new variant cannot be forgotten here.
    #[test]
    fn display_names_are_stable() {
        let known = [
            (TravelMode::Car, "Car"),
            (TravelMode::Motorcycle, "Motorcycle"),
            (TravelMode::Bicycle, "Bicycle"),
            (TravelMode::Pedestrian, "Pedestrian"),
            (TravelMode::Boat, "Boat"),
            (TravelMode::Rail, "Rail"),
            (TravelMode::Aircraft, "Aircraft"),
        ];
        // Every variant except the `Unknown` carrier must appear in the table.
        assert_eq!(known.len(), TravelMode::COUNT - 1);
        for (mode, display) in known {
            assert_eq!(mode.display_name(), display);
        }
        assert_eq!(
            TravelMode::Unknown("hovercraft".into()).display_name(),
            "hovercraft"
        );
    }
}
