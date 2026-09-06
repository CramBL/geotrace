use crate::channel::Channel;
use crate::coordinates::{Latitude, Longitude};
use crate::geo_bounds::GeoBounds;
use crate::highlight::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};
use crate::load_warning::LoadWarning;
use crate::markers::{CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker};
use crate::mercator::{self, MercPoint};
use crate::nav_point::{NavPoint, ResolvedPosition};
use crate::placed_point::PlacedPoints;
use crate::sat_label::SatLabelAnchor;
use crate::satellites::Satellites;
use crate::time_types::GpsTime;
use chrono::{DateTime, Days, Duration, NaiveDate, Utc};
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::Arc;
use uom::si::f64::Length;

/// Normalised Web Mercator bounding box, with all values in `[0.0, 1.0]`.
///
/// Mercator Y increases south (0 = north pole, 1 = south pole), so `y_min`
/// corresponds to the northernmost latitude.
///
/// `x_min > x_max` describes a box across the antimeridian: the world's
/// eastern edge cuts it into `[x_min, 1]` and `[0, x_max]`, and
/// [`MercBounds::intersects`] tests both pieces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MercBounds {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl MercBounds {
    /// Returns `true` when `self` overlaps `viewport` in both axes. Either box
    /// may cross the antimeridian.
    pub fn intersects(self, viewport: MercBounds) -> bool {
        self.y_max >= viewport.y_min && self.y_min <= viewport.y_max && self.x_overlaps(viewport)
    }

    pub fn crosses_the_antimeridian(self) -> bool {
        self.x_min > self.x_max
    }

    fn x_overlaps(self, other: MercBounds) -> bool {
        match (
            self.crosses_the_antimeridian(),
            other.crosses_the_antimeridian(),
        ) {
            // Both cover the antimeridian, so they share at least it.
            (true, true) => true,
            // The crossing box covers `[x_min, 1]` and `[0, x_max]`: the other
            // one overlaps as soon as it reaches either piece.
            (true, false) | (false, true) => self.x_max >= other.x_min || self.x_min <= other.x_max,
            (false, false) => self.x_max >= other.x_min && self.x_min <= other.x_max,
        }
    }

    /// The narrower of the two boxes holding both `self` and `other`. In x it
    /// closes whichever of the two gaps between the boxes is shorter, the way
    /// [`crate::geo_bounds::LonRange::union`] grows a longitude range: the
    /// union of two boxes on either side of the antimeridian crosses it. In y
    /// it takes the plain minimum and maximum: the projection has no wrap
    /// north to south.
    pub fn union(self, other: MercBounds) -> Self {
        let eastward = self.x_grown_east_over(other);
        let westward = other.x_grown_east_over(self);
        let narrower = if eastward.x_span() <= westward.x_span() {
            eastward
        } else {
            westward
        };
        Self {
            x_min: narrower.x_min,
            x_max: narrower.x_max,
            y_min: self.y_min.min(other.y_min),
            y_max: self.y_max.max(other.y_max),
        }
    }

    /// The part of `self` inside `viewport`, `None` when the two do not
    /// overlap. A crossing box on either side gives back the whole `viewport`:
    /// a box across the antimeridian meets it in two pieces, which one box
    /// cannot hold.
    pub fn clamped_within(self, viewport: MercBounds) -> Option<Self> {
        if !self.intersects(viewport) {
            return None;
        }
        if self.crosses_the_antimeridian() || viewport.crosses_the_antimeridian() {
            return Some(viewport);
        }
        Some(Self {
            x_min: self.x_min.max(viewport.x_min),
            x_max: self.x_max.min(viewport.x_max),
            y_min: self.y_min.max(viewport.y_min),
            y_max: self.y_max.min(viewport.y_max),
        })
    }

    /// This box as an envelope over the [`SpatialPoint`]s inside it.
    ///
    /// A box across the antimeridian covers two pieces of the world, which one
    /// envelope cannot express: the caller must widen or split such a box
    /// before querying with it.
    pub fn envelope(self) -> rstar::AABB<[f64; 2]> {
        debug_assert!(
            !self.crosses_the_antimeridian(),
            "an envelope across the antimeridian reads its x bounds the other way round"
        );
        rstar::AABB::from_corners([self.x_min, self.y_min], [self.x_max, self.y_max])
    }

    /// How wide the box is in x, from nothing to the world's whole width.
    fn x_span(self) -> f64 {
        match self.x_max - self.x_min >= WORLD_WIDTH_MERC {
            true => WORLD_WIDTH_MERC,
            false => (self.x_max - self.x_min).rem_euclid(WORLD_WIDTH_MERC),
        }
    }

    /// `self` grown eastward in x until it reaches the eastern edge of
    /// `other`, keeping its own y bounds. A box grown over the whole world
    /// comes back as the full width from 0 to 1.
    fn x_grown_east_over(self, other: MercBounds) -> Self {
        let span = ((other.x_min - self.x_min).rem_euclid(WORLD_WIDTH_MERC) + other.x_span())
            .max(self.x_span());
        if span >= WORLD_WIDTH_MERC {
            return Self {
                x_min: 0.0,
                x_max: WORLD_WIDTH_MERC,
                ..self
            };
        }
        let east = self.x_min + span;
        Self {
            x_max: if east >= WORLD_WIDTH_MERC {
                east - WORLD_WIDTH_MERC
            } else {
                east
            },
            ..self
        }
    }
}

