use chrono::{DateTime, Duration, Utc};
use gt_analysis::clock_offset::{self, ClockOffsetExcursion};
use gt_analysis::robust::median_i64;
use gt_geo_math::{GreatCircleArc, path_distance_km, point_set_diameter_m, segment_length_range_m};
use gt_types::channel::Channel;
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::geo_bounds::{GeoBounds, PoleWinding};
use gt_types::load_warning::{AlterationWording, LoadWarning};
use gt_types::markers::{
    CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker, GeneratedMarkerKind,
};
use gt_types::nav_point::{NavPoint, ResolvedPosition};
use gt_types::placed_point::{PlacedPoint, PlacedPoints};
use gt_types::satellites::SlipEvent;
use gt_types::time_types::GpsTime;
use gt_types::track::{
    FileMetadata, FileSource, FixStats, LoadedFile, LoadedTrack, MeasuredTrackGeometry, MercBounds,
    SegmentLengthRange, TimeRange, TotalDistance, TrackGeometry, TrackMetadata, TravelMode,
};
use rustc_hash::FxHashMap;
use std::fmt;
use std::ops::Range;
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

/// The rule segmentation splits tracks by.
///
/// The history database holds the rule a recording's stored tracks were split
/// by: re-running segmentation under that rule reproduces the stored ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackSplitRule {
    /// A forward timestamp gap reaching `track_split_gap` starts a new track.
    /// A recording stored by an earlier version was split by this rule.
    ForwardGapOnly,
    /// A timestamp step reaching `track_split_gap` in either direction starts a
    /// new track.
    #[default]
    StepInEitherDirection,
}

/// Configuration that affects the track ranges produced by segmentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackLayoutConfig {
    /// Size of the timestamp step between consecutive points that starts a new
    /// track, in the directions `track_split_rule` covers.
    pub track_split_gap: Duration,
    pub track_split_rule: TrackSplitRule,
}

impl Default for TrackLayoutConfig {
    fn default() -> Self {
        Self {
            track_split_gap: Duration::seconds(300),
            track_split_rule: TrackSplitRule::default(),
        }
    }
}

impl TrackLayoutConfig {
    fn starts_a_new_track(self, step: Duration) -> bool {
        match self.track_split_rule {
            TrackSplitRule::ForwardGapOnly => step >= self.track_split_gap,
            TrackSplitRule::StepInEitherDirection => step.abs() >= self.track_split_gap,
        }
    }
}

/// Configuration for per-kind generated-marker detection.
///
/// These settings affect marker output only. They do not change track ranges
/// or hidden-track index meaning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeneratedMarkerConfig {
    pub detect_gnss_fix_lost: bool,
    pub detect_gnss_fix_regained: bool,
    /// Whether to flag abrupt GPS↔system clock-offset jumps as
    /// [`GeneratedMarkerKind::ClockDiscontinuity`] markers.
    pub detect_clock_discontinuities: bool,
    /// Sensitivity of the clock-discontinuity outlier test: a step must exceed
    /// this many robust standard deviations from the track's median step to be
    /// flagged.  Lower is more sensitive.  See `detect_clock_discontinuities`.
    pub clock_discontinuity_sigmas: f64,
    /// Whether to flag isolated departures of the GPS↔system clock offset as
    /// [`GeneratedMarkerKind::ClockOffsetExcursion`] markers.
    pub detect_clock_offset_excursions: bool,
    /// Deviation from a track's baseline clock offset, in seconds, above which a
    /// sample counts as an excursion.  Shared with the plot, which keeps those
    /// samples off its shared y-axis. See `gt_analysis::clock_offset`.
    pub clock_excursion_threshold_s: f32,
    /// Whether to flag loss-of-lock (cycle slip) events as
    /// [`GeneratedMarkerKind::Slip`] markers.
    pub detect_slips: bool,
    /// Elevation mask (degrees) for slip detection.  Shared with the slip-rate
    /// plot so markers and plot agree. See `gt_analysis::slip`.
    pub slip_elevation_mask_deg: f32,
    /// SNR drop (dB-Hz between epochs) above which a still-tracked satellite is
    /// counted as having slipped.
    pub slip_snr_drop_db: f32,
}

impl Default for GeneratedMarkerConfig {
    fn default() -> Self {
        Self {
            detect_gnss_fix_lost: true,
            detect_gnss_fix_regained: true,
            detect_clock_discontinuities: true,
            clock_discontinuity_sigmas: DEFAULT_CLOCK_OUTLIER_SIGMAS,
            detect_clock_offset_excursions: true,
            clock_excursion_threshold_s: DEFAULT_CLOCK_EXCURSION_THRESHOLD_S,
            detect_slips: true,
            slip_elevation_mask_deg: DEFAULT_SLIP_ELEVATION_MASK_DEG,
            slip_snr_drop_db: DEFAULT_SLIP_SNR_DROP_DB,
        }
    }
}

/// Full processing configuration for building a loaded file.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SegmentationConfig {
    pub track_layout: TrackLayoutConfig,
    pub generated_markers: GeneratedMarkerConfig,
}

/// Default elevation mask (degrees) for slip detection.  Mirrors the slip-rate
/// plot's default so a fresh config matches the plot out of the box.
pub const DEFAULT_SLIP_ELEVATION_MASK_DEG: f32 = 15.0;

/// Default SNR drop (dB-Hz) that counts as a slip.
pub const DEFAULT_SLIP_SNR_DROP_DB: f32 = 10.0;

/// Default deviation from a track's baseline clock offset, in seconds, above
/// which a sample counts as a clock offset excursion.  Re-exported from the
/// detector so the marker default and the plot default are one value.
pub const DEFAULT_CLOCK_EXCURSION_THRESHOLD_S: f32 = clock_offset::DEFAULT_EXCURSION_THRESHOLD_S;

/// Partitions `points` into contiguous track ranges. A new track begins where
/// the timestamp step between consecutive points reaches
/// `config.track_split_gap` in a direction `config.track_split_rule` covers.
/// Returns an empty vec for empty input.
pub fn segment_tracks(points: &[NavPoint], config: &TrackLayoutConfig) -> Vec<Range<usize>> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;

    for (i, pair) in points.windows(2).enumerate() {
        if let [a, b] = pair
            && config.starts_a_new_track(b.tpv.time() - a.tpv.time())
        {
            ranges.push(start..i + 1);
            start = i + 1;
        }
    }
    ranges.push(start..points.len());
    ranges
}

/// State machine for tracking GPS fix transitions within a track.
enum GpsFixState {
    /// No satellite report seen yet.
    Waiting,
    /// The most recent satellite report had `fix_count > 0`.
    HasFix {
        last_time: GpsTime,
        last_position: (Latitude, Longitude),
    },
    /// The most recent satellite report had `fix_count == 0`.
    LostFix {
        /// When the fix was last seen, for the regained-duration.
        lost_at: GpsTime,
    },
}

struct GpsFixTracker {
    state: GpsFixState,
}

impl GpsFixTracker {
    fn new() -> Self {
        Self {
            state: GpsFixState::Waiting,
        }
    }

    /// Advance the state machine by one satellite report.
    ///
    /// Returns `Some(marker)` when a transition emits a generated marker
    /// (`GnssFixLost` or `GnssFixRegained`), or `None` for silent transitions.
    fn update(&mut self, point: PlacedPoint<'_>, fix_count: u32) -> Option<GeneratedMarker> {
        let result;
        self.state = match self.state {
            GpsFixState::Waiting => {
                result = None;
                if fix_count > 0 {
                    GpsFixState::HasFix {
                        last_time: point.fix.tpv.time(),
                        last_position: point.resolved_position(),
                    }
                } else {
                    GpsFixState::Waiting
                }
            }
            GpsFixState::HasFix {
                last_time,
                last_position: (last_lat, last_lon),
            } => {
                if fix_count == 0 {
                    result = Some(GeneratedMarker::new(
                        last_time.utc(),
                        GeneratedMarkerKind::GnssFixLost,
                        last_lat,
                        last_lon,
                    ));
                    GpsFixState::LostFix { lost_at: last_time }
                } else {
                    result = None;
                    GpsFixState::HasFix {
                        last_time: point.fix.tpv.time(),
                        last_position: point.resolved_position(),
                    }
                }
            }
            GpsFixState::LostFix { lost_at } => {
                if fix_count > 0 {
                    let duration = point.fix.tpv.time().signed_duration_since(lost_at);
                    let (lat, lon) = point.resolved_position();
                    result = Some(GeneratedMarker::new(
                        point.fix.tpv.time().utc(),
                        GeneratedMarkerKind::GnssFixRegained {
                            fix_lost_duration: duration,
                        },
                        lat,
                        lon,
                    ));
                    GpsFixState::HasFix {
                        last_time: point.fix.tpv.time(),
                        last_position: (lat, lon),
                    }
                } else {
                    result = None;
                    GpsFixState::LostFix { lost_at }
                }
            }
        };
        result
    }
}

/// Every generated marker sits at a position, so only a track that has a
/// geometry has any.
fn detect_generated_markers(
    points: PlacedPoints<'_>,
    config: &GeneratedMarkerConfig,
) -> Vec<GeneratedMarker> {
    let mut tracker = GpsFixTracker::new();
    let mut markers = Vec::new();
    for point in points.iter() {
        // The state machine always advances so the regained-duration stays
        // correct, but a marker is only kept when its kind is enabled.
        if let Some(sats) = &point.fix.satellites
            && let Some(marker) = tracker.update(point, sats.fix_count())
            && fix_marker_enabled(&marker.kind, config)
        {
            markers.push(marker);
        }
    }
    // Excursions are classified whatever the marker toggle is set to: the
    // discontinuity pass needs them out of its step series either way, or one
    // excursion reads as a pair of jumps - out and straight back.
    let excursions =
        clock_offset::detect_excursions(points.fixes(), config.clock_excursion_threshold_s);
    if config.detect_clock_offset_excursions {
        markers.extend(excursion_markers(points, &excursions));
    }
    if config.detect_clock_discontinuities {
        markers.extend(detect_clock_discontinuities(
            points,
            config.clock_discontinuity_sigmas,
            &clock_offset::excursion_indices(&excursions),
        ));
    }
    if config.detect_slips {
        markers.extend(detect_slip_markers(points, config));
    }
    markers.sort_by_key(|m| m.time);
    markers
}

