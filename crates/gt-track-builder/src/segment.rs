use chrono::{DateTime, Duration, Utc};
use gt_analysis::clock_offset::{self, ClockOffsetExcursion};
use gt_analysis::robust;
use gt_geo_math::GreatCircleArc;
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
    SegmentLengthRange, TimeRange, TrackAggregates, TrackGeometry, TrackMetadata, TravelMode,
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

/// Which fixes the builder places between the fixes around them, and where it
/// places one stamped outside the time span those two fixes cover.
///
/// The history database holds the rule a recording's stored geometry was
/// placed by. Re-running the builder under that rule reproduces the positions
/// its fixes are drawn at, and the distances and bounding boxes measured over
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FixPlacementRule {
    /// A fix with no heading is dead-reckoned. One stamped outside its
    /// anchors' time span is placed along the great circle through them,
    /// continued past the anchor it is stamped beyond. A recording stored by
    /// an earlier version was placed by this rule.
    MissingHeading,
    /// A fix with no heading and no satellite in fix is dead-reckoned. One
    /// stamped outside its anchors' time span is placed at the anchor it is
    /// stamped nearer to.
    #[default]
    MissingHeadingAndNothingInFix,
}

impl FixPlacementRule {
    fn classifies_as_dead_reckoned(self, point: &NavPoint) -> bool {
        match self {
            Self::MissingHeading => point.tpv.heading().is_none(),
            Self::MissingHeadingAndNothingInFix => {
                point.tpv.heading().is_none() && point.fix_count() == 0
            }
        }
    }