/// What the normalised Mercator world spans in x, from the antimeridian back
/// to itself.
const WORLD_WIDTH_MERC: f64 = 1.0;

/// A longitude range crossing the antimeridian projects to `x_min > x_max`,
/// and a full circle to the world's whole width.
impl From<GeoBounds> for MercBounds {
    fn from(bounds: GeoBounds) -> Self {
        let north_west = mercator::normalize(bounds.lat.north(), bounds.lon.start());
        let south_east = mercator::normalize(bounds.lat.south(), bounds.lon.end());
        let (x_min, x_max) = if bounds.lon.is_full_circle() {
            (0.0, 1.0)
        } else {
            (north_west.x, south_east.x)
        };
        Self {
            x_min,
            x_max,
            y_min: north_west.y,
            y_max: south_east.y,
        }
    }
}

/// A closed time interval `[start, end]` with named fields.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        Self { start, end }
    }

    /// The span from the earliest to the latest of `first` and `rest`, in
    /// whatever order they arrive.
    pub fn spanning(first: DateTime<Utc>, rest: impl IntoIterator<Item = DateTime<Utc>>) -> Self {
        rest.into_iter().fold(Self::new(first, first), |span, at| {
            Self::new(span.start.min(at), span.end.max(at))
        })
    }

    pub fn duration(&self) -> Duration {
        self.end - self.start
    }

    /// Whether `instant` falls in the range, both ends included.
    pub fn contains(self, instant: DateTime<Utc>) -> bool {
        (self.start..=self.end).contains(&instant)
    }

    /// The smallest range covering both `self` and `other`, including any gap
    /// between them.
    pub fn union(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// The span `self` and `other` share, `None` when they are disjoint. Two
    /// ranges meeting at one instant share a zero-length range.
    pub fn intersection(self, other: Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        (start <= end).then(|| Self::new(start, end))
    }

    /// Returns `true` when `self` overlaps the optional `[window_start, window_end]` window.
    ///
    /// An absent bound is treated as unbounded (−∞ or +∞ respectively), so a
    /// fully absent window matches every range. A window whose start is after
    /// its end overlaps no range: it excludes every instant.
    pub fn overlaps_window(
        self,
        window_start: Option<DateTime<Utc>>,
        window_end: Option<DateTime<Utc>>,
    ) -> bool {
        if let (Some(start), Some(end)) = (window_start, window_end)
            && start > end
        {
            return false;
        }
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

    /// The UTC days this range touches, oldest first.
    ///
    /// [`None`] when the range touches more than `max_days` of them, or when
    /// [`Self::end`] precedes [`Self::start`].
    pub fn utc_days(self, max_days: usize) -> Option<Vec<NaiveDate>> {
        let (first, last) = (self.start.date_naive(), self.end.date_naive());
        if last < first {
            return None;
        }
        // A range of centuries is rejected without being walked: the bound
        // stops one day past the cap.
        let bound = first.checked_add_days(Days::new(u64::try_from(max_days).ok()?))?;
        let days = crate::utc_days::days_in_range(first..=last.min(bound), |_| true);
        (days.len() <= max_days).then_some(days)
    }
}

/// Which marker types a track must have to pass the marker filter.
///
/// `CustomMarker` is a strict subset of `AnyMarker`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkerRequirement {
    /// No marker constraint - all tracks pass.
    #[default]
    None,
    /// Track must have at least one custom *or* generated marker.
    AnyMarker,
    /// Track must have at least one *custom* marker.
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

impl FixStats {
    /// `None` when no track of `tracks` has satellite reports.
    fn summed_over_tracks(tracks: &[LoadedTrack]) -> Option<Self> {
        let mut summed: Option<Self> = None;
        for stats in tracks.iter().filter_map(|track| track.metadata.fix_stats) {
            let summed = summed.get_or_insert(Self {
                time_with_fix: Duration::zero(),
                time_without_fix: Duration::zero(),
                fix_loss_count: 0,
                max_continuous_no_fix: Duration::zero(),
            });
            summed.time_with_fix += stats.time_with_fix;
            summed.time_without_fix += stats.time_without_fix;
            summed.fix_loss_count = summed.fix_loss_count.saturating_add(stats.fix_loss_count);
            summed.max_continuous_no_fix = summed
                .max_continuous_no_fix
                .max(stats.max_continuous_no_fix);
        }
        summed
    }
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
/// iteration by on-screen detail. Mercator space
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
    /// at `px_per_merc` (pixels per Mercator unit, i.e. the world's width in
    /// pixels). `None` when no stored level is fine enough - render from the
    /// full point list.
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

/// Where a track's fixes are drawn, and the measures taken over them.
///
/// A recording places its fixes before it is cut into tracks, so a track has a
/// geometry unless the whole recording has no fix with a latitude and a
/// longitude in range.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackGeometry {
    Measured(MeasuredTrackGeometry),
    /// No fix of the recording has a position, so nothing places this track's
    /// fixes: it is drawn nowhere and measures nothing. Its fixes still carry
    /// everything the receiver recorded.
    NoValidPosition,
}

impl TrackGeometry {
    pub fn measured(&self) -> Option<&MeasuredTrackGeometry> {
        match self {
            Self::Measured(measured) => Some(measured),
            Self::NoValidPosition => None,
        }
    }
}

/// The geometry of a track whose fixes all have a position: where each of them
/// is drawn, and what that path measures.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredTrackGeometry {
    /// One position per fix of the track, in fix order.
    pub resolved_positions: Vec<ResolvedPosition>,
    pub bounding_box: GeoBounds,
    /// Pre-computed from `bounding_box` so map renderers get O(1) viewport
    /// intersection tests without trigonometry.
    pub merc_bounds: MercBounds,
    pub distance_km: Length,
    pub point_set_diameter_m: Length,
    /// `None` when the track has fewer than two points (no segments).
    pub segment_length_range: Option<SegmentLengthRange>,
}