/// Non-fix kinds are gated at their call sites and return `true` here.
fn fix_marker_enabled(kind: &GeneratedMarkerKind, config: &GeneratedMarkerConfig) -> bool {
    match kind {
        GeneratedMarkerKind::GnssFixLost => config.detect_gnss_fix_lost,
        GeneratedMarkerKind::GnssFixRegained { .. } => config.detect_gnss_fix_regained,
        GeneratedMarkerKind::ClockDiscontinuity { .. }
        | GeneratedMarkerKind::ClockOffsetExcursion { .. }
        | GeneratedMarkerKind::Slip(_) => true,
    }
}

/// Build one [`GeneratedMarkerKind::Slip`] marker per epoch that had any
/// loss-of-lock, grouping every satellite that slipped at that epoch into the
/// one marker, placed at the position and time of that epoch.
fn detect_slip_markers(
    points: PlacedPoints<'_>,
    config: &GeneratedMarkerConfig,
) -> Vec<GeneratedMarker> {
    gt_analysis::loss_of_lock::detect_slip_events(
        points.fixes(),
        config.slip_elevation_mask_deg,
        config.slip_snr_drop_db,
    )
    .into_iter()
    .filter_map(|(i, slips)| {
        let point = points.get(i)?;
        let (lat, lon) = point.resolved_position();
        Some(GeneratedMarker::new(
            point.fix.tpv.time().utc(),
            GeneratedMarkerKind::Slip(SlipEvent { slips }),
            lat,
            lon,
        ))
    })
    .collect()
}

/// Fewest with-system-timestamp samples a track needs before clock-outlier
/// detection runs.  Detection works on the step series (one shorter), and the
/// median/MAD must survive a single outlier step, so at least three steps - four
/// samples - are required. Below that, detection is skipped to avoid a spurious
/// marker from an unstable estimate.
const MIN_CLOCK_SAMPLES: usize = 4;

/// Scales the median absolute deviation to an estimate of the standard
/// deviation for normally-distributed data (the usual robust-statistics
/// constant, `1 / Φ⁻¹(3/4)`).
const MAD_TO_SIGMA: f64 = 1.4826;

/// Default sensitivity for the clock-discontinuity outlier test (robust σ from
/// the median step), used when no configuration overrides it.  Public so the
/// persisted settings default and this algorithm stay in sync from one source.
pub const DEFAULT_CLOCK_OUTLIER_SIGMAS: f64 = 5.0;

/// Floor on the robust spread of the step series, in milliseconds.  A healthy
/// clock has near-zero step-to-step change and thus a near-zero MAD. Without a
/// floor, ordinary sub-second jitter would register as an outlier.  This is a
/// noise gate, not the detection threshold - on a track with genuinely jittery
/// clock steps the MAD dominates and the bar rises with the data.
const MIN_CLOCK_SPREAD_MS: f64 = 200.0;

/// Smallest clock-offset jump, in seconds, that a given sensitivity flags on a
/// track with negligible clock jitter (where the noise floor dominates).
///
/// Lets the UI give users a concrete sense of a `sigmas` setting without
/// duplicating the noise-floor constant: the threshold there is
/// `sigmas × MIN_CLOCK_SPREAD_MS`.  On noisier tracks the real bar is higher,
/// since the track's own spread takes over.
pub fn clock_discontinuity_floor_seconds(sigmas: f64) -> f64 {
    sigmas * MIN_CLOCK_SPREAD_MS / 1000.0
}

/// Emit a [`GeneratedMarkerKind::ClockDiscontinuity`] for each sample where the
/// GPS−system offset *jumps* abruptly from the previous sample.
///
/// Detection runs on the first-difference (step) series, the change in offset
/// between consecutive with-system-timestamp samples. Two passes: the first
/// measures the track's typical step size (median and median absolute
/// deviation), the second flags any step more than `sigmas` robust standard
/// deviations from that, floored at [`MIN_CLOCK_SPREAD_MS`].
///
/// Working on jumps, not levels, flags a discontinuity once at the transition,
/// not once per sample of a shifted plateau. A steady large offset produces no
/// jumps.
///
/// `sigmas` is the outlier sensitivity (see
/// [`SegmentationConfig::clock_discontinuity_sigmas`]). `excursion_indices`
/// (ascending) lists the samples already explained by a
/// [`GeneratedMarkerKind::ClockOffsetExcursion`], which are left out of the
/// step series.
fn detect_clock_discontinuities(
    points: PlacedPoints<'_>,
    sigmas: f64,
    excursion_indices: &[usize],
) -> Vec<GeneratedMarker> {
    // Pass 1: offset (ms) and source index for each with-system-timestamp
    // sample, then the step (first difference) between consecutive samples.
    let samples: Vec<(usize, i64)> = points
        .fixes()
        .iter()
        .enumerate()
        .filter(|(i, _)| excursion_indices.binary_search(i).is_err())
        .filter_map(|(i, p)| Some((i, p.tpv.gps_system_clock_offset()?.num_milliseconds())))
        .collect();
    if samples.len() < MIN_CLOCK_SAMPLES {
        return Vec::new();
    }
    // Saturating arithmetic throughout: offsets come from a parsed binary
    // format and may be adversarial.
    let steps: Vec<i64> = samples
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some(b.1.saturating_sub(a.1)),
            _ => None,
        })
        .collect();

    let Some(median) = median_i64(&steps) else {
        return Vec::new();
    };
    let deviations: Vec<i64> = steps
        .iter()
        .map(|&s| s.saturating_sub(median).saturating_abs())
        .collect();
    let Some(mad) = median_i64(&deviations) else {
        return Vec::new();
    };
    #[expect(
        clippy::cast_precision_loss,
        reason = "comparison only; realistic offsets are exact in f64, and precision \
                  loss at extreme (adversarial) magnitudes cannot change the verdict"
    )]
    let threshold = (mad as f64 * MAD_TO_SIGMA).max(MIN_CLOCK_SPREAD_MS) * sigmas;

    // Pass 2: flag the later sample of each outlier step.
    let mut markers = Vec::new();
    for pair in samples.windows(2) {
        let [a, b] = pair else { continue };
        let step = b.1.saturating_sub(a.1);
        #[expect(
            clippy::cast_precision_loss,
            reason = "comparison only; realistic offsets are exact in f64, and precision \
                      loss at extreme (adversarial) magnitudes cannot change the verdict"
        )]
        let is_outlier = (step.saturating_sub(median).saturating_abs() as f64) > threshold;
        if is_outlier && let Some(point) = points.get(b.0) {
            let (lat, lon) = point.resolved_position();
            markers.push(GeneratedMarker::new(
                point.fix.tpv.time().utc(),
                GeneratedMarkerKind::ClockDiscontinuity {
                    step: Duration::milliseconds(step),
                },
                lat,
                lon,
            ));
        }
    }
    markers
}

/// Build one [`GeneratedMarkerKind::ClockOffsetExcursion`] per excursion,
/// placed at the sample that departed furthest from the track's baseline
/// offset.  Detection lives in `gt_analysis::clock_offset` so the plot and these
/// markers agree on what an excursion is.
fn excursion_markers(
    points: PlacedPoints<'_>,
    excursions: &[ClockOffsetExcursion],
) -> Vec<GeneratedMarker> {
    excursions
        .iter()
        .filter_map(|excursion| {
            let peak = excursion.peak();
            let point = points.get(peak.index)?;
            let (lat, lon) = point.resolved_position();
            Some(GeneratedMarker::new(
                point.fix.tpv.time().utc(),
                GeneratedMarkerKind::ClockOffsetExcursion {
                    deviation: Duration::milliseconds(excursion.deviation_ms()),
                    offset: Duration::milliseconds(peak.offset_ms),
                    samples: u32::try_from(excursion.samples.len()).unwrap_or(u32::MAX),
                },
                lat,
                lon,
            ))
        })
        .collect()
}

/// Computes GNSS fix-quality statistics from a slice of nav points.
///
/// Returns `None` when there are fewer than two points with satellite reports
/// (not enough consecutive pairs to measure any interval).
pub fn compute_fix_stats(points: &[NavPoint]) -> Option<FixStats> {
    let sat_points: Vec<&NavPoint> = points.iter().filter(|p| p.satellites.is_some()).collect();

    if sat_points.len() < 2 {
        return None;
    }

    let mut time_with_fix = Duration::zero();
    let mut time_without_fix = Duration::zero();
    let mut fix_loss_count: u32 = 0;
    let mut max_continuous_no_fix = Duration::zero();
    let mut current_no_fix_streak = Duration::zero();

    for pair in sat_points.windows(2) {
        if let [a, b] = pair {
            let interval = b.tpv.time() - a.tpv.time();
            let a_has_fix = a.fix_count() > 0;
            let b_has_fix = b.fix_count() > 0;

            if a_has_fix {
                time_with_fix += interval;
                if current_no_fix_streak > Duration::zero() {
                    if current_no_fix_streak > max_continuous_no_fix {
                        max_continuous_no_fix = current_no_fix_streak;
                    }
                    current_no_fix_streak = Duration::zero();
                }
            } else {
                time_without_fix += interval;
                current_no_fix_streak += interval;
            }

            // Count the fix→no-fix transition here. The no-fix duration itself
            // is accumulated in the next iteration when this `b` becomes the
            // new `a` (and `a_has_fix` will be false).
            if a_has_fix && !b_has_fix {
                fix_loss_count = fix_loss_count.saturating_add(1);
            }
        }
    }

    if current_no_fix_streak > max_continuous_no_fix {
        max_continuous_no_fix = current_no_fix_streak;
    }

    Some(FixStats {
        time_with_fix,
        time_without_fix,
        fix_loss_count,
        max_continuous_no_fix,
    })
}

