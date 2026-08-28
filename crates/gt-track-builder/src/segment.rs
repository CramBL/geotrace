use chrono::{DateTime, Duration, Utc};
use gt_analysis::clock_offset::{self, ClockOffsetExcursion};
use gt_analysis::robust::median_i64;
use gt_geo_math::{GreatCircleArc, path_distance_km, point_set_diameter_m, segment_length_range_m};
use gt_types::channel::Channel;
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::geo_bounds::{GeoBounds, PoleWinding};
use gt_types::markers::{
    CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker, GeneratedMarkerKind,
};
use gt_types::nav_point::NavPoint;
use gt_types::satellites::SlipEvent;
use gt_types::time_types::GpsTime;
use gt_types::track::{
    FileMetadata, FileSource, FixStats, LoadWarning, LoadedFile, LoadedTrack, MercBounds,
    SegmentLengthRange, TimeRange, TrackMetadata, TravelMode,
};
use std::ops::Range;
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

/// Configuration that affects the track ranges produced by segmentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackLayoutConfig {
    /// Timestamp gap between consecutive points that triggers a new track split.
    pub track_split_gap: Duration,
}

impl Default for TrackLayoutConfig {
    fn default() -> Self {
        Self {
            track_split_gap: Duration::seconds(300),
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
/// plot's default so a fresh config agrees with the plot out of the box.
pub const DEFAULT_SLIP_ELEVATION_MASK_DEG: f32 = 15.0;

/// Default SNR drop (dB-Hz) that counts as a slip.
pub const DEFAULT_SLIP_SNR_DROP_DB: f32 = 10.0;

/// Default deviation from a track's baseline clock offset, in seconds, above
/// which a sample counts as a clock offset excursion.  Re-exported from the
/// detector so the marker default and the plot default are one value.
pub const DEFAULT_CLOCK_EXCURSION_THRESHOLD_S: f32 = clock_offset::DEFAULT_EXCURSION_THRESHOLD_S;

/// Partitions `points` into contiguous track ranges. A new track begins when the
/// timestamp gap between consecutive points reaches `config.track_split_gap`.
/// Returns an empty vec for empty input.
pub fn segment_tracks(points: &[NavPoint], config: &TrackLayoutConfig) -> Vec<Range<usize>> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;

    for (i, pair) in points.windows(2).enumerate() {
        if let [a, b] = pair {
            let gap = b.tpv.time() - a.tpv.time();
            if gap >= config.track_split_gap {
                ranges.push(start..i + 1);
                start = i + 1;
            }
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
    fn update(&mut self, point: &NavPoint, fix_count: u32) -> Option<GeneratedMarker> {
        let result;
        self.state = match self.state {
            GpsFixState::Waiting => {
                result = None;
                if fix_count > 0 {
                    GpsFixState::HasFix {
                        last_time: point.tpv.time(),
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
                        last_time: point.tpv.time(),
                        last_position: point.resolved_position(),
                    }
                }
            }
            GpsFixState::LostFix { lost_at } => {
                if fix_count > 0 {
                    let duration = point.tpv.time().signed_duration_since(lost_at);
                    let (lat, lon) = point.resolved_position();
                    result = Some(GeneratedMarker::new(
                        point.tpv.time().utc(),
                        GeneratedMarkerKind::GnssFixRegained {
                            fix_lost_duration: duration,
                        },
                        lat,
                        lon,
                    ));
                    GpsFixState::HasFix {
                        last_time: point.tpv.time(),
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

fn detect_generated_markers(
    points: &[NavPoint],
    config: &GeneratedMarkerConfig,
) -> Vec<GeneratedMarker> {
    let mut tracker = GpsFixTracker::new();
    let mut markers = Vec::new();
    for point in points {
        // The state machine always advances so the regained-duration stays
        // correct, but a marker is only kept when its kind is enabled.
        if let Some(sats) = &point.satellites
            && let Some(marker) = tracker.update(point, sats.fix_count())
            && fix_marker_enabled(&marker.kind, config)
        {
            markers.push(marker);
        }
    }
    // Excursions are classified whatever the marker toggle says: the
    // discontinuity pass needs them out of its step series either way, or one
    // excursion reads as a pair of jumps - out and straight back.
    let excursions = clock_offset::detect_excursions(points, config.clock_excursion_threshold_s);
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
    points: &[NavPoint],
    config: &GeneratedMarkerConfig,
) -> Vec<GeneratedMarker> {
    gt_analysis::loss_of_lock::detect_slip_events(
        points,
        config.slip_elevation_mask_deg,
        config.slip_snr_drop_db,
    )
    .into_iter()
    .filter_map(|(i, slips)| {
        let point = points.get(i)?;
        let (lat, lon) = point.resolved_position();
        Some(GeneratedMarker::new(
            point.tpv.time().utc(),
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
/// (ascending) names the samples already explained by a
/// [`GeneratedMarkerKind::ClockOffsetExcursion`], which are left out of the
/// step series.
fn detect_clock_discontinuities(
    points: &[NavPoint],
    sigmas: f64,
    excursion_indices: &[usize],
) -> Vec<GeneratedMarker> {
    // Pass 1: offset (ms) and source index for each with-system-timestamp
    // sample, then the step (first difference) between consecutive samples.
    let samples: Vec<(usize, i64)> = points
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
                point.tpv.time().utc(),
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
    points: &[NavPoint],
    excursions: &[ClockOffsetExcursion],
) -> Vec<GeneratedMarker> {
    excursions
        .iter()
        .filter_map(|excursion| {
            let peak = excursion.peak();
            let point = points.get(peak.index)?;
            let (lat, lon) = point.resolved_position();
            Some(GeneratedMarker::new(
                point.tpv.time().utc(),
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
/// The geometry - distance, diameter, segment lengths and bounding box - is
/// measured over [`NavPoint::resolved_position`], so it describes the path the
/// map draws. [`build_loaded_file`] resolves a track's ghost fixes before it
/// calls this.
pub fn compute_track_metadata(
    index: usize,
    points: &vec1::Vec1<NavPoint>,
    custom_markers: &[CustomMarker],
    generated_markers: &[GeneratedMarker],
) -> TrackMetadata {
    let first = points.first();

    let bounding_box = GeoBounds::from_first_position_and_rest(
        first.resolved_position(),
        points.iter().skip(1).map(NavPoint::resolved_position),
    )
    .extended_to_the_encircled_pole(PoleWinding::of_track(
        points.iter().map(NavPoint::resolved_position),
    ));
    let merc_bounds = MercBounds::from(bounding_box);

    let distance_km = Length::new::<kilometer>(path_distance_km(points));
    let diameter_m = Length::new::<meter>(point_set_diameter_m(points));
    let segment_length_range =
        segment_length_range_m(points).map(|(min_m, max_m)| SegmentLengthRange {
            min: Length::new::<meter>(min_m),
            max: Length::new::<meter>(max_m),
        });

    let time_range = time_range_spanning_every_fix(points);
    let duration = time_range.duration();

    TrackMetadata {
        index,
        distance_km,
        duration,
        time_range,
        bounding_box,
        merc_bounds,
        point_set_diameter_m: diameter_m,
        segment_length_range,
        has_custom_markers: !custom_markers.is_empty(),
        tpv_count: points.len(),
        satellite_report_count: points.iter().filter(|p| p.satellites.is_some()).count(),
        custom_marker_count: custom_markers.len(),
        generated_marker_count: generated_markers.len(),
        event_marker_count: 0, // filled in by build_loaded_file after event marker assignment
        fix_stats: compute_fix_stats(points),
    }
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

/// Segments `points` into tracks and builds a fully-populated `LoadedFile`.
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
    let ranges = segment_tracks(points, &config.track_layout);

    let mut loaded_tracks: Vec<LoadedTrack> = ranges
        .into_iter()
        .enumerate()
        .map(|(track_idx, range)| {
            let track_points_slice = points
                .get(range)
                .expect("ranges from segment_tracks are in bounds");

            let mut track_points: vec1::Vec1<NavPoint> =
                vec1::Vec1::try_from_vec(track_points_slice.to_vec())
                    .expect("segment_tracks produces only non-empty ranges");

            // The ghost fixes are resolved first: the metadata geometry, the
            // generated markers, the LOD levels and the satellite-label
            // anchors below all read the resolved positions.
            precompute_ghost_positions(&mut track_points);

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

            let track_generated =
                detect_generated_markers(&track_points, &config.generated_markers);

            let metadata = compute_track_metadata(
                track_idx + 1,
                &track_points,
                &track_custom,
                &track_generated,
            );

            let track_points_vec = track_points.into_vec();
            let lod = crate::lod::build_track_lod(&track_points_vec);
            let sat_label_anchors = crate::sat_label::build_sat_label_anchors(&track_points_vec);

            LoadedTrack {
                metadata,
                points: track_points_vec,
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

    let total_distance_km = loaded_tracks
        .iter()
        .map(|t| t.metadata.distance_km)
        .sum::<Length>();
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
            total_distance_km,
            total_duration,
            time_range: file_time_range,
            fix_stats: file_fix_stats,
            title: file_meta.title,
            device: file_meta.device,
            notes: file_meta.notes,
            travel_mode: file_meta.travel_mode,
        },
        tracks: loaded_tracks,
        event_marker_styles: event_marker_styles
            .into_iter()
            .map(|s| (s.variant_path.clone(), s))
            .collect(),
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

/// Sets the resolved position of ghost points (those with `heading == None`)
/// to a position interpolated in time along the great circle between the
/// surrounding real fixes (`fix_count > 0`).
///
/// The interpolated position is where the point is taken to be and what the
/// renderers draw: the coordinates a receiver reports without a heading may be
/// unreliable. [`NavPoint::tpv`] keeps the recorded coordinates.
///
/// Runs in O(n) over all points in the track.
#[expect(
    clippy::indexing_slicing,
    reason = "all indices are constructed from 0..n and arrays have length n, so always in bounds"
)]
fn precompute_ghost_positions(points: &mut [NavPoint]) {
    let n = points.len();
    if n == 0 {
        return;
    }

    // Forward pass: for each index, the nearest preceding index with fix_count > 0.
    let mut prev_real: Vec<Option<usize>> = vec![None; n];
    let mut last_real: Option<usize> = None;
    for i in 0..n {
        prev_real[i] = last_real;
        if points[i].fix_count() > 0 {
            last_real = Some(i);
        }
    }

    // Backward pass: for each index, the nearest following index with fix_count > 0.
    let mut next_real: Vec<Option<usize>> = vec![None; n];
    let mut next_real_fix: Option<usize> = None;
    for i in (0..n).rev() {
        next_real[i] = next_real_fix;
        if points[i].fix_count() > 0 {
            next_real_fix = Some(i);
        }
    }

    // Collect updates to avoid simultaneous mutable and immutable borrows.
    let mut updates: Vec<(usize, (Latitude, Longitude))> = Vec::new();
    for i in 0..n {
        if points[i].tpv.heading().is_some() {
            continue;
        }
        let position = match (prev_real[i], next_real[i]) {
            (Some(pi), Some(ni)) => {
                let anchor_span_secs =
                    (points[ni].tpv.time() - points[pi].tpv.time()).as_seconds_f64();
                let elapsed_secs = (points[i].tpv.time() - points[pi].tpv.time()).as_seconds_f64();
                if anchor_span_secs > 0.0 {
                    let arc = GreatCircleArc {
                        start: points[pi].resolved_position(),
                        end: points[ni].resolved_position(),
                    };
                    arc.position_at_ratio(elapsed_secs / anchor_span_secs)
                } else {
                    points[i].resolved_position()
                }
            }
            (Some(pi), None) => points[pi].resolved_position(),
            (None, Some(ni)) => points[ni].resolved_position(),
            (None, None) => points[i].resolved_position(),
        };
        updates.push((i, position));
    }

    for (i, position) in updates {
        points[i].set_resolved_position(position);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::TimeZone;
    use geotrace_sdk_units::Unit;
    use gt_types::coordinates::{Latitude, Longitude, RecordedLatitude, RecordedLongitude};
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::{GpsTime, SysTime};
    use gt_types::tpv::TimePositionVelocity;
    use rstest::rstest;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    use super::*;

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
        NavPoint::new(tpv, None).expect("coordinates in range")
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
        let markers = detect_clock_discontinuities(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS, &[]);
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
            detect_generated_markers(&resume_from_gap_points(), &GeneratedMarkerConfig::default());
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
        let markers = detect_generated_markers(&resume_from_gap_points(), &config);
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
        let markers = detect_generated_markers(&points, &GeneratedMarkerConfig::default());
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
        assert!(
            detect_clock_discontinuities(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS, &[]).is_empty()
        );
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
        assert!(
            detect_clock_discontinuities(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS, &[]).is_empty()
        );
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
                detect_clock_discontinuities(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS, &[]).is_empty(),
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
        NavPoint::new(tpv, None).expect("coordinates in range")
    }

    fn make_point_at_pos(t: i64, lat: f64, lon: f64) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(lat))
            .lon(gt_types::coordinates::Longitude::new(lon))
            .heading(Angle::new::<degree>(0.0))
            .build();
        NavPoint::new(tpv, None).expect("coordinates in range")
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

    #[test]
    fn segment_tracks_gap_exactly_300s_starts_new_trip() {
        // [0s, 300s] → gap of exactly 300 s triggers a new track
        let pts = vec![make_point_at(0), make_point_at(300), make_point_at(360)];
        let ranges = segment_tracks(&pts, &TrackLayoutConfig::default());
        assert_eq!(ranges, vec![0..1, 1..3]);
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
        assert!(
            meta.distance_km > Length::new::<kilometer>(5.0),
            "expected > 5 km, got {:?}",
            meta.distance_km
        );
        assert!(!meta.has_custom_markers);
        assert_eq!(meta.satellite_report_count, 0);
    }

    #[test]
    fn compute_track_metadata_single_point_has_zero_duration() {
        let pts = vec1::vec1![make_point_at_pos(0, 55.0, 12.0)];
        let meta = compute_track_metadata(1, &pts, &[], &[]);
        assert_eq!(meta.duration.num_seconds(), 0);
        assert_eq!(meta.distance_km, Length::new::<kilometer>(0.0));
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
        NavPoint::new(tpv, Some(sats)).expect("coordinates in range")
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
        NavPoint::new(tpv, Some(sats)).expect("coordinates in range")
    }

    fn make_ghost(t: i64, lat: Latitude, lon: Longitude) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(lat)
            .lon(lon)
            .build();
        NavPoint::new(tpv, None).expect("coordinates in range")
    }

    #[test]
    fn precompute_ghost_positions_empty_slice() {
        let mut points: Vec<NavPoint> = vec![];
        precompute_ghost_positions(&mut points);
    }

    #[test]
    fn precompute_ghost_positions_all_real_unchanged() {
        let mut points = vec![
            make_real_fix(0, Latitude::new(55.0), Longitude::new(12.0)),
            make_real_fix(1, Latitude::new(55.1), Longitude::new(12.1)),
        ];
        let before: Vec<_> = points.iter().map(NavPoint::resolved_position).collect();
        precompute_ghost_positions(&mut points);
        let after: Vec<_> = points.iter().map(NavPoint::resolved_position).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn precompute_ghost_positions_ghost_between_two_anchors_interpolates() {
        // Real fixes on the equator at t=0 (lon=0) and t=10 (lon=1), ghost at
        // t=5. The equator is a great circle, so the ghost lands at lon=0.5.
        let mut points = vec![
            make_real_fix(0, Latitude::new(0.0), Longitude::new(0.0)),
            make_ghost(5, Latitude::new(10.0), Longitude::new(10.0)),
            make_real_fix(10, Latitude::new(0.0), Longitude::new(1.0)),
        ];
        precompute_ghost_positions(&mut points);

        let (latitude, longitude) = points[1].resolved_position();
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
    fn precompute_ghost_positions_ghost_before_first_anchor_snaps_to_it() {
        let mut points = vec![
            make_ghost(0, Latitude::new(10.0), Longitude::new(10.0)),
            make_real_fix(10, Latitude::new(55.0), Longitude::new(12.0)),
        ];
        precompute_ghost_positions(&mut points);

        assert_eq!(
            points[0].resolved_position(),
            (Latitude::new(55.0), Longitude::new(12.0))
        );
    }

    #[test]
    fn precompute_ghost_positions_ghost_after_last_anchor_snaps_to_it() {
        let mut points = vec![
            make_real_fix(0, Latitude::new(55.0), Longitude::new(12.0)),
            make_ghost(10, Latitude::new(10.0), Longitude::new(10.0)),
        ];
        precompute_ghost_positions(&mut points);

        assert_eq!(
            points[1].resolved_position(),
            (Latitude::new(55.0), Longitude::new(12.0))
        );
    }

    /// Every fix in the placement tests below sits on the equator, so the
    /// longitudes alone say where the builder placed them. 1e-9° is about
    /// 0.1 mm.
    const PLACEMENT_TOLERANCE_DEGREES: f64 = 1e-9;

    /// A fix with a position and a heading, but no satellite report: the
    /// receiver said where it was without saying what it tracked.
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

    /// A fix the receiver wrote a latitude of NaN for. It has a heading, so
    /// only its unusable coordinate makes the builder place it, and a
    /// longitude far from the fixes around it, so a position kept as recorded
    /// is distinguishable from a placed one.
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
        #[case] mut points: Vec<NavPoint>,
        #[case] expected_longitudes: Vec<f64>,
    ) {
        place_fixes_without_a_measured_position(&mut points);

        let drawn_longitudes: Vec<f64> = points
            .iter()
            .map(|point| point.resolved_position().1.as_degrees())
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

    #[test]
    fn track_metadata_counts_the_fixes_whose_recorded_position_is_out_of_range() {
        let mut points = vec![
            measured_fix_without_a_satellite_report(0, 0.0),
            fix_without_a_recorded_position(5),
            measured_fix_without_a_satellite_report(10, 10.0),
        ];
        place_fixes_without_a_measured_position(&mut points);
        let points = vec1::Vec1::try_from_vec(points).expect("three fixes");

        let metadata = compute_track_metadata(1, &points, &[], &[]);

        assert_eq!(metadata.invalid_position_count, 1);
    }

    #[test]
    fn precompute_ghost_positions_all_ghosts_no_anchors_unchanged() {
        let mut points = vec![
            make_ghost(0, Latitude::new(55.0), Longitude::new(12.0)),
            make_ghost(5, Latitude::new(56.0), Longitude::new(13.0)),
        ];
        let before: Vec<_> = points.iter().map(NavPoint::resolved_position).collect();
        precompute_ghost_positions(&mut points);
        let after: Vec<_> = points.iter().map(NavPoint::resolved_position).collect();
        assert_eq!(before, after);
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