#[derive(Debug, Clone, Copy)]
pub struct TrackMetadata {
    /// The track's 1-based number. For a recording in the history database it
    /// is the row that the track takes in the stored track table, plus one: a
    /// permanently deleted track leaves a tombstone in its row, and the tracks
    /// after it keep their numbers.
    pub index: usize,
    pub duration: Duration,
    pub time_range: TimeRange,
    pub has_custom_markers: bool,
    pub tpv_count: usize,
    /// Fixes whose recorded latitude or longitude is outside its range. Those
    /// are drawn where the track builder placed them, not where the receiver
    /// wrote them.
    pub invalid_position_count: usize,
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

/// A point in the global spatial index, covering TPV fixes and all marker
/// categories. A fix is indexed at the position it is drawn at, which for a
/// ghost fix is not the position the receiver recorded.
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
    /// Where the track builder placed `points`, and what they measure.
    pub geometry: TrackGeometry,
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

/// How far from a point a satellite report may be borrowed for display, in
/// seconds. Satellites move arc-minutes per second, so a report this close is
/// geometrically accurate as long as its age is shown.
pub const SKY_REPORT_MAX_AGE_SECS: i64 = 10;

/// A satellite report resolved for a track point by
/// [`LoadedTrack::nearest_satellite_report`].
#[derive(Debug, Clone, Copy)]
pub struct NearestSatelliteReport<'a> {
    pub satellites: &'a Satellites,
    /// Signed offset from the point's time to the report's time: positive
    /// when the report is earlier, negative when it is later, zero for the
    /// point's own report.
    pub age: Duration,
}

impl LoadedTrack {
    /// This track's fixes with the position each is drawn at, `None` for a
    /// track no fix of the recording has a position for.
    pub fn placed_points(&self) -> Option<PlacedPoints<'_>> {
        let measured = self.geometry.measured()?;
        PlacedPoints::new(&self.points, &measured.resolved_positions)
    }

    /// Where the map draws the fix at `index`, `None` for a track with no
    /// geometry and for an index past its fixes.
    pub fn resolved_position_at(&self, index: usize) -> Option<(Latitude, Longitude)> {
        Some(self.placed_points()?.get(index)?.resolved_position())
    }

    /// The satellite report to show for `point_index`: the point's own report
    /// when it has one, otherwise the nearest report within
    /// [`SKY_REPORT_MAX_AGE_SECS`] of it, preferring the earlier side on a
    /// tie (the same rule the SDK builder uses when attaching reports).
    ///
    /// Ages are measured between the two points' TPV times, relying on points
    /// being in recording order.
    pub fn nearest_satellite_report(
        &self,
        point_index: PointIdx,
    ) -> Option<NearestSatelliteReport<'_>> {
        let point = point_index.get(&self.points)?;
        let time = point.tpv.time();
        if let Some(own) = report_with_age(point, time) {
            return Some(own);
        }

        let max_age = Duration::seconds(SKY_REPORT_MAX_AGE_SECS);
        let index = point_index.as_usize();
        let earlier = self
            .points
            .iter()
            .take(index)
            .rev()
            .take_while(|p| time - p.tpv.time() <= max_age)
            .find_map(|p| report_with_age(p, time));
        let later = self
            .points
            .iter()
            .skip(index + 1)
            .take_while(|p| p.tpv.time() - time <= max_age)
            .find_map(|p| report_with_age(p, time));
        match (earlier, later) {
            (Some(earlier), Some(later)) if later.age.abs() < earlier.age.abs() => Some(later),
            (earlier, later) => earlier.or(later),
        }
    }
}