/// The span from the earliest to the latest fix time. A fix stamped before its
/// predecessor still falls inside it: nothing sorts the points a recording is
/// read from.
fn time_range_spanning_every_fix(points: &vec1::Vec1<NavPoint>) -> TimeRange {
    TimeRange::spanning(
        points.first().tpv.time().utc(),
        points.iter().map(|p| p.tpv.time().utc()),
    )
}

/// Computes `TrackMetadata` from a non-empty slice of points.
///
/// What the track's fixes measure is its geometry, computed separately by
/// [`measure_track_geometry`].
pub fn compute_track_metadata(
    index: usize,
    points: &vec1::Vec1<NavPoint>,
    custom_markers: &[CustomMarker],
    generated_markers: &[GeneratedMarker],
) -> TrackMetadata {
    let time_range = time_range_spanning_every_fix(points);

    TrackMetadata {
        index,
        duration: time_range.duration(),
        time_range,
        has_custom_markers: !custom_markers.is_empty(),
        tpv_count: points.len(),
        invalid_position_count: points
            .iter()
            .filter(|point| point.tpv.position().is_none())
            .count(),
        satellite_report_count: points.iter().filter(|p| p.satellites.is_some()).count(),
        custom_marker_count: custom_markers.len(),
        generated_marker_count: generated_markers.len(),
        event_marker_count: 0, // filled in by build_loaded_file after event marker assignment
        fix_stats: compute_fix_stats(points),
    }
}

/// Where the builder has placed each fix so far, `None` for a fix it has not
/// placed: the receiver wrote no position for it and no anchor gives it one.
type FixPlacements = Vec<Option<ResolvedPosition>>;

/// The recorded position of every fix that has one, before any interpolation.
fn recorded_placements(points: &[NavPoint]) -> FixPlacements {
    points
        .iter()
        .map(|point| {
            point
                .tpv
                .position()
                .map(|(latitude, longitude)| ResolvedPosition::measured(latitude, longitude))
        })
        .collect()
}

/// The geometry of `points` taken as a track on their own: every fix the
/// receiver did not measure is placed from the fixes that have a recorded
/// position, and the geometry is measured over where they all landed.
///
/// [`TrackGeometry::NoValidPosition`] when no fix of `points` has a recorded
/// position, which leaves the whole track unplaced.
pub fn measure_track_geometry(points: &[NavPoint]) -> TrackGeometry {
    let mut placements = recorded_placements(points);
    place_track_fixes(points, &mut placements);
    track_geometry(placements)
}

/// Places the fixes of one track from its own fixes: first those the receiver
/// wrote no position for, then the ones it dead-reckoned.
fn place_track_fixes(points: &[NavPoint], placements: &mut FixPlacements) {
    place_fixes(points, placements, UnmeasuredFix::CoordinateOutOfRange);
    place_fixes(points, placements, UnmeasuredFix::Ghost);
}

fn track_geometry(placements: FixPlacements) -> TrackGeometry {
    placements
        .into_iter()
        .collect::<Option<Vec<ResolvedPosition>>>()
        .and_then(measured_geometry)
        .map_or(TrackGeometry::NoValidPosition, TrackGeometry::Measured)
}

/// Measures a track over the positions its fixes are drawn at, so the geometry
/// describes the path the map draws. `None` for an empty track.
fn measured_geometry(resolved_positions: Vec<ResolvedPosition>) -> Option<MeasuredTrackGeometry> {
    let positions: Vec<(Latitude, Longitude)> = resolved_positions
        .iter()
        .map(|resolved| resolved.coordinates())
        .collect();
    let (first, rest) = positions.split_first()?;

    let bounding_box = GeoBounds::from_first_position_and_rest(*first, rest.iter().copied())
        .extended_to_the_encircled_pole(PoleWinding::of_track(positions.iter().copied()));

    Some(MeasuredTrackGeometry {
        bounding_box,
        merc_bounds: MercBounds::from(bounding_box),
        distance_km: Length::new::<kilometer>(path_distance_km(&positions)),
        point_set_diameter_m: Length::new::<meter>(point_set_diameter_m(&positions)),
        segment_length_range: segment_length_range_m(&positions).map(|(min_m, max_m)| {
            SegmentLengthRange {
                min: Length::new::<meter>(min_m),
                max: Length::new::<meter>(max_m),
            }
        }),
        resolved_positions,
    })
}

/// Optional file-level metadata carried from the recording's SDK metadata into
/// the built [`LoadedFile`]. All fields are absent for sources that have none.
#[derive(Debug, Clone, Default)]
pub struct FileMeta {
    pub title: Option<String>,
    pub device: Option<String>,
    pub notes: Option<String>,
    pub travel_mode: Option<TravelMode>,
}

impl From<&FileMetadata> for FileMeta {
    /// Recover the metadata inputs from an already-built [`FileMetadata`], so a
    /// re-segmentation preserves them without re-listing the field names.
    fn from(metadata: &FileMetadata) -> Self {
        Self {
            title: metadata.title.clone(),
            device: metadata.device.clone(),
            notes: metadata.notes.clone(),
            travel_mode: metadata.travel_mode.clone(),
        }
    }
}

/// A variant path a recording holds more than one event marker style for, and
/// how many it holds.
struct RepeatedEventMarkerStyle {
    variant_path: String,
    styles: usize,
}

impl fmt::Display for RepeatedEventMarkerStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            variant_path,
            styles,
        } = self;
        write!(f, "{variant_path:?}: {styles} styles")
    }
}

const REPEATED_EVENT_MARKER_STYLES: AlterationWording = AlterationWording {
    issue: "event marker variant path(s) with several styles",
    consequence: "Every marker on those paths is drawn with the last style the recording \
        holds for it: one style is kept per variant path.",
};

/// Keeps the last style a recording holds for each variant path, which is the
/// one every marker on that path is drawn with.
fn keep_the_last_event_marker_style_per_variant_path(
    styles: Vec<EventMarkerStyle>,
    load_warnings: &mut Vec<LoadWarning>,
) -> FxHashMap<String, EventMarkerStyle> {
    let mut kept: FxHashMap<String, EventMarkerStyle> = FxHashMap::default();
    let mut repeated: Vec<RepeatedEventMarkerStyle> = Vec::new();
    for style in styles {
        let Some(replaced) = kept.insert(style.variant_path.clone(), style) else {
            continue;
        };
        match repeated
            .iter_mut()
            .find(|entry| entry.variant_path == replaced.variant_path)
        {
            Some(entry) => entry.styles += 1,
            None => repeated.push(RepeatedEventMarkerStyle {
                variant_path: replaced.variant_path,
                styles: 2,
            }),
        }
    }
    load_warnings.extend(REPEATED_EVENT_MARKER_STYLES.load_warning(&repeated));
    kept
}

