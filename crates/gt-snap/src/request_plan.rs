//! Turn a track's points into the sequence of requests a snap run sends.
//!
//! Three concerns, all pure functions over the input points:
//!
//! - **Downsampling**: drop ghost fixes, then thin what remains to at most
//!   one point per [`MIN_POINT_INTERVAL`]. Snap error is only defined for
//!   sent points.
//! - **Chunking**: split into [`CHUNK_POINTS`]-sized requests sharing
//!   [`CHUNK_OVERLAP_POINTS`] of context.
//! - **`gps_accuracy` derivation**: median eph of the sent points, clamped
//!   to [`GPS_ACCURACY_RANGE_M`] - the one parameter GeoTrace derives more
//!   accurately than the server default.

use std::ops::{Range, RangeInclusive};
use std::time::Duration;

use gt_types::{NavPoint, PointIdx};

use crate::wire::{Costing, ShapePoint, TraceAttributesRequest, TraceOptions};

/// Minimum time between two sent points. Input at a higher rate is thinned.
pub const MIN_POINT_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum sent points per request chunk. Far under the server's observed
/// 16 000-point cap. Sized so one failed chunk loses little work and
/// progress updates stay frequent.
pub const CHUNK_POINTS: usize = 1000;

/// Points of context shared between consecutive chunks (~40 s at 1 Hz):
/// enough HMM warm-up that match quality does not degrade at chunk cuts.
pub const CHUNK_OVERLAP_POINTS: usize = 40;

const _: () = assert!(
    CHUNK_OVERLAP_POINTS.is_multiple_of(2),
    "CHUNK_OVERLAP_POINTS must be even so both chunks split the overlap \
     without double-owning or dropping a point"
);

/// Bounds for the derived `gps_accuracy`, meters. The lower bound keeps a
/// receiver's optimistic eph from starving the candidate search. The upper
/// bound keeps outlier eph from letting the match wander off the road.
pub const GPS_ACCURACY_RANGE_M: RangeInclusive<f64> = 5.0..=30.0;

/// Server-accepted range for `trace_options.search_radius`, meters.
/// Pinned empirically against the FOSSGIS server (2026-07): 100 is
/// accepted, 101 and negative values are rejected with error 158.
pub const SEARCH_RADIUS_RANGE_M: RangeInclusive<f64> = 0.0..=100.0;

/// Server-accepted range for a user-supplied `gps_accuracy`, meters.
/// Pinned empirically like [`SEARCH_RADIUS_RANGE_M`]. Deliberately wider
/// than [`GPS_ACCURACY_RANGE_M`], which bounds the value *derived* from
/// eph - an explicit override may use the server's full range.
pub const GPS_ACCURACY_OVERRIDE_RANGE_M: RangeInclusive<f64> = 0.0..=100.0;

/// Client-side range for `trace_options.turn_penalty_factor`.
/// Empirically the server enforces only the lower bound (negative values
/// are rejected with error 158, and 10^9 was accepted), so the upper bound is
/// this client's own sanity cap - Valhalla's guidance suggests around 500
/// to smooth wandering matches, and far larger values stop changing the
/// match.
pub const TURN_PENALTY_FACTOR_RANGE: RangeInclusive<f64> = 0.0..=100_000.0;

/// The user-facing parameters of a snap run: the costing plus the optional
/// advanced trace options. An unset option means server default - except
/// [`gps_accuracy_override_m`](Self::gps_accuracy_override_m), where unset
/// means derived from the track's eph.
///
/// Values are clamped to the server-accepted ranges when the request is
/// built ([`SnapParams::trace_options`]): the server rejects out-of-range
/// trace options with a 400, error 158.
///
/// Serde derives exist for persisting a run's parameters with its cached
/// result. The options default so parameters added later decode absent from
/// older stored results.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SnapParams {
    pub costing: Costing,
    /// Meters around each input point searched for candidate road edges,
    /// bounded by [`SEARCH_RADIUS_RANGE_M`].
    #[serde(default)]
    pub search_radius_m: Option<f64>,
    /// Cost multiplier penalizing route reversals, bounded by
    /// [`TURN_PENALTY_FACTOR_RANGE`].
    #[serde(default)]
    pub turn_penalty_factor: Option<f64>,
    /// Expected GNSS accuracy in meters, replacing the eph-derived value.
    /// Bounded by [`GPS_ACCURACY_OVERRIDE_RANGE_M`].
    #[serde(default)]
    pub gps_accuracy_override_m: Option<f64>,
}

impl SnapParams {
    /// Parameters with every advanced option at its default.
    pub fn new(costing: Costing) -> Self {
        Self {
            costing,
            search_radius_m: None,
            turn_penalty_factor: None,
            gps_accuracy_override_m: None,
        }
    }