    /// Where along its anchors' arc a fix is placed, given the share of their
    /// time span that has passed at its own timestamp. A backward time step
    /// under the split gap leaves a fix inside one track with a share below
    /// zero, and a forward one with a share above one.
    fn share_along_the_arc(self, elapsed_share: f64) -> f64 {
        match self {
            Self::MissingHeading => elapsed_share,
            Self::MissingHeadingAndNothingInFix => elapsed_share.clamp(0.0, 1.0),
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
    pub fix_placement_rule: FixPlacementRule,
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
/// Returns an empty `Vec` for empty input.
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
    // discontinuity pass needs them out of its step series either way, or it
    // counts one excursion as a pair of jumps - out and straight back.
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
/// not once per sample of a shifted plateau. A steady large offset has no jump
/// to flag.
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

    let Some(median) = robust::median_i64(&steps) else {
        return Vec::new();
    };
    let deviations: Vec<i64> = steps
        .iter()
        .map(|&s| s.saturating_sub(median).saturating_abs())
        .collect();
    let Some(mad) = robust::median_i64(&deviations) else {
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
        event_marker_count: 0, // filled in by `build_loaded_file` after event marker assignment
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

/// The geometry of `points` taken as a track on their own: every fix `rule`
/// names as dead-reckoned is placed from the fixes that have a recorded
/// position, and the geometry is measured over where they all landed.
///
/// [`TrackGeometry::NoValidPosition`] when no fix of `points` has a recorded
/// position, which leaves the whole track unplaced.
pub fn measure_track_geometry(points: &[NavPoint], rule: FixPlacementRule) -> TrackGeometry {
    let mut placements = recorded_placements(points);
    place_track_fixes(points, &mut placements, rule);
    track_geometry(placements)
}

/// Places the fixes of one track from its own fixes: first those the receiver
/// wrote no position for, then the ones it dead-reckoned.
fn place_track_fixes(points: &[NavPoint], placements: &mut FixPlacements, rule: FixPlacementRule) {
    place_fixes(
        points,
        placements,
        UnmeasuredFix::CoordinateOutOfRange,
        rule,
    );
    place_fixes(points, placements, UnmeasuredFix::Ghost, rule);
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
        distance_km: Length::new::<kilometer>(gt_geo_math::path_distance_km(&positions)),
        point_set_diameter_m: Length::new::<meter>(gt_geo_math::point_set_diameter_m(&positions)),
        segment_length_range: gt_geo_math::segment_length_range_m(&positions).map(
            |(min_m, max_m)| SegmentLengthRange {
                min: Length::new::<meter>(min_m),
                max: Length::new::<meter>(max_m),
            },
        ),
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
    place_fixes(
        points,
        &mut placements,
        UnmeasuredFix::CoordinateOutOfRange,
        config.fix_placement_rule,
    );

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
            place_track_fixes(
                &track_points,
                &mut track_placements,
                config.fix_placement_rule,
            );
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
    place_event_markers(&mut loaded_tracks);

    // Back-fill `event_marker_count` now that assignment is done.
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

    let TrackAggregates {
        total_distance,
        total_duration,
        time_range,
        fix_stats,
    } = TrackAggregates::over_tracks(&loaded_tracks);

    LoadedFile {
        metadata: FileMetadata {
            filename,
            total_distance,
            total_duration,
            time_range,
            fix_stats,
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

/// A position with the instant the recording holds it at.
#[derive(Clone, Copy)]
struct TimedPosition {
    time: DateTime<Utc>,
    position: (Latitude, Longitude),
}

/// The great circle from `start` to `end`, interpolated in proportion to the
/// time between the two.
#[derive(Clone, Copy)]
struct TimedArc {
    start: TimedPosition,
    end: TimedPosition,
}

impl TimedArc {
    /// The share of the arc's time span that has passed at `time`: negative
    /// before its start, above one after its end. `None` when both ends stamp
    /// one instant, spanning no time to place `time` in.
    fn elapsed_share(self, time: DateTime<Utc>) -> Option<f64> {
        let Self { start, end } = self;
        let span_secs = (end.time - start.time).as_seconds_f64();
        (span_secs > 0.0).then(|| (time - start.time).as_seconds_f64() / span_secs)
    }

    fn position_at_share(self, share: f64) -> (Latitude, Longitude) {
        let Self { start, end } = self;
        GreatCircleArc {
            start: start.position,
            end: end.position,
        }
        .position_at_ratio(share)
    }
}

/// A fix the receiver did not measure at the coordinates it holds, and what
/// anchors the position the builder places it at.
#[derive(Clone, Copy)]
enum UnmeasuredFix {
    /// A fix the receiver dead-reckoned, which [`FixPlacementRule`] names,
    /// anchored by the fixes with a satellite in fix.
    Ghost,
    /// A fix with a recorded latitude or longitude outside its range, anchored
    /// by every fix that has a recorded position: it holds none of its own to
    /// fall back on.
    CoordinateOutOfRange,
}

impl UnmeasuredFix {
    fn is_target(self, point: &NavPoint, rule: FixPlacementRule) -> bool {
        match self {
            Self::Ghost => {
                point.tpv.position().is_some() && rule.classifies_as_dead_reckoned(point)
            }
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
        rule: FixPlacementRule,
    ) -> Option<ResolvedPosition> {
        let anchor = |index: Option<usize>| {
            let index = index?;
            Some((points.get(index)?, (*placements.get(index)?)?))
        };
        let before = anchor(preceding);
        let after = anchor(following);

        let interpolated = match (before, after) {
            (Some((before, before_position)), Some((after, after_position))) => {
                let arc = TimedArc {
                    start: TimedPosition {
                        time: before.tpv.time().utc(),
                        position: before_position.coordinates(),
                    },
                    end: TimedPosition {
                        time: after.tpv.time().utc(),
                        position: after_position.coordinates(),
                    },
                };
                arc.elapsed_share(point.tpv.time().utc())
                    .map(|share| arc.position_at_share(rule.share_along_the_arc(share)))
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
fn place_fixes(
    points: &[NavPoint],
    placements: &mut FixPlacements,
    kind: UnmeasuredFix,
    rule: FixPlacementRule,
) {
    let anchors = nearest_anchors(points, |point| kind.is_anchor(point));

    let placed: Vec<(usize, ResolvedPosition)> = points
        .iter()
        .enumerate()
        .zip(&anchors)
        .filter(|((_, point), _)| kind.is_target(point, rule))
        .filter_map(|((index, point), anchors)| {
            Some((
                index,
                kind.placement(points, placements, point, *anchors, rule)?,
            ))
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

/// Writes onto each event marker of `tracks` where the map draws it, which is
/// where the builder drew the fixes its timestamp falls between.
///
/// Running this again over the same markers repeats the same placement: it
/// reads the recorded coordinates of every marker and every fix.
fn place_event_markers(tracks: &mut [LoadedTrack]) {
    for track in tracks {
        if track.event_markers.is_empty() {
            continue;
        }
        let fixes = track
            .placed_points()
            .map(placed_fixes_in_time_order)
            .unwrap_or_default();
        for marker in &mut track.event_markers {
            marker.resolved_position = position_between_placed_fixes(&fixes, marker.time)
                .unwrap_or_else(|| ResolvedPosition::measured(marker.lat, marker.lon));
        }
    }
}

/// Every fix of `placed` with where the builder drew it, in ascending time
/// order. A track holds its fixes in recording order, which a backward time
/// step leaves out of time order.
fn placed_fixes_in_time_order(placed: PlacedPoints<'_>) -> Vec<(DateTime<Utc>, ResolvedPosition)> {
    let mut fixes: Vec<(DateTime<Utc>, ResolvedPosition)> = placed
        .iter()
        .map(|point| (point.fix.tpv.time().utc(), point.resolved()))
        .collect();
    fixes.sort_by_key(|&(time, _)| time);
    fixes
}

/// Where `time` falls along the drawn track: on the great circle between the
/// two fixes of `fixes` it lies between.
///
/// `None` leaves an event marker at the coordinates the recording holds for
/// it, which is where a marker between two measured fixes already sits: the
/// recorder interpolated it over those same two positions. A track with no
/// drawn fix returns `None` too.
fn position_between_placed_fixes(
    fixes: &[(DateTime<Utc>, ResolvedPosition)],
    time: DateTime<Utc>,
) -> Option<ResolvedPosition> {
    let index = fixes.partition_point(|&(fix_time, _)| fix_time <= time);
    let before = index.checked_sub(1).and_then(|i| fixes.get(i));
    let after = fixes.get(index);

    match (before, after) {
        (Some(&(before_time, before_position)), Some(&(after_time, after_position))) => {
            if matches!(before_position, ResolvedPosition::Measured(_))
                && matches!(after_position, ResolvedPosition::Measured(_))
            {
                return None;
            }
            let arc = TimedArc {
                start: TimedPosition {
                    time: before_time,
                    position: before_position.coordinates(),
                },
                end: TimedPosition {
                    time: after_time,
                    position: after_position.coordinates(),
                },
            };
            let (latitude, longitude) = arc.position_at_share(arc.elapsed_share(time)?);
            Some(ResolvedPosition::interpolated(latitude, longitude))
        }
        (Some(&(_, position)), None) | (None, Some(&(_, position))) => match position {
            ResolvedPosition::Measured(_) => None,
            ResolvedPosition::Interpolated(_) => Some(position),
        },
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests;