/// Segments `points` into tracks and builds a fully-populated `LoadedFile`.
///
/// Every fix whose recorded coordinates are out of range is placed between the
/// fixes that have a recorded position. A recording that holds no such fix
/// builds tracks with no geometry: they carry every fix the receiver wrote and
/// are drawn nowhere.
#[expect(
    clippy::expect_used,
    reason = "ranges from segment_tracks are always in-bounds and non-empty"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "constructor assembles all LoadedFile fields; no natural grouping to extract"
)]
pub fn build_loaded_file(
    filename: String,
    points: &[NavPoint],
    custom_markers: &[CustomMarker],
    event_markers: Vec<EventMarker>,
    event_marker_styles: Vec<EventMarkerStyle>,
    channels: &[Channel],
    config: &SegmentationConfig,
    source: FileSource,
    file_meta: FileMeta,
    mut load_warnings: Vec<LoadWarning>,
) -> LoadedFile {
    // A fix with no recorded position is placed across the whole recording
    // first. The per-track pass below refines it from its own track's fixes,
    // and a track holding no fix with a position keeps this placement.
    let mut placements = recorded_placements(points);
    place_fixes(points, &mut placements, UnmeasuredFix::CoordinateOutOfRange);

    let event_marker_styles =
        keep_the_last_event_marker_style_per_variant_path(event_marker_styles, &mut load_warnings);

    let ranges = segment_tracks(points, &config.track_layout);

    let mut loaded_tracks: Vec<LoadedTrack> = ranges
        .into_iter()
        .enumerate()
        .map(|(track_idx, range)| {
            let track_points_slice = points
                .get(range.clone())
                .expect("ranges from segment_tracks are in bounds");

            let track_points: vec1::Vec1<NavPoint> =
                vec1::Vec1::try_from_vec(track_points_slice.to_vec())
                    .expect("segment_tracks produces only non-empty ranges");

            // The fixes the receiver did not measure are placed first: the
            // geometry, the generated markers and the LOD levels below all
            // read the positions they landed at.
            let mut track_placements: FixPlacements = placements
                .get(range)
                .expect("ranges from segment_tracks are in bounds")
                .to_vec();
            place_track_fixes(&track_points, &mut track_placements);
            let geometry = track_geometry(track_placements);

            let placed_points = geometry.measured().and_then(|measured| {
                PlacedPoints::new(&track_points, &measured.resolved_positions)
            });

            let track_time_range = time_range_spanning_every_fix(&track_points);

            let track_custom: Vec<CustomMarker> = custom_markers
                .iter()
                .filter(|m| track_time_range.contains(m.time))
                .cloned()
                .collect();

            // Each channel keeps only the samples in this track's time range.
            // Tracks are time-disjoint, so a sample lands in at most one
            // track. A channel with no samples here is dropped from this
            // track.
            let track_channels: Vec<Channel> = channels
                .iter()
                .map(|c| c.slice_time_range(track_time_range.start, track_time_range.end))
                .filter(|c| !c.times.is_empty())
                .collect();

            let track_generated = placed_points.map_or_else(Vec::new, |placed| {
                detect_generated_markers(placed, &config.generated_markers)
            });
            let lod = placed_points
                .map(crate::lod::build_track_lod)
                .unwrap_or_default();
            let sat_label_anchors =
                placed_points.map_or_else(Vec::new, crate::sat_label::build_sat_label_anchors);

            let metadata = compute_track_metadata(
                track_idx + 1,
                &track_points,
                &track_custom,
                &track_generated,
            );

            let track_points_vec = track_points.into_vec();

            LoadedTrack {
                metadata,
                points: track_points_vec,
                geometry,
                lod,
                sat_label_anchors,
                custom_markers: track_custom,
                generated_markers: track_generated,
                event_markers: Vec::new(),
                channels: track_channels,
            }
        })
        .collect();

    // Assign event markers to tracks by timestamp. Orphans go into LoadedFile.
    let mut orphaned_event_markers = Vec::new();
    for em in event_markers {
        let mut em = Some(em);
        for track in &mut loaded_tracks {
            let start = track.metadata.time_range.start;
            let end = track.metadata.time_range.end;
            if em
                .as_ref()
                .is_some_and(|e| e.time >= start && e.time <= end)
            {
                track.event_markers.push(
                    #[expect(clippy::expect_used, reason = "just checked is_some")]
                    em.take().expect("checked above"),
                );
                break;
            }
        }
        if let Some(unassigned) = em {
            orphaned_event_markers.push(unassigned);
        }
    }
    // Back-fill event_marker_count now that assignment is done.
    for track in &mut loaded_tracks {
        track.metadata.event_marker_count = track.event_markers.len();
    }

    // Channel samples that fell in a between-track gap (e.g. a sensor still
    // logging while GPS had no fix) belong to no track and were dropped above.
    // Surface the loss.
    let input_samples: usize = channels.iter().map(|c| c.times.len()).sum();
    let kept_samples: usize = loaded_tracks
        .iter()
        .flat_map(|t| &t.channels)
        .map(|c| c.times.len())
        .sum();
    if let Some(dropped) = input_samples.checked_sub(kept_samples).filter(|&d| d > 0) {
        load_warnings.push(LoadWarning {
            count: u32::try_from(dropped).unwrap_or(u32::MAX),
            issue: "channel sample(s) outside every track".to_owned(),
            description: "Sensor samples whose timestamp fell between tracks (no \
                nav fix covers that time) were dropped and are not shown."
                .to_owned(),
        });
    }

    let total_distance = loaded_tracks
        .iter()
        .filter_map(|t| Some(t.geometry.measured()?.distance_km))
        .reduce(|total, distance| total + distance)
        .map_or(TotalDistance::NoMeasuredTrack, TotalDistance::Measured);
    let total_duration = loaded_tracks
        .iter()
        .fold(Duration::zero(), |acc, t| acc + t.metadata.duration);

    let file_fix_stats = {
        let mut time_with_fix = Duration::zero();
        let mut time_without_fix = Duration::zero();
        let mut fix_loss_count: u32 = 0;
        let mut max_continuous_no_fix = Duration::zero();
        let mut has_any = false;
        for track in &loaded_tracks {
            if let Some(s) = track.metadata.fix_stats {
                has_any = true;
                time_with_fix += s.time_with_fix;
                time_without_fix += s.time_without_fix;
                fix_loss_count = fix_loss_count.saturating_add(s.fix_loss_count);
                if s.max_continuous_no_fix > max_continuous_no_fix {
                    max_continuous_no_fix = s.max_continuous_no_fix;
                }
            }
        }
        if has_any {
            Some(FixStats {
                time_with_fix,
                time_without_fix,
                fix_loss_count,
                max_continuous_no_fix,
            })
        } else {
            None
        }
    };

    let fallback = DateTime::<Utc>::UNIX_EPOCH;
    let file_time_range = match (loaded_tracks.first(), loaded_tracks.last()) {
        (Some(first), Some(last)) => TimeRange::new(
            first.metadata.time_range.start,
            last.metadata.time_range.end,
        ),
        _ => TimeRange::new(fallback, fallback),
    };

    LoadedFile {
        metadata: FileMetadata {
            filename,
            total_distance,
            total_duration,
            time_range: file_time_range,
            fix_stats: file_fix_stats,
            title: file_meta.title,
            device: file_meta.device,
            notes: file_meta.notes,
            travel_mode: file_meta.travel_mode,
        },
        tracks: loaded_tracks,
        event_marker_styles,
        orphaned_event_markers,
        source,
        load_warnings,
    }
}

/// Reassemble file-level channels from a file's per-track channel slices, for
/// re-segmentation. Concatenates each channel's samples across tracks (in track
/// order, which is time order) and returns them sorted by name, mirroring the
/// order a fresh load produces.
pub fn reassemble_channels(tracks: &[LoadedTrack]) -> Vec<Channel> {
    let mut by_name: Vec<Channel> = Vec::new();
    for track in tracks {
        for channel in &track.channels {
            if let Some(existing) = by_name.iter_mut().find(|c| c.name == channel.name) {
                existing.times.extend_from_slice(&channel.times);
                existing.values.extend_from_slice(&channel.values);
            } else {
                by_name.push(channel.clone());
            }
        }
    }
    by_name.sort_by(|a, b| a.name.cmp(&b.name));
    by_name
}

/// A fix the receiver did not measure at the coordinates it holds, and what
/// anchors the position the builder places it at.
#[derive(Clone, Copy)]
enum UnmeasuredFix {
    /// A fix with no heading, anchored by the fixes with a satellite in fix:
    /// the coordinates a receiver reports without a heading are often its own
    /// dead reckoning, as is a position it reports with nothing in fix.
    Ghost,
    /// A fix with a recorded latitude or longitude outside its range, anchored
    /// by every fix that has a recorded position: it holds none of its own to
    /// fall back on.
    CoordinateOutOfRange,
}

impl UnmeasuredFix {
    fn is_target(self, point: &NavPoint) -> bool {
        match self {
            Self::Ghost => point.tpv.position().is_some() && point.tpv.heading().is_none(),
            Self::CoordinateOutOfRange => point.tpv.position().is_none(),
        }
    }

    fn is_anchor(self, point: &NavPoint) -> bool {
        match self {
            Self::Ghost => point.tpv.position().is_some() && point.fix_count() > 0,
            Self::CoordinateOutOfRange => point.tpv.position().is_some(),
        }
    }

    /// Where `point` falls between its anchors: the share of their time span
    /// that has passed at its own timestamp, along the great circle between
    /// them.
    ///
    /// `None` leaves the point where it is, which is what a ghost fix does
    /// when nothing anchors it and when its anchors stamp the same instant,
    /// spanning no time to place it in.
    fn placement(
        self,
        points: &[NavPoint],
        placements: &FixPlacements,
        point: &NavPoint,
        (preceding, following): (Option<usize>, Option<usize>),
    ) -> Option<ResolvedPosition> {
        let anchor = |index: Option<usize>| {
            let index = index?;
            Some((points.get(index)?, (*placements.get(index)?)?))
        };
        let before = anchor(preceding);
        let after = anchor(following);

        let interpolated = match (before, after) {
            (Some((before, before_position)), Some((after, after_position))) => {
                let anchor_span_secs = (after.tpv.time() - before.tpv.time()).as_seconds_f64();
                let elapsed_secs = (point.tpv.time() - before.tpv.time()).as_seconds_f64();
                (anchor_span_secs > 0.0).then(|| {
                    GreatCircleArc {
                        start: before_position.coordinates(),
                        end: after_position.coordinates(),
                    }
                    .position_at_ratio(elapsed_secs / anchor_span_secs)
                })
            }
            (Some((_, position)), None) | (None, Some((_, position))) => {
                Some(position.coordinates())
            }
            (None, None) => None,
        };

        let (latitude, longitude) = match self {
            Self::Ghost => interpolated,
            // Any anchor places a fix out of range better than none does: it
            // has no recorded position to be left at.
            Self::CoordinateOutOfRange => interpolated
                .or_else(|| before.or(after).map(|(_, position)| position.coordinates())),
        }?;
        Some(ResolvedPosition::interpolated(latitude, longitude))
    }
}

/// Places every fix of `kind` at the position its anchors give it, writing it
/// into `placements`. What the receiver recorded stays on [`NavPoint::tpv`],
/// and the placed position is what the renderers draw.
///
/// Running this again over the same placements repeats the same placement:
/// both the targets and the anchors are read off the recorded coordinates.
/// Runs in O(n) over the points.
fn place_fixes(points: &[NavPoint], placements: &mut FixPlacements, kind: UnmeasuredFix) {
    let anchors = nearest_anchors(points, |point| kind.is_anchor(point));

    let placed: Vec<(usize, ResolvedPosition)> = points
        .iter()
        .enumerate()
        .zip(&anchors)
        .filter(|((_, point), _)| kind.is_target(point))
        .filter_map(|((index, point), anchors)| {
            Some((index, kind.placement(points, placements, point, *anchors)?))
        })
        .collect();

    for (index, position) in placed {
        if let Some(placement) = placements.get_mut(index) {
            *placement = Some(position);
        }
    }
}