    /// The `gps_accuracy` a run with these parameters sends, given the
    /// plan's eph-derived value: the clamped override when set, else the
    /// derived value, else the server default (`None`).
    pub fn gps_accuracy_sent_m(&self, derived_m: Option<f64>) -> Option<f64> {
        self.gps_accuracy_override_m
            .map(|v| clamp_to(&GPS_ACCURACY_OVERRIDE_RANGE_M, v))
            .or(derived_m)
    }

    /// The `trace_options` a run with these parameters sends, all values
    /// clamped to their server-accepted ranges. `None` when every option is
    /// at its server default.
    pub fn trace_options(&self, derived_gps_accuracy_m: Option<f64>) -> Option<TraceOptions> {
        let options = TraceOptions {
            gps_accuracy: self.gps_accuracy_sent_m(derived_gps_accuracy_m),
            search_radius: self
                .search_radius_m
                .map(|v| clamp_to(&SEARCH_RADIUS_RANGE_M, v)),
            turn_penalty_factor: self
                .turn_penalty_factor
                .map(|v| clamp_to(&TURN_PENALTY_FACTOR_RANGE, v)),
        };
        (options != TraceOptions::default()).then_some(options)
    }
}

fn clamp_to(range: &RangeInclusive<f64>, value: f64) -> f64 {
    value.clamp(*range.start(), *range.end())
}

/// One point selected for sending, tied back to its origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SentPoint {
    /// Index of the source point within the track.
    pub point: PointIdx,
    pub shape_point: ShapePoint,
    /// Receiver-reported horizontal accuracy of the source point, carried
    /// along so accuracy derivation needs no second walk over the track.
    pub eph_m: Option<f32>,
}

/// How a chunk follows the one before it in the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkContinuity {
    /// Same stretch as the previous chunk, sharing [`CHUNK_OVERLAP_POINTS`]
    /// of context with it: the two match one continuous drive.
    ContinuesPrevious,
    /// First chunk of a stretch. What precedes its first point is the start
    /// of the track or a run of ghost fixes the plan dropped, so nothing
    /// connects this chunk to the previous one.
    OpensStretch,
}

/// One request's worth of points, with provenance and ownership.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// The points this chunk sends, in track order. Includes overlap.
    pub sent: Vec<SentPoint>,
    /// The sub-range of [`sent`](Self::sent) this chunk is authoritative
    /// for. Overlap points outside this range are context only. Stitching
    /// takes their match results from the neighboring chunk, where they are
    /// more interior.
    pub owned: Range<usize>,
    pub continuity: ChunkContinuity,
}

impl Chunk {
    /// See [`owned`](Self::owned).
    pub fn owned_sent(&self) -> &[SentPoint] {
        self.sent.get(self.owned.clone()).unwrap_or_default()
    }

    /// The wire request sending this chunk: all sent points (overlap
    /// included), the production attribute filter, and the parameters'
    /// trace options resolved against the plan's derived accuracy.
    pub fn request(
        &self,
        params: &SnapParams,
        derived_gps_accuracy_m: Option<f64>,
    ) -> TraceAttributesRequest {
        let mut request = TraceAttributesRequest::new(
            params.costing,
            self.sent.iter().map(|s| s.shape_point).collect(),
        );
        request.trace_options = params.trace_options(derived_gps_accuracy_m);
        request
    }
}

/// The full request plan for one track: the chunk sequence plus the derived
/// accuracy shared by every chunk's `trace_options`.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestPlan {
    pub chunks: Vec<Chunk>,
    /// Median eph of the sent points clamped to [`GPS_ACCURACY_RANGE_M`],
    /// or `None` (server default) when no sent point carries eph.
    pub gps_accuracy_m: Option<f64>,
}

impl RequestPlan {
    /// Total number of distinct points that will be sent, counting overlap
    /// points once. Ownership ranges partition the sent sequence exactly,
    /// so this is the sum of owned lengths.
    pub fn sent_point_count(&self) -> usize {
        self.chunks.iter().map(|c| c.owned.len()).sum()
    }
}

/// A maximal run of sent points the matcher may treat as one continuous
/// drive: no ghost fix was dropped between any two of them.
struct SendableStretch {
    sent: Vec<SentPoint>,
}

/// Build the request plan for a track's points.
pub fn plan(points: &[NavPoint]) -> RequestPlan {
    let stretches = downsample(points);
    let gps_accuracy_m = derive_gps_accuracy(stretches.iter().flat_map(|s| s.sent.iter()));
    let chunks = stretches
        .iter()
        .flat_map(|stretch| chunk_stretch(&stretch.sent))
        .collect();
    RequestPlan {
        chunks,
        gps_accuracy_m,
    }
}