/// `point`'s report paired with its age relative to `time`, `None` for
/// report-free points.
fn report_with_age(point: &NavPoint, time: GpsTime) -> Option<NearestSatelliteReport<'_>> {
    let age = time - point.tpv.time();
    point
        .satellites
        .as_ref()
        .map(|satellites| NearestSatelliteReport { satellites, age })
}

impl TrackRef {
    /// The track this ref addresses, `None` when either index is stale.
    pub fn resolve(self, files: &[LoadedFile]) -> Option<&LoadedTrack> {
        self.fi.get(files).and_then(|f| self.index.get(&f.tracks))
    }
}

/// Platform a recording was made on, declared by the recorder via the SDK's
/// `travel_mode` metadata field.
///
/// Matches `geotrace_sdk::TravelMode` (the structurally-identical wire-format
/// type). `gt-loader` converts between the two. Keep the variant sets in sync.
/// Wire values outside the known set are preserved in [`TravelMode::Unknown`],
/// never dropped.
///
/// `Display`/`FromStr` (via `strum`) give the lower `snake_case` wire form used
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
    /// Parses the lower `snake_case` wire form (the `meta_travel_mode` attribute
    /// value).
    ///
    /// Never fails: values outside the known set become
    /// [`TravelMode::Unknown`], preserving the input verbatim.
    pub fn from_wire(s: impl AsRef<str>) -> Self {
        let s = s.as_ref();
        match s.parse() {
            Ok(mode) => mode,
            // `#[strum(default)]` makes parsing infallible. The explicit
            // fallback survives removing that default.
            Err(_) => TravelMode::Unknown(s.to_owned()),
        }
    }

    /// Canonical human-readable name, e.g. `TravelMode::Car.display_name() == "Car"`.
    ///
    /// Unknown values display their preserved wire form verbatim.
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

/// How far a recording travelled, summed over the tracks that have a geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TotalDistance {
    Measured(Length),
    /// No track of the recording has a geometry, so nothing measures a
    /// distance.
    NoMeasuredTrack,
}

impl TotalDistance {
    pub fn measured(self) -> Option<Length> {
        match self {
            Self::Measured(distance) => Some(distance),
            Self::NoMeasuredTrack => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub filename: String,
    pub total_distance: TotalDistance,
    pub total_duration: Duration,
    /// The span from the earliest start to the latest end over every track of
    /// the recording, whatever order the tracks are stored in. `None` when the
    /// recording has no track.
    pub time_range: Option<TimeRange>,
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

impl FileMetadata {
    pub fn set_track_aggregates(
        &mut self,
        TrackAggregates {
            total_distance,
            total_duration,
            time_range,
            fix_stats,
        }: TrackAggregates,
    ) {
        self.total_distance = total_distance;
        self.total_duration = total_duration;
        self.time_range = time_range;
        self.fix_stats = fix_stats;
    }
}

/// The [`FileMetadata`] figures aggregated over a recording's tracks. They
/// cover the tracks the view holds, and are computed again whenever a track
/// leaves a loaded recording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackAggregates {
    pub total_distance: TotalDistance,
    pub total_duration: Duration,
    pub time_range: Option<TimeRange>,
    pub fix_stats: Option<FixStats>,
}

impl TrackAggregates {
    pub fn over_tracks(tracks: &[LoadedTrack]) -> Self {
        let total_distance = tracks
            .iter()
            .filter_map(|track| Some(track.geometry.measured()?.distance_km))
            .reduce(|total, distance| total + distance)
            .map_or(TotalDistance::NoMeasuredTrack, TotalDistance::Measured);
        let total_duration = tracks.iter().fold(Duration::zero(), |total, track| {
            total + track.metadata.duration
        });
        let time_range = tracks
            .iter()
            .map(|track| track.metadata.time_range)
            .reduce(TimeRange::union);
        Self {
            total_distance,
            total_duration,
            time_range,
            fix_stats: FixStats::summed_over_tracks(tracks),
        }
    }
}

/// Configuration for associating log entries with recorded positions.
///
/// Stored in `Settings` and persisted to the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AssociationConfig {
    /// Association window a freshly loaded log starts with: the time an entry
    /// may be away from the nearest fix of the recording it is anchored to and
    /// still take a position from it. Adjustable per log afterwards.
    pub log_association_window_s: u64,
}

impl Default for AssociationConfig {
    fn default() -> Self {
        Self {
            log_association_window_s: 60,
        }
    }
}

/// Where a recording's content came from. Stored on [`LoadedFile`] so it can be
/// re-processed under new settings.
#[derive(Debug, Clone)]
pub enum FileSource {
    /// Loaded from a path on disk (GTD file).
    GtdPath(PathBuf),
    /// Loaded from bytes delivered via drag-and-drop (GTD file).
    GtdBytes(Arc<[u8]>),
}

#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub metadata: FileMetadata,
    pub tracks: Vec<LoadedTrack>,
    /// Icon/color overrides keyed by variant path. File-level (shared across tracks).
    pub event_marker_styles: FxHashMap<String, EventMarkerStyle>,
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
    /// be preserved verbatim.
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

#[cfg(test)]
mod time_range_tests {
    use chrono::{NaiveDate, TimeZone as _, Utc};
    use rstest::rstest;