/// For each point, the nearest anchor before it and the nearest one after it.
fn nearest_anchors(
    points: &[NavPoint],
    is_anchor: impl Fn(&NavPoint) -> bool,
) -> Vec<(Option<usize>, Option<usize>)> {
    let mut preceding: Vec<Option<usize>> = Vec::with_capacity(points.len());
    let mut latest: Option<usize> = None;
    for (index, point) in points.iter().enumerate() {
        preceding.push(latest);
        if is_anchor(point) {
            latest = Some(index);
        }
    }

    let mut following: Vec<Option<usize>> = Vec::with_capacity(points.len());
    let mut earliest: Option<usize> = None;
    for (index, point) in points.iter().enumerate().rev() {
        following.push(earliest);
        if is_anchor(point) {
            earliest = Some(index);
        }
    }
    following.reverse();

    preceding.into_iter().zip(following).collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeZone;
    use geotrace_sdk_units::Unit;
    use gt_types::coordinates::{Latitude, Longitude, RecordedLatitude, RecordedLongitude};
    use gt_types::markers::{MarkerColor, MarkerIcon};
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::{GpsTime, SysTime};
    use gt_types::tpv::TimePositionVelocity;
    use rstest::rstest;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    use super::*;

    /// `points` taken as a track of their own, each fix with where the builder
    /// places it. `None` for a track it places no fix of.
    fn placed<'a>(points: &'a [NavPoint], geometry: &'a TrackGeometry) -> Option<PlacedPoints<'a>> {
        geometry
            .measured()
            .and_then(|measured| PlacedPoints::new(points, &measured.resolved_positions))
    }

    fn generated_markers_of(
        points: &[NavPoint],
        config: &GeneratedMarkerConfig,
    ) -> Vec<GeneratedMarker> {
        let geometry = measure_track_geometry(points);
        placed(points, &geometry)
            .map_or_else(Vec::new, |placed| detect_generated_markers(placed, config))
    }

    fn clock_discontinuities_of(points: &[NavPoint], sigmas: f64) -> Vec<GeneratedMarker> {
        let geometry = measure_track_geometry(points);
        placed(points, &geometry).map_or_else(Vec::new, |placed| {
            detect_clock_discontinuities(placed, sigmas, &[])
        })
    }

    /// A point at GPS second `gps_secs` whose system clock is `sys_ahead_ms`
    /// ahead of GPS (so the GPS−system offset is `-sys_ahead_ms`).
    fn point_with_sys(gps_secs: i64, sys_ahead_ms: i64) -> NavPoint {
        let gps = GpsTime::from_utc(Utc.timestamp_opt(gps_secs, 0).single().expect("valid"));
        let sys = SysTime::from_utc(
            Utc.timestamp_millis_opt(gps_secs * 1000 + sys_ahead_ms)
                .single()
                .expect("valid"),
        );
        let tpv = TimePositionVelocity::builder()
            .time(gps)
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .sys_time(sys)
            .build();
        NavPoint::new(tpv, None)
    }

    #[test]
    fn clock_discontinuity_flags_suspend_boundary_once() {
        // Steady ~300 ms offset, then one sample whose system clock has jumped
        // ~2 h ahead (the device resumed from suspend) - the mortmobil.gtd case.
        let two_hours_ms = 2 * 3600 * 1000;
        let points = vec![
            point_with_sys(1000, 300),
            point_with_sys(1001, 300),
            point_with_sys(1002, 300),
            point_with_sys(1003, 300),
            point_with_sys(1004, 300 + two_hours_ms),
        ];
        let markers = clock_discontinuities_of(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS);
        assert_eq!(
            markers.len(),
            1,
            "exactly one discontinuity at the boundary"
        );
        let marker = markers.first().expect("one marker");
        assert!(matches!(
            marker.kind,
            GeneratedMarkerKind::ClockDiscontinuity { .. }
        ));
        if let GeneratedMarkerKind::ClockDiscontinuity { step } = marker.kind {
            // System clock jumped 2 h ahead, so GPS−system dropped by 2 h.
            assert_eq!(step.num_milliseconds(), -two_hours_ms);
        }
        assert_eq!(
            marker.time,
            Utc.timestamp_opt(1004, 0).single().expect("valid")
        );
    }

    /// Steady 234 ms offset with one sample carrying a 1 h 09 m recording gap -
    /// the `gnss.h5.gtd` case, where the receiver reported its pre-gap GPS epoch
    /// for the first fix after resuming.
    fn resume_from_gap_points() -> Vec<NavPoint> {
        vec![
            point_with_sys(1000, 210),
            point_with_sys(1001, 227),
            point_with_sys(1002, 240),
            point_with_sys(1003, 234),
            point_with_sys(1004, 4_127_054),
            point_with_sys(1005, 240),
            point_with_sys(1006, 215),
            point_with_sys(1007, 235),
        ]
    }

    #[test]
    fn an_excursion_is_one_marker_not_a_pair_of_discontinuities() {
        let markers =
            generated_markers_of(&resume_from_gap_points(), &GeneratedMarkerConfig::default());
        let [marker] = markers.as_slice() else {
            panic!("expected exactly one marker, got {}", markers.len());
        };
        let GeneratedMarkerKind::ClockOffsetExcursion {
            deviation,
            offset,
            samples,
        } = marker.kind
        else {
            panic!("expected a clock offset excursion, got {:?}", marker.kind);
        };
        assert_eq!(offset.num_milliseconds(), -4_127_054);
        assert_eq!(deviation.num_milliseconds(), -4_126_820);
        assert_eq!(samples, 1);
        assert_eq!(
            marker.time,
            Utc.timestamp_opt(1004, 0).single().expect("valid"),
            "placed at the sample that departed furthest"
        );
    }

    #[test]
    fn excursion_detection_off_leaves_the_discontinuity_markers() {
        let config = GeneratedMarkerConfig {
            detect_clock_offset_excursions: false,
            ..GeneratedMarkerConfig::default()
        };
        let markers = generated_markers_of(&resume_from_gap_points(), &config);
        assert!(
            markers.is_empty(),
            "the excursion sample stays out of the step series either way, so the \
             departure is never re-reported as a pair of jumps: {markers:?}"
        );
    }

    #[test]
    fn a_permanent_offset_step_stays_a_discontinuity() {
        let mut points: Vec<NavPoint> = (0..6).map(|i| point_with_sys(1000 + i, 200)).collect();
        points.extend((6..12).map(|i| point_with_sys(1000 + i, 3_600_000)));
        let markers = generated_markers_of(&points, &GeneratedMarkerConfig::default());
        let [marker] = markers.as_slice() else {
            panic!("expected exactly one marker, got {}", markers.len());
        };
        assert!(matches!(
            marker.kind,
            GeneratedMarkerKind::ClockDiscontinuity { .. }
        ));
    }

    #[test]
    fn clock_discontinuity_ignores_normal_jitter() {
        let points = vec![
            point_with_sys(1000, 300),
            point_with_sys(1001, 305),
            point_with_sys(1002, 298),
            point_with_sys(1003, 302),
        ];
        assert!(clock_discontinuities_of(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS).is_empty());
    }

    #[test]
    fn clock_discontinuity_ignores_large_but_steady_offset() {
        // A host clock that drifted far (e.g. parked underground for days) but
        // is internally consistent across the track is NOT an outlier - the
        // data-aware median makes that the norm, so nothing is flagged.
        let big = 5 * 60 * 1000; // 5 minutes of steady offset
        let points = vec![
            point_with_sys(1000, big),
            point_with_sys(1001, big + 4),
            point_with_sys(1002, big - 3),
            point_with_sys(1003, big + 2),
            point_with_sys(1004, big - 5),
        ];
        assert!(clock_discontinuities_of(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS).is_empty());
    }

    #[test]
    fn clock_discontinuity_needs_enough_samples() {
        // Below MIN_CLOCK_SAMPLES, detection is skipped even with an obvious 2 h
        // jump on the last sample - too few samples for a robust estimate.
        let two_hours_ms = 2 * 3600 * 1000;
        for count in 0..MIN_CLOCK_SAMPLES {
            let points: Vec<NavPoint> = (0..count)
                .map(|i| {
                    let ahead = if i + 1 == count {
                        300 + two_hours_ms
                    } else {
                        300
                    };
                    point_with_sys(1000 + i as i64, ahead)
                })
                .collect();
            assert!(
                clock_discontinuities_of(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS).is_empty(),
                "detection must be skipped with {count} samples (< {MIN_CLOCK_SAMPLES})"
            );
        }
    }

    fn make_point_at(t: i64) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(55.0))
            .lon(gt_types::coordinates::Longitude::new(12.0))
            .heading(Angle::new::<degree>(0.0))
            .build();
        NavPoint::new(tpv, None)
    }

    fn make_point_at_pos(t: i64, lat: f64, lon: f64) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(lat))
            .lon(gt_types::coordinates::Longitude::new(lon))
            .heading(Angle::new::<degree>(0.0))
            .build();
        NavPoint::new(tpv, None)
    }

    #[test]
    fn segment_tracks_empty_input() {
        assert!(segment_tracks(&[], &TrackLayoutConfig::default()).is_empty());
    }

    #[test]
    fn segment_tracks_single_point() {
        let pts = vec![make_point_at(0)];
        let ranges = segment_tracks(&pts, &TrackLayoutConfig::default());
        assert_eq!(ranges, vec![0..1]);
    }

    #[test]
    fn segment_tracks_all_within_five_minutes() {
        let pts: Vec<NavPoint> = (0..5).map(|i| make_point_at(i * 60)).collect();
        let ranges = segment_tracks(&pts, &TrackLayoutConfig::default());
        assert_eq!(ranges, vec![0..5]);
    }

    #[rstest]
    #[case::forward_step_below_the_split_gap(TrackSplitRule::StepInEitherDirection, 299, vec![0..2])]
    #[case::forward_step_at_the_split_gap(TrackSplitRule::StepInEitherDirection, 300, vec![0..1, 1..2])]
    #[case::backward_step_below_the_split_gap(TrackSplitRule::StepInEitherDirection, -299, vec![0..2])]
    #[case::backward_step_at_the_split_gap(TrackSplitRule::StepInEitherDirection, -300, vec![0..1, 1..2])]
    #[case::forward_step_at_the_split_gap_under_forward_gaps_only(TrackSplitRule::ForwardGapOnly, 300, vec![0..1, 1..2])]
    #[case::backward_step_at_the_split_gap_under_forward_gaps_only(TrackSplitRule::ForwardGapOnly, -300, vec![0..2])]
    fn segment_tracks_applies_the_configured_split_rule(
        #[case] track_split_rule: TrackSplitRule,
        #[case] step_seconds: i64,
        #[case] expected_ranges: Vec<Range<usize>>,
    ) {
        let pts = vec![make_point_at(1_000), make_point_at(1_000 + step_seconds)];
        let config = TrackLayoutConfig {
            track_split_rule,
            ..TrackLayoutConfig::default()
        };

        let ranges = segment_tracks(&pts, &config);

        assert_eq!(ranges, expected_ranges);
    }

    #[test]
    fn segment_tracks_one_gap_gives_two_trips() {
        let pts = vec![
            make_point_at(0),
            make_point_at(60),
            make_point_at(3600), // +1 h gap
            make_point_at(3660),
        ];
        let ranges = segment_tracks(&pts, &TrackLayoutConfig::default());
        assert_eq!(ranges, vec![0..2, 2..4]);
    }

    #[test]
    fn segment_tracks_multiple_gaps() {
        let pts = vec![
            make_point_at(0),
            make_point_at(3600), // gap
            make_point_at(7200), // gap
        ];
        let ranges = segment_tracks(&pts, &TrackLayoutConfig::default());
        assert_eq!(ranges, vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn compute_track_metadata_basic() {
        let pts = vec1::vec1![
            make_point_at_pos(0, 55.0, 12.0),
            make_point_at_pos(3600, 55.1, 12.1), // 1 h later, ~13 km away
        ];
        let meta = compute_track_metadata(1, &pts, &[], &[]);
        assert_eq!(meta.index, 1);
        assert_eq!(meta.tpv_count, 2);
        assert_eq!(meta.duration.num_seconds(), 3600);
        assert!(!meta.has_custom_markers);
        assert_eq!(meta.satellite_report_count, 0);

        let distance_km = measure_track_geometry(&pts)
            .measured()
            .expect("both fixes have a recorded position")
            .distance_km;
        assert!(
            distance_km > Length::new::<kilometer>(5.0),
            "expected > 5 km, got {distance_km:?}"
        );
    }

    #[test]
    fn compute_track_metadata_single_point_has_zero_duration() {
        let pts = vec1::vec1![make_point_at_pos(0, 55.0, 12.0)];
        let meta = compute_track_metadata(1, &pts, &[], &[]);
        assert_eq!(meta.duration.num_seconds(), 0);

        let distance_km = measure_track_geometry(&pts)
            .measured()
            .expect("the fix has a recorded position")
            .distance_km;
        assert_eq!(distance_km, Length::new::<kilometer>(0.0));
    }

    #[test]
    fn build_loaded_file_empty_points() {
        let f = build_loaded_file(
            "test.gtd".to_owned(),
            &[],
            &[],
            vec![],
            vec![],
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("test.gtd")),
            FileMeta::default(),
            vec![],
        );
        assert!(f.tracks.is_empty());
        assert_eq!(f.metadata.filename, "test.gtd");
    }

    const FIRST_STYLE_COLOR: MarkerColor = MarkerColor::new(0x11, 0x22, 0x33);
    const SECOND_STYLE_COLOR: MarkerColor = MarkerColor::new(0x44, 0x55, 0x66);
    const THIRD_STYLE_COLOR: MarkerColor = MarkerColor::new(0x77, 0x88, 0x99);

    fn event_marker_style(variant_path: &str, color: MarkerColor) -> EventMarkerStyle {
        EventMarkerStyle {
            variant_path: variant_path.to_owned(),
            icon: MarkerIcon::Pin,
            color,
        }
    }

    #[rstest]
    #[case::two_styles(vec![FIRST_STYLE_COLOR, SECOND_STYLE_COLOR], SECOND_STYLE_COLOR)]
    #[case::three_styles(
        vec![FIRST_STYLE_COLOR, SECOND_STYLE_COLOR, THIRD_STYLE_COLOR],
        THIRD_STYLE_COLOR
    )]
    fn several_event_marker_styles_for_one_variant_path_keep_the_last_and_warn(
        #[case] written_colors: Vec<MarkerColor>,
        #[case] expected_color: MarkerColor,
    ) {
        let written_count = written_colors.len();
        let mut styles: Vec<EventMarkerStyle> = written_colors
            .iter()
            .map(|color| event_marker_style("power/boot", *color))
            .collect();
        styles.push(event_marker_style("power/shutdown", FIRST_STYLE_COLOR));

        let file = build_loaded_file(
            "test.gtd".to_owned(),
            &[make_point_at(0), make_point_at(30)],
            &[],
            vec![],
            styles,
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("test.gtd")),
            FileMeta::default(),
            vec![],
        );

        assert_eq!(
            file.event_marker_styles
                .get("power/boot")
                .map(|style| style.color),
            Some(expected_color)
        );
        assert_eq!(
            file.event_marker_styles
                .get("power/shutdown")
                .map(|style| style.color),
            Some(FIRST_STYLE_COLOR)
        );
        let expected_description = format!(
            "\"power/boot\": {written_count} styles. Every marker on those paths is drawn \
             with the last style the recording holds for it: one style is kept per variant \
             path."
        );
        assert_eq!(
            file.load_warnings
                .iter()
                .map(|warning| (
                    warning.count,
                    warning.issue.as_str(),
                    warning.description.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![(
                1,
                "event marker variant path(s) with several styles",
                expected_description.as_str()
            )]
        );
    }

    #[test]
    fn build_loaded_file_two_trips() {
        let pts = vec![
            make_point_at(0),
            make_point_at(60),
            make_point_at(3600), // gap → new track
            make_point_at(3660),
        ];
        let f = build_loaded_file(
            "ride.gtd".to_owned(),
            &pts,
            &[],
            vec![],
            vec![],
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("ride.gtd")),
            FileMeta::default(),
            vec![],
        );
        assert_eq!(f.tracks.len(), 2);
        assert_eq!(f.tracks[0].points.len(), 2);
        assert_eq!(f.tracks[1].points.len(), 2);
        assert_eq!(f.tracks[0].metadata.index, 1);
        assert_eq!(f.tracks[1].metadata.index, 2);
    }

    fn utc(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn channels_partition_to_tracks_by_timestamp() {
        // Two tracks: [0, 60] and [3600, 3660]. A scalar channel with samples in
        // track 1 (0, 30), the gap (1800), and track 2 (3600).
        let pts = vec![
            make_point_at(0),
            make_point_at(60),
            make_point_at(3600),
            make_point_at(3660),
        ];
        let channel = Channel {
            name: "incline".to_owned(),
            unit: Some(Unit::DEG.into()),
            period: None,
            description: None,
            components: vec![],
            times: vec![utc(0), utc(30), utc(1800), utc(3600)],
            values: vec![1.0, 2.0, 9.0, 3.0],
        };
        let f = build_loaded_file(
            "ride.gtd".to_owned(),
            &pts,
            &[],
            vec![],
            vec![],
            std::slice::from_ref(&channel),
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("ride.gtd")),
            FileMeta::default(),
            vec![],
        );
        assert_eq!(f.tracks.len(), 2);

        // Track 1 keeps the two in-range samples. The gap sample (1800) is
        // dropped.
        let t0 = &f.tracks[0].channels;
        assert_eq!(t0.len(), 1);
        assert_eq!(t0[0].name, "incline");
        assert_eq!(t0[0].times, vec![utc(0), utc(30)]);
        assert_eq!(t0[0].values, vec![1.0, 2.0]);

        // Track 2 keeps its single sample.
        let t1 = &f.tracks[1].channels;
        assert_eq!(t1.len(), 1);
        assert_eq!(t1[0].times, vec![utc(3600)]);

        // Reassembly concatenates the per-track slices back in time order.
        // The dropped gap sample stays dropped.
        let reassembled = reassemble_channels(&f.tracks);
        assert_eq!(reassembled.len(), 1);
        assert_eq!(reassembled[0].times, vec![utc(0), utc(30), utc(3600)]);
        assert_eq!(reassembled[0].values, vec![1.0, 2.0, 3.0]);

        // The one gap sample (1800) that landed in no track is surfaced as a warning.
        let warning = f
            .load_warnings
            .iter()
            .find(|w| w.issue.contains("outside every track"))
            .expect("dropped gap sample should be reported");
        assert_eq!(warning.count, 1);
    }

    #[test]
    fn a_vector_channel_partitions_and_reassembles_with_columns_aligned() {
        // Two tracks split at the 3600s gap. A 3-component accel channel with two
        // samples in track 1, one in the gap (dropped), and one in track 2.
        let pts = vec![
            make_point_at(0),
            make_point_at(60),
            make_point_at(3600),
            make_point_at(3660),
        ];
        let channel = Channel {
            name: "accel".to_owned(),
            unit: Some(Unit::G.into()),
            period: None,
            description: None,
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            times: vec![utc(0), utc(30), utc(1800), utc(3600)],
            // Row-major: four samples of (x, y, z).
            values: vec![
                0.0, 0.1, 1.0, // t=0
                1.0, 1.1, 2.0, // t=30
                8.0, 8.1, 8.2, // t=1800 (gap, dropped)
                3.0, 3.1, 4.0, // t=3600
            ],
        };
        let f = build_loaded_file(
            "ride.gtd".to_owned(),
            &pts,
            &[],
            vec![],
            vec![],
            std::slice::from_ref(&channel),
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("ride.gtd")),
            FileMeta::default(),
            vec![],
        );

        // Track 1 keeps rows 0 and 1 with their columns intact.
        let t0 = &f.tracks[0].channels[0];
        assert_eq!(t0.components, ["x", "y", "z"]);
        assert_eq!(t0.times, vec![utc(0), utc(30)]);
        assert_eq!(t0.values, vec![0.0, 0.1, 1.0, 1.0, 1.1, 2.0]);
        // Track 2 keeps the last row.
        assert_eq!(f.tracks[1].channels[0].values, vec![3.0, 3.1, 4.0]);

        // Reassembly restores the surviving rows in time order, columns aligned.
        let reassembled = reassemble_channels(&f.tracks);
        assert_eq!(reassembled[0].components, ["x", "y", "z"]);
        assert_eq!(reassembled[0].times, vec![utc(0), utc(30), utc(3600)]);
        assert_eq!(
            reassembled[0].values,
            vec![0.0, 0.1, 1.0, 1.0, 1.1, 2.0, 3.0, 3.1, 4.0]
        );
    }

    #[test]
    fn a_channel_absent_from_a_track_is_not_attached() {
        // Channel samples only in track 2's range. Track 1 has no channel.
        let pts = vec![make_point_at(0), make_point_at(3600), make_point_at(3660)];
        let channel = Channel {
            name: "accel".to_owned(),
            unit: None,
            period: None,
            description: None,
            components: vec![],
            times: vec![utc(3600), utc(3660)],
            values: vec![1.0, 2.0],
        };
        let f = build_loaded_file(
            "ride.gtd".to_owned(),
            &pts,
            &[],
            vec![],
            vec![],
            std::slice::from_ref(&channel),
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("ride.gtd")),
            FileMeta::default(),
            vec![],
        );
        assert_eq!(f.tracks.len(), 2);
        assert!(f.tracks[0].channels.is_empty());
        assert_eq!(f.tracks[1].channels.len(), 1);
    }

    fn make_point_with_fix(t: i64, fix_count_positive: bool) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(55.0))
            .lon(gt_types::coordinates::Longitude::new(12.0))
            .heading(Angle::new::<degree>(0.0))
            .build();
        let sats = Satellites::new(
            Some(time),
            None,
            vec![Satellite::new(
                Constellation::Gps,
                1,
                None,
                None,
                None,
                fix_count_positive,
            )],
        );
        NavPoint::new(tpv, Some(sats))
    }

    #[test]
    fn compute_fix_stats_empty() {
        assert!(compute_fix_stats(&[]).is_none());
    }

    #[test]
    fn compute_fix_stats_no_satellite_reports() {
        // make_point_at produces points with no satellite data (NavPoint::new(tpv, None))
        let pts = vec![make_point_at(0), make_point_at(60)];
        assert!(compute_fix_stats(&pts).is_none());
    }

    #[test]
    fn compute_fix_stats_single_sat_point_is_none() {
        let pts = vec![make_point_with_fix(0, true)];
        assert!(compute_fix_stats(&pts).is_none());
    }

    #[test]
    fn compute_fix_stats_all_with_fix() {
        // Two consecutive sat points, both in fix → all time_with_fix, no losses
        let pts = vec![make_point_with_fix(0, true), make_point_with_fix(60, true)];
        let stats = compute_fix_stats(&pts).expect("has satellite data");
        assert_eq!(stats.time_with_fix, Duration::seconds(60));
        assert_eq!(stats.time_without_fix, Duration::zero());
        assert_eq!(stats.fix_loss_count, 0);
        assert_eq!(stats.max_continuous_no_fix, Duration::zero());
    }

    #[test]
    fn compute_fix_stats_all_without_fix() {
        let pts = vec![
            make_point_with_fix(0, false),
            make_point_with_fix(120, false),
        ];
        let stats = compute_fix_stats(&pts).expect("has satellite data");
        assert_eq!(stats.time_with_fix, Duration::zero());
        assert_eq!(stats.time_without_fix, Duration::seconds(120));
        assert_eq!(stats.fix_loss_count, 0);
        assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
    }

    #[test]
    fn compute_fix_stats_fix_then_lost() {
        // fix 0→60, lost 60→180 → one loss, 120s without fix
        let pts = vec![
            make_point_with_fix(0, true),
            make_point_with_fix(60, false),
            make_point_with_fix(180, false),
        ];
        let stats = compute_fix_stats(&pts).expect("has satellite data");
        assert_eq!(stats.time_with_fix, Duration::seconds(60));
        assert_eq!(stats.time_without_fix, Duration::seconds(120));
        assert_eq!(stats.fix_loss_count, 1);
        assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
    }

    #[test]
    fn compute_fix_stats_multiple_losses() {
        // fix→lost→fix→lost pattern. Two separate no-fix stretches
        let pts = vec![
            make_point_with_fix(0, true),    // fix
            make_point_with_fix(100, false), // lost (100s with fix)
            make_point_with_fix(200, false), // still lost (100s without fix, streak=100)
            make_point_with_fix(300, true),  // regained (100s more without fix, streak=200)
            make_point_with_fix(400, false), // lost again (100s with fix)
            make_point_with_fix(450, false), // still lost (50s without fix, streak=50)
        ];
        let stats = compute_fix_stats(&pts).expect("has satellite data");
        assert_eq!(stats.time_with_fix, Duration::seconds(200));
        assert_eq!(stats.time_without_fix, Duration::seconds(250));
        assert_eq!(stats.fix_loss_count, 2);
        assert_eq!(stats.max_continuous_no_fix, Duration::seconds(200));
    }

    #[test]
    fn compute_fix_stats_ignores_points_without_sat_data() {
        // Gaps between sat-report points (no satellite data) are not counted in either bucket
        let pts = vec![
            make_point_with_fix(0, true),
            make_point_at(30), // no satellite data - ignored
            make_point_at(60), // no satellite data - ignored
            make_point_with_fix(90, true),
        ];
        let stats = compute_fix_stats(&pts).expect("has satellite data");
        // Interval 0→90 attributed to first sat point (has fix) = 90s with fix
        assert_eq!(stats.time_with_fix, Duration::seconds(90));
        assert_eq!(stats.time_without_fix, Duration::zero());
        assert_eq!(stats.fix_loss_count, 0);
    }

    fn make_real_fix(t: i64, lat: Latitude, lon: Longitude) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(lat)
            .lon(lon)
            .heading(Angle::new::<degree>(0.0))
            .build();
        let sats = Satellites::new(
            Some(time),
            None,
            vec![Satellite::new(
                Constellation::Gps,
                1,
                None,
                None,
                None,
                true,
            )],
        );
        NavPoint::new(tpv, Some(sats))
    }

    fn make_ghost(t: i64, lat: Latitude, lon: Longitude) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(lat)
            .lon(lon)
            .build();
        NavPoint::new(tpv, None)
    }

    /// Where the builder draws each fix of `points`, taken as a track of their
    /// own. Empty for a track it places no fix of.
    fn drawn_positions(points: &[NavPoint]) -> Vec<(Latitude, Longitude)> {
        measure_track_geometry(points)
            .measured()
            .map_or_else(Vec::new, |measured| {
                measured
                    .resolved_positions
                    .iter()
                    .map(|resolved| resolved.coordinates())
                    .collect()
            })
    }

    /// The recorded position of every fix, which is where a measured one is
    /// drawn.
    fn recorded_positions(points: &[NavPoint]) -> Vec<(Latitude, Longitude)> {
        points
            .iter()
            .filter_map(|point| point.tpv.position())
            .collect()
    }

    #[test]
    fn a_track_of_no_fixes_has_no_geometry() {
        assert_eq!(measure_track_geometry(&[]), TrackGeometry::NoValidPosition);
    }

    #[test]
    fn measured_fixes_stay_where_they_were_recorded() {
        let points = vec![
            make_real_fix(0, Latitude::new(55.0), Longitude::new(12.0)),
            make_real_fix(1, Latitude::new(55.1), Longitude::new(12.1)),
        ];

        assert_eq!(drawn_positions(&points), recorded_positions(&points));
    }

    #[test]
    fn a_ghost_fix_between_two_anchors_is_interpolated() {
        // Real fixes on the equator at t=0 (lon=0) and t=10 (lon=1), ghost at
        // t=5. The equator is a great circle, so the ghost lands at lon=0.5.
        let points = vec![
            make_real_fix(0, Latitude::new(0.0), Longitude::new(0.0)),
            make_ghost(5, Latitude::new(10.0), Longitude::new(10.0)),
            make_real_fix(10, Latitude::new(0.0), Longitude::new(1.0)),
        ];

        let (latitude, longitude) = drawn_positions(&points)[1];
        assert!(
            latitude.as_degrees().abs() < 1e-9,
            "latitude mismatch: {} vs 0.0",
            latitude.as_degrees(),
        );
        assert!(
            (longitude.as_degrees() - 0.5).abs() < 1e-9,
            "longitude mismatch: {} vs 0.5",
            longitude.as_degrees(),
        );
        assert_eq!(
            points[1].tpv.position(),
            Some((Latitude::new(10.0), Longitude::new(10.0))),
            "the recorded coordinates must survive interpolation"
        );
    }

    #[test]
    fn a_ghost_fix_before_the_first_anchor_snaps_to_it() {
        let points = vec![
            make_ghost(0, Latitude::new(10.0), Longitude::new(10.0)),
            make_real_fix(10, Latitude::new(55.0), Longitude::new(12.0)),
        ];

        assert_eq!(
            drawn_positions(&points)[0],
            (Latitude::new(55.0), Longitude::new(12.0))
        );
    }

    #[test]
    fn a_ghost_fix_after_the_last_anchor_snaps_to_it() {
        let points = vec![
            make_real_fix(0, Latitude::new(55.0), Longitude::new(12.0)),
            make_ghost(10, Latitude::new(10.0), Longitude::new(10.0)),
        ];

        assert_eq!(
            drawn_positions(&points)[1],
            (Latitude::new(55.0), Longitude::new(12.0))
        );
    }

    /// Placement is read from longitude alone in the tests below: every fix
    /// in them sits on the equator. 1e-9° is about 0.1 mm.
    const PLACEMENT_TOLERANCE_DEGREES: f64 = 1e-9;

    /// A fix with a position and a heading, but no satellite report: the
    /// receiver reported where it was but not what it tracked.
    fn measured_fix_without_a_satellite_report(secs: i64, lon_degrees: f64) -> NavPoint {
        let time = GpsTime::from_utc(
            Utc.timestamp_opt(secs, 0)
                .single()
                .expect("valid timestamp"),
        );
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(0.0))
            .lon(Longitude::new(lon_degrees))
            .heading(Angle::new::<degree>(90.0))
            .build();
        NavPoint::new(tpv, None)
    }

    /// A fix the receiver wrote a latitude of NaN for. Its heading is present,
    /// leaving the unusable coordinate as the only reason to place it. A
    /// position kept as recorded is distinguishable from a placed one: its
    /// longitude is far from the fixes around it.
    fn fix_without_a_recorded_position(secs: i64) -> NavPoint {
        let time = GpsTime::from_utc(
            Utc.timestamp_opt(secs, 0)
                .single()
                .expect("valid timestamp"),
        );
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(RecordedLatitude::from_degrees(f64::NAN))
            .lon(RecordedLongitude::from_degrees(88.0))
            .heading(Angle::new::<degree>(90.0))
            .build();
        NavPoint::new(tpv, None)
    }

    #[rstest]
    #[case::between_two_measured_fixes(
        vec![
            measured_fix_without_a_satellite_report(0, 0.0),
            fix_without_a_recorded_position(5),
            measured_fix_without_a_satellite_report(10, 10.0),
        ],
        vec![0.0, 5.0, 10.0]
    )]
    #[case::before_the_first_measured_fix(
        vec![
            fix_without_a_recorded_position(0),
            measured_fix_without_a_satellite_report(10, 10.0),
            measured_fix_without_a_satellite_report(20, 20.0),
        ],
        vec![10.0, 10.0, 20.0]
    )]
    #[case::after_the_last_measured_fix(
        vec![
            measured_fix_without_a_satellite_report(0, 0.0),
            measured_fix_without_a_satellite_report(10, 10.0),
            fix_without_a_recorded_position(20),
        ],
        vec![0.0, 10.0, 10.0]
    )]
    #[case::a_run_of_three_spreads_over_the_time_they_span(
        vec![
            measured_fix_without_a_satellite_report(0, 0.0),
            fix_without_a_recorded_position(2),
            fix_without_a_recorded_position(5),
            fix_without_a_recorded_position(8),
            measured_fix_without_a_satellite_report(10, 10.0),
        ],
        vec![0.0, 2.0, 5.0, 8.0, 10.0]
    )]
    fn a_fix_without_a_recorded_position_is_placed_from_the_fixes_around_it(
        #[case] points: Vec<NavPoint>,
        #[case] expected_longitudes: Vec<f64>,
    ) {
        let drawn_longitudes: Vec<f64> = drawn_positions(&points)
            .into_iter()
            .map(|(_, longitude)| longitude.as_degrees())
            .collect();
        assert_eq!(drawn_longitudes.len(), expected_longitudes.len());
        for (index, (drawn, expected)) in drawn_longitudes
            .iter()
            .zip(&expected_longitudes)
            .enumerate()
        {
            assert!(
                (drawn - expected).abs() < PLACEMENT_TOLERANCE_DEGREES,
                "fix {index} drawn at lon {drawn}, expected {expected}"
            );
        }
    }

    /// A track whose every fix is out of range holds no anchor of its own, and
    /// the fixes of the recording's other tracks place it: 3610 s is halfway
    /// between the fixes at 10 s (lon 10) and 7210 s (lon 20).
    #[test]
    fn a_track_without_a_position_is_placed_from_the_rest_of_the_recording() {
        let points = vec![
            measured_fix_without_a_satellite_report(0, 0.0),
            measured_fix_without_a_satellite_report(10, 10.0),
            fix_without_a_recorded_position(3610),
            measured_fix_without_a_satellite_report(7210, 20.0),
        ];

        let file = build_loaded_file(
            "out_of_range.gtd".to_owned(),
            &points,
            &[],
            vec![],
            vec![],
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("out_of_range.gtd")),
            FileMeta::default(),
            vec![],
        );

        let drawn = file
            .tracks
            .get(1)
            .and_then(|track| track.placed_points()?.get(0))
            .expect("the middle fix is a track of its own");
        let longitude = drawn.resolved_position().1.as_degrees();
        assert!(
            (longitude - 15.0).abs() < PLACEMENT_TOLERANCE_DEGREES,
            "drawn at lon {longitude}, expected 15"
        );
    }

    #[test]
    fn track_metadata_counts_the_fixes_whose_recorded_position_is_out_of_range() {
        let points = vec1::vec1![
            measured_fix_without_a_satellite_report(0, 0.0),
            fix_without_a_recorded_position(5),
            measured_fix_without_a_satellite_report(10, 10.0),
        ];

        let metadata = compute_track_metadata(1, &points, &[], &[]);

        assert_eq!(metadata.invalid_position_count, 1);
    }

    #[test]
    fn ghost_fixes_with_no_anchor_stay_where_they_were_recorded() {
        let points = vec![
            make_ghost(0, Latitude::new(55.0), Longitude::new(12.0)),
            make_ghost(5, Latitude::new(56.0), Longitude::new(13.0)),
        ];

        assert_eq!(drawn_positions(&points), recorded_positions(&points));
    }

    #[test]
    fn build_loaded_file_file_fix_stats_aggregates_tracks() {
        // Default split gap is 300 s, so consecutive points must be < 300 s apart to stay in
        // the same track. Track 1: t=0 (fix)→t=60 (no-fix)→t=180 (no-fix). One loss, max=120s.
        // Track 2 (after a 9820s gap): t=10000 (fix)→t=10060 (no-fix)→t=10120 (fix). One loss,
        // max=60s.
        // sum(120, 60) = 180s != max(120, 60) = 120s, so the assertion below distinguishes
        // "max across tracks" from "sum across tracks".
        let pts = vec![
            make_point_with_fix(0, true),
            make_point_with_fix(60, false),
            make_point_with_fix(180, false),   // end of track 1
            make_point_with_fix(10_000, true), // large gap → new track 2
            make_point_with_fix(10_060, false),
            make_point_with_fix(10_120, true),
        ];
        let f = build_loaded_file(
            "test.gtd".to_owned(),
            &pts,
            &[],
            vec![],
            vec![],
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("test.gtd")),
            FileMeta::default(),
            vec![],
        );
        assert_eq!(f.tracks.len(), 2, "expected two tracks");
        let stats = f.metadata.fix_stats.expect("fix stats should be present");
        assert_eq!(stats.time_with_fix, Duration::seconds(60 + 60));
        assert_eq!(stats.time_without_fix, Duration::seconds(120 + 60));
        assert_eq!(stats.fix_loss_count, 2);
        // max taken across tracks, not summed
        assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
    }

    #[test]
    fn build_loaded_file_carries_file_meta() {
        let pts = vec![make_point_with_fix(0, true), make_point_with_fix(60, true)];
        let file_meta = FileMeta {
            title: Some("Morning ride".to_owned()),
            device: Some("uBlox F9P".to_owned()),
            notes: Some("cross-town".to_owned()),
            travel_mode: Some(TravelMode::Bicycle),
        };
        let f = build_loaded_file(
            "ride.gtd".to_owned(),
            &pts,
            &[],
            vec![],
            vec![],
            &[],
            &SegmentationConfig::default(),
            FileSource::GtdPath(PathBuf::from("ride.gtd")),
            file_meta,
            vec![],
        );
        assert_eq!(f.metadata.title.as_deref(), Some("Morning ride"));
        assert_eq!(f.metadata.device.as_deref(), Some("uBlox F9P"));
        assert_eq!(f.metadata.notes.as_deref(), Some("cross-town"));
        assert_eq!(f.metadata.travel_mode, Some(TravelMode::Bicycle));

        // Round-trip: rebuilding from the built metadata (the re-segmentation
        // path) preserves the fields.
        let recovered = FileMeta::from(&f.metadata);
        assert_eq!(recovered.title.as_deref(), Some("Morning ride"));
        assert_eq!(recovered.device.as_deref(), Some("uBlox F9P"));
        assert_eq!(recovered.notes.as_deref(), Some("cross-town"));
        assert_eq!(recovered.travel_mode, Some(TravelMode::Bicycle));
    }

    proptest::proptest! {
        /// Invariant: `time_with_fix + time_without_fix` equals the sum of all
        /// intervals between consecutive satellite-report points, regardless of
        /// fix pattern or gap sizes.
        #[test]
        fn fix_stats_durations_sum_to_total_interval(
            deltas_and_fixes in proptest::collection::vec(
                (1i64..300i64, proptest::bool::ANY),
                2..20usize,
            )
        ) {
            let mut t: i64 = 0;
            let points: Vec<NavPoint> = deltas_and_fixes
                .iter()
                .map(|(dt, has_fix)| {
                    t += dt;
                    make_point_with_fix(t, *has_fix)
                })
                .collect();

            // All points have satellite data, so fix stats must be Some.
            let stats = compute_fix_stats(&points).expect("all points have satellite data");

            // Compute expected total: sum of intervals between consecutive sat-report points.
            let expected_total = points
                .windows(2)
                .map(|pair| {
                    if let [a, b] = pair {
                        b.tpv.time() - a.tpv.time()
                    } else {
                        Duration::zero()
                    }
                })
                .fold(Duration::zero(), |acc, d| acc + d);

            proptest::prop_assert_eq!(
                stats.time_with_fix + stats.time_without_fix,
                expected_total,
            );
        }
    }
}