/// Select the points to send, split into stretches at the ghost fixes.
///
/// Ghost fixes are never sent: they are dead-reckoning guesses, not measured
/// positions. Each one ends the stretch. Two points sent either side of a
/// dropped run arrive as neighbors and the matcher routes a road through the
/// gap between them.
fn downsample(points: &[NavPoint]) -> Vec<SendableStretch> {
    let mut stretches = Vec::new();
    let mut start = 0;
    for run in points.split(NavPoint::is_ghost_fix) {
        let sent = downsample_run(run, start);
        if !sent.is_empty() {
            stretches.push(SendableStretch { sent });
        }
        // Past this run and past the ghost fix that ended it.
        start += run.len() + 1;
    }
    stretches
}

/// Thin one run of real fixes: its first point, then every point at least
/// [`MIN_POINT_INTERVAL`] after the previously selected one. `base` is the
/// run's start index within the track, which the sent points carry back.
fn downsample_run(run: &[NavPoint], base: usize) -> Vec<SentPoint> {
    let mut sent = Vec::new();
    let mut last_kept = None;
    for (offset, point) in run.iter().enumerate() {
        let time = point.tpv.time().utc();
        let keep = match last_kept {
            None => true,
            // `to_std` fails for a negative elapsed time (out-of-order
            // timestamps). Such a point is never kept as a new sample.
            Some(last) => time
                .signed_duration_since(last)
                .to_std()
                .is_ok_and(|elapsed| elapsed >= MIN_POINT_INTERVAL),
        };
        if keep {
            last_kept = Some(time);
            sent.push(SentPoint {
                point: PointIdx::new(base + offset),
                shape_point: ShapePoint {
                    lat: point.tpv.lat().as_degrees(),
                    lon: point.tpv.lon().as_degrees(),
                    time: Some(time.timestamp()),
                },
                eph_m: point.tpv.eph_m(),
            });
        }
    }
    sent
}

/// Split one stretch's sent points into overlapping chunks with ownership
/// ranges. Chunks never span two stretches, so the first one opens a
/// stretch and the rest continue it.
fn chunk_stretch(sent: &[SentPoint]) -> Vec<Chunk> {
    if sent.is_empty() {
        return Vec::new();
    }
    let step = CHUNK_POINTS - CHUNK_OVERLAP_POINTS;
    let mut chunks = Vec::new();
    let mut start = 0;
    loop {
        let end = (start + CHUNK_POINTS).min(sent.len());
        let is_first = start == 0;
        let is_last = end == sent.len();
        // The overlap between two chunks is split at its middle. Each point
        // belongs to the chunk where it lies further from the chunk edge
        // (the design's "prefer interior" stitching rule).
        let own_from = if is_first {
            0
        } else {
            CHUNK_OVERLAP_POINTS / 2
        };
        let own_to = if is_last {
            end - start
        } else {
            (end - start) - CHUNK_OVERLAP_POINTS / 2
        };
        // A non-first chunk is always longer than the overlap (the loop
        // only continues while at least `step` points remain), so the
        // ownership range can never invert.
        debug_assert!(own_from <= own_to);
        chunks.push(Chunk {
            sent: sent.get(start..end).unwrap_or_default().to_vec(),
            owned: own_from..own_to,
            continuity: if is_first {
                ChunkContinuity::OpensStretch
            } else {
                ChunkContinuity::ContinuesPrevious
            },
        });
        if is_last {
            return chunks;
        }
        start += step;
    }
}

/// Median eph of the sent points, clamped to [`GPS_ACCURACY_RANGE_M`].
fn derive_gps_accuracy<'a>(sent: impl Iterator<Item = &'a SentPoint>) -> Option<f64> {
    let mut ephs: Vec<f64> = sent.filter_map(|s| s.eph_m).map(f64::from).collect();
    if ephs.is_empty() {
        return None;
    }
    ephs.sort_unstable_by(f64::total_cmp);
    let mid = ephs.len() / 2;
    // The lookups cannot fail: `ephs` is non-empty (guarded above), so
    // `mid < len`, and in the even branch `len >= 2` implies `mid >= 1`.
    // `?` keeps the arithmetic panic-free regardless.
    let median = if ephs.len().is_multiple_of(2) {
        (ephs.get(mid.checked_sub(1)?)? + ephs.get(mid)?) / 2.0
    } else {
        *ephs.get(mid)?
    };
    Some(median.clamp(*GPS_ACCURACY_RANGE_M.start(), *GPS_ACCURACY_RANGE_M.end()))
}