    use super::TimeRange;

    fn at(year: i32, month: u32, day: u32, hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0)
            .single()
            .unwrap_or_default()
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    #[rstest]
    #[case::in_order(8, &[12, 17], 8, 17)]
    #[case::reversed(17, &[12, 8], 8, 17)]
    #[case::a_step_backwards_between_two_earlier_ones(8, &[17, 12], 8, 17)]
    #[case::the_same_instant_twice(8, &[8], 8, 8)]
    #[case::a_single_instant(8, &[], 8, 8)]
    fn spanning_covers_every_instant(
        #[case] first_hour: u32,
        #[case] rest_hours: &[u32],
        #[case] expected_start_hour: u32,
        #[case] expected_end_hour: u32,
    ) {
        let span = TimeRange::spanning(
            at(2026, 7, 20, first_hour),
            rest_hours.iter().map(|&hour| at(2026, 7, 20, hour)),
        );

        assert_eq!(
            span,
            TimeRange::new(
                at(2026, 7, 20, expected_start_hour),
                at(2026, 7, 20, expected_end_hour)
            )
        );
    }

    #[rstest]
    #[case::in_order(8, 12, 17, 19)]
    #[case::reversed(17, 19, 8, 12)]
    fn union_covers_both_ranges(
        #[case] first_start_hour: u32,
        #[case] first_end_hour: u32,
        #[case] second_start_hour: u32,
        #[case] second_end_hour: u32,
    ) {
        let first = TimeRange::new(
            at(2026, 7, 20, first_start_hour),
            at(2026, 7, 20, first_end_hour),
        );
        let second = TimeRange::new(
            at(2026, 7, 20, second_start_hour),
            at(2026, 7, 20, second_end_hour),
        );

        assert_eq!(
            first.union(second),
            TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 19))
        );
    }

    #[rstest]
    #[case::disjoint(at(2026, 7, 20, 0), at(2026, 7, 20, 6), None)]
    #[case::meeting_at_one_instant(
        at(2026, 7, 20, 0),
        at(2026, 7, 20, 8),
        Some((at(2026, 7, 20, 8), at(2026, 7, 20, 8)))
    )]
    #[case::partly_covered(
        at(2026, 7, 20, 6),
        at(2026, 7, 20, 12),
        Some((at(2026, 7, 20, 8), at(2026, 7, 20, 12)))
    )]
    #[case::covering_the_other_whole(
        at(2026, 7, 20, 0),
        at(2026, 7, 21, 0),
        Some((at(2026, 7, 20, 8), at(2026, 7, 20, 17)))
    )]
    fn intersection_is_the_shared_span(
        #[case] start: chrono::DateTime<Utc>,
        #[case] end: chrono::DateTime<Utc>,
        #[case] expected: Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)>,
    ) {
        let day = TimeRange::new(at(2026, 7, 20, 8), at(2026, 7, 20, 17));
        let other = TimeRange::new(start, end);
        let expected = expected.map(|(start, end)| TimeRange::new(start, end));

        assert_eq!(day.intersection(other), expected);
        assert_eq!(
            other.intersection(day),
            expected,
            "the shared span does not depend on argument order"
        );
    }

    #[rstest]
    #[case::unbounded(None, None, true)]
    #[case::a_window_ending_on_the_range_start(None, Some(8), true)]
    #[case::a_window_ending_before_the_range(None, Some(7), false)]
    #[case::a_window_starting_on_the_range_end(Some(17), None, true)]
    #[case::a_window_starting_after_the_range(Some(18), None, false)]
    #[case::a_window_inside_the_range(Some(10), Some(12), true)]
    #[case::an_inverted_window_the_range_straddles(Some(12), Some(10), false)]
    fn overlaps_window_is_true_only_for_a_window_sharing_an_instant_with_the_range(
        #[case] window_start: Option<u32>,
        #[case] window_end: Option<u32>,
        #[case] expected: bool,
    ) {
        let hour_of_day = |hour: u32| at(2026, 7, 20, hour);
        let range = TimeRange::new(hour_of_day(8), hour_of_day(17));

        assert_eq!(
            range.overlaps_window(window_start.map(hour_of_day), window_end.map(hour_of_day)),
            expected
        );
    }

    #[rstest]
    #[case::within_one_day(at(2026, 7, 20, 8), at(2026, 7, 20, 17), Some(vec![date(2026, 7, 20)]))]
    #[case::across_midnight(
        at(2026, 7, 20, 23),
        at(2026, 7, 21, 1),
        Some(vec![date(2026, 7, 20), date(2026, 7, 21)])
    )]
    #[case::exactly_the_limit(
        at(2026, 7, 20, 0),
        at(2026, 7, 22, 23),
        Some(vec![date(2026, 7, 20), date(2026, 7, 21), date(2026, 7, 22)])
    )]
    #[case::one_past_the_limit(at(2026, 7, 20, 0), at(2026, 7, 23, 0), None)]
    #[case::end_before_start(at(2026, 7, 21, 0), at(2026, 7, 20, 0), None)]
    fn utc_days_walks_the_range_up_to_the_cap(
        #[case] start: chrono::DateTime<Utc>,
        #[case] end: chrono::DateTime<Utc>,
        #[case] expected: Option<Vec<NaiveDate>>,
    ) {
        assert_eq!(TimeRange::new(start, end).utc_days(3), expected);
    }
}

#[cfg(test)]
mod nearest_satellite_report_tests {
    use chrono::{DateTime, Duration, Utc};
    use rstest::rstest;

    use crate::coordinates::{Latitude, Longitude};
    use crate::geo_bounds::GeoBounds;
    use crate::highlight::PointIdx;
    use crate::nav_point::NavPoint;
    use crate::satellites::{Constellation, Satellite, Satellites};
    use crate::time_types::GpsTime;
    use crate::tpv::TimePositionVelocity;

    use uom::si::f64::Length;
    use uom::si::length::{kilometer, meter};

    use super::{
        LoadedTrack, MeasuredTrackGeometry, MercBounds, TimeRange, TrackGeometry, TrackMetadata,
    };
    use crate::nav_point::ResolvedPosition;

    fn empty_metadata() -> TrackMetadata {
        TrackMetadata {
            index: 0,
            duration: Duration::zero(),
            time_range: TimeRange::new(DateTime::<Utc>::UNIX_EPOCH, DateTime::<Utc>::UNIX_EPOCH),
            has_custom_markers: false,
            tpv_count: 0,
            invalid_position_count: 0,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 0,
            event_marker_count: 0,
            fix_stats: None,
        }
    }

    /// Every fix of these tracks sits at one position, which is where the
    /// builder would place them.
    fn geometry_at_one_position(fix_count: usize) -> TrackGeometry {
        let bounding_box = GeoBounds::single_position(Latitude::new(55.0), Longitude::new(12.0));
        TrackGeometry::Measured(MeasuredTrackGeometry {
            resolved_positions: (0..fix_count)
                .map(|_| ResolvedPosition::measured(Latitude::new(55.0), Longitude::new(12.0)))
                .collect(),
            bounding_box,
            merc_bounds: MercBounds::from(bounding_box),
            distance_km: Length::new::<kilometer>(0.0),
            point_set_diameter_m: Length::new::<meter>(0.0),
            segment_length_range: None,
        })
    }

    /// A track from `(seconds, report)` specs, where `Some(n)` attaches a
    /// report with `n` satellites so assertions can distinguish reports.
    fn track(spec: &[(i64, Option<u32>)]) -> LoadedTrack {
        let start = DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid");
        let points: Vec<NavPoint> = spec
            .iter()
            .map(|&(secs, report)| {
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(start + Duration::seconds(secs)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .build();
                let satellites = report.map(|count| {
                    let sats = (0..count)
                        .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, true))
                        .collect();
                    Satellites::new(None, None, sats)
                });
                NavPoint::new(tpv, satellites)
            })
            .collect();
        LoadedTrack {
            metadata: empty_metadata(),
            geometry: geometry_at_one_position(points.len()),
            points,
            lod: Default::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: Vec::new(),
            generated_markers: Vec::new(),
            event_markers: Vec::new(),
            channels: Vec::new(),
        }
    }

    #[rstest]
    #[case::own_report(&[(0, Some(3))], 0, Some((3, 0)))]
    #[case::earlier_within_window(&[(0, Some(3)), (4, None)], 1, Some((3, 4)))]
    #[case::later_within_window(&[(0, None), (6, Some(5))], 0, Some((5, -6)))]
    #[case::nearer_side_wins(&[(0, Some(3)), (4, None), (5, Some(7))], 1, Some((7, -1)))]
    #[case::earlier_wins_a_tie(&[(0, Some(3)), (4, None), (8, Some(7))], 1, Some((3, 4)))]
    #[case::window_edge_included(&[(0, Some(3)), (10, None)], 1, Some((3, 10)))]
    #[case::beyond_window(&[(0, Some(3)), (11, None)], 1, None)]
    #[case::skips_reportless_neighbors(&[(0, Some(3)), (1, None), (2, None)], 2, Some((3, 2)))]
    #[case::no_reports_at_all(&[(0, None), (1, None)], 0, None)]
    fn resolves_the_nearest_report(
        #[case] spec: &[(i64, Option<u32>)],
        #[case] point_index: usize,
        #[case] expected: Option<(u32, i64)>,
    ) {
        let track = track(spec);
        let report = track.nearest_satellite_report(PointIdx::new(point_index));
        let actual = report.map(|r| (r.satellites.satellite_count(), r.age.num_seconds()));
        assert_eq!(actual, expected);
    }

    #[test]
    fn out_of_bounds_index_is_none() {
        let track = track(&[(0, Some(3))]);
        assert!(track.nearest_satellite_report(PointIdx::new(9)).is_none());
    }
}

#[cfg(test)]
mod merc_bounds_tests {
    use rstest::rstest;

    use super::MercBounds;
    use crate::coordinates::{Latitude, Longitude};
    use crate::geo_bounds::{GeoBounds, LatRange, LonRange};
    use crate::mercator;

    const MERC_TOLERANCE: f64 = 1e-9;

    /// 1.5° of longitude wide, from 179.0° E to 179.5° W.
    fn bounds_across_the_antimeridian() -> GeoBounds {
        GeoBounds::from_positions([
            (Latitude::new(60.0), Longitude::new(179.0)),
            (Latitude::new(61.0), Longitude::new(-179.5)),
        ])
        .expect("two positions")
    }

    #[test]
    fn merc_bounds_across_the_antimeridian_wrap_at_the_world_edge() {
        let bounds = MercBounds::from(bounds_across_the_antimeridian());

        assert!(bounds.crosses_the_antimeridian());
        let width = (1.0 - bounds.x_min) + bounds.x_max;
        assert!(
            (width - 1.5 / 360.0).abs() < MERC_TOLERANCE,
            "1.5° of longitude is 1.5/360 of the world, got {width}"
        );
    }

    #[test]
    fn merc_bounds_put_the_northern_edge_at_y_min() {
        let bounds = MercBounds::from(bounds_across_the_antimeridian());
        let north = mercator::normalize(Latitude::new(61.0), Longitude::new(179.0));

        assert!((bounds.y_min - north.y).abs() < MERC_TOLERANCE);
        assert!(bounds.y_min < bounds.y_max);
    }

    #[test]
    fn merc_bounds_of_a_full_circle_span_the_world() {
        let bounds = MercBounds::from(GeoBounds {
            lat: LatRange::single_parallel(Latitude::new(89.9)),
            lon: LonRange::full_circle(),
        });

        assert!(!bounds.crosses_the_antimeridian());
        assert!(bounds.x_min.abs() < MERC_TOLERANCE);
        assert!((bounds.x_max - 1.0).abs() < MERC_TOLERANCE);
    }

    #[test]
    fn merc_bounds_ending_on_the_antimeridian_reach_the_eastern_edge() {
        let bounds = MercBounds::from(
            GeoBounds::from_positions([
                (Latitude::new(60.0), Longitude::new(170.0)),
                (Latitude::new(60.0), Longitude::new(180.0)),
            ])
            .expect("two positions"),
        );

        assert!(!bounds.crosses_the_antimeridian());
        assert!((bounds.x_max - 1.0).abs() < MERC_TOLERANCE);
    }

    #[rstest]
    #[case::viewport_east_of_the_antimeridian(0.9997, 0.9999, true)]
    #[case::viewport_west_of_the_antimeridian(0.0001, 0.0003, true)]
    #[case::viewport_on_the_prime_meridian(0.49, 0.51, false)]
    #[case::viewport_just_short_of_the_track(0.9960, 0.9965, false)]
    #[case::viewport_wrapping_too(0.99, 0.01, true)]
    #[case::viewport_covering_the_world(0.0, 1.0, true)]
    fn a_crossing_box_intersects_only_the_viewports_it_reaches(
        #[case] x_min: f64,
        #[case] x_max: f64,
        #[case] expected: bool,
    ) {
        let bounds = MercBounds::from(bounds_across_the_antimeridian());
        let viewport = MercBounds {
            x_min,
            x_max,
            y_min: 0.0,
            y_max: 1.0,
        };

        assert_eq!(bounds.intersects(viewport), expected);
        assert_eq!(
            viewport.intersects(bounds),
            expected,
            "overlap does not depend on argument order"
        );
    }

    /// A box over `x`, given as `[west, east]` in normalised Mercator x, at a
    /// fixed band north to south. It crosses the antimeridian when east is the
    /// smaller of the two.
    fn box_over_x([west, east]: [f64; 2]) -> MercBounds {
        MercBounds {
            x_min: west,
            x_max: east,
            y_min: 0.4,
            y_max: 0.6,
        }
    }

    fn assert_bounds_close(actual: MercBounds, expected: MercBounds) {
        let apart = (actual.x_min - expected.x_min)
            .abs()
            .max((actual.x_max - expected.x_max).abs())
            .max((actual.y_min - expected.y_min).abs())
            .max((actual.y_max - expected.y_max).abs());
        assert!(apart < MERC_TOLERANCE, "{actual:?} against {expected:?}");
    }

    /// The rule in x mirrors [`LonRange::union`] over longitudes.
    #[rstest]
    #[case::overlapping([0.2, 0.4], [0.3, 0.5], [0.2, 0.5])]
    #[case::one_inside_the_other([0.2, 0.8], [0.3, 0.4], [0.2, 0.8])]
    #[case::the_gap_east_of_the_first_box_is_shorter([0.2, 0.3], [0.6, 0.7], [0.2, 0.7])]
    #[case::the_gap_east_of_the_second_box_is_shorter([0.6, 0.7], [0.2, 0.3], [0.2, 0.7])]
    #[case::across_the_antimeridian([0.95, 0.99], [0.01, 0.05], [0.95, 0.05])]
    #[case::over_a_box_spanning_the_world([0.0, 1.0], [0.3, 0.4], [0.0, 1.0])]
    #[case::over_a_crossing_box([0.9, 0.1], [0.95, 0.99], [0.9, 0.1])]
    fn a_union_closes_the_shorter_gap_in_x(
        #[case] left: [f64; 2],
        #[case] right: [f64; 2],
        #[case] expected: [f64; 2],
    ) {
        assert_bounds_close(
            box_over_x(left).union(box_over_x(right)),
            box_over_x(expected),
        );
    }

    #[test]
    fn a_union_reaches_from_the_northern_edge_of_one_box_to_the_southern_edge_of_the_other() {
        let north = MercBounds {
            x_min: 0.2,
            x_max: 0.3,
            y_min: 0.1,
            y_max: 0.2,
        };
        let south = MercBounds {
            y_min: 0.7,
            y_max: 0.8,
            ..north
        };

        assert_bounds_close(
            north.union(south),
            MercBounds {
                x_min: 0.2,
                x_max: 0.3,
                y_min: 0.1,
                y_max: 0.8,
            },
        );
    }

    /// The clamp keeps the part of the box the viewport holds. A box across
    /// the antimeridian meets the viewport in two pieces, and the clamp
    /// returns the whole viewport.
    #[rstest]
    #[case::inside_the_viewport([0.3, 0.4], [0.2, 0.5], Some([0.3, 0.4]))]
    #[case::wider_than_the_viewport([0.1, 0.9], [0.2, 0.5], Some([0.2, 0.5]))]
    #[case::over_one_edge_of_the_viewport([0.1, 0.3], [0.2, 0.5], Some([0.2, 0.3]))]
    #[case::clear_of_the_viewport([0.6, 0.7], [0.2, 0.5], None)]
    #[case::across_the_antimeridian([0.95, 0.05], [0.9, 1.0], Some([0.9, 1.0]))]
    #[case::across_the_antimeridian_clear_of_the_viewport([0.95, 0.05], [0.2, 0.5], None)]
    fn a_box_is_clamped_to_the_viewport(
        #[case] bounds: [f64; 2],
        #[case] viewport: [f64; 2],
        #[case] expected: Option<[f64; 2]>,
    ) {
        assert_eq!(
            box_over_x(bounds).clamped_within(box_over_x(viewport)),
            expected.map(box_over_x)
        );
    }

    #[test]
    fn a_clamped_box_takes_the_nearer_edge_north_and_south() {
        let bounds = MercBounds {
            x_min: 0.2,
            x_max: 0.4,
            y_min: 0.3,
            y_max: 0.9,
        };
        let viewport = MercBounds {
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.5,
            y_max: 0.6,
        };

        assert_eq!(
            bounds.clamped_within(viewport),
            Some(MercBounds {
                x_min: 0.2,
                x_max: 0.4,
                y_min: 0.5,
                y_max: 0.6,
            })
        );
    }

    #[test]
    fn a_crossing_box_misses_a_viewport_south_of_it() {
        let bounds = MercBounds::from(bounds_across_the_antimeridian());
        let south_of_the_track = MercBounds {
            x_min: 0.999,
            x_max: 1.0,
            y_min: 0.9,
            y_max: 1.0,
        };

        assert!(!bounds.intersects(south_of_the_track));
    }
}
