//! Turn a track's points into the sequence of requests a snap run sends.
//!
//! Three concerns, all pure functions over the input points:
//!
//! - **Downsampling**: thin to at most one point per [`MIN_POINT_INTERVAL`].
//!   Snap error is only defined for sent points.
//! - **Chunking**: split into [`CHUNK_POINTS`]-sized requests sharing
//!   [`CHUNK_OVERLAP_POINTS`] of context; the constants carry the rationale.
//! - **`gps_accuracy` derivation**: median eph of the sent points, clamped
//!   to [`GPS_ACCURACY_RANGE_M`] - the one parameter GeoTrace knows better
//!   than the server default.

use std::ops::{Range, RangeInclusive};
use std::time::Duration;

use gt_types::{NavPoint, PointIdx};

use crate::wire::{Costing, ShapePoint, TraceAttributesRequest, TraceOptions};

/// Minimum time between two sent points; input at a higher rate is thinned.
pub const MIN_POINT_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum sent points per request chunk. Far under the server's observed
/// 16 000-point cap; sized so one failed chunk loses little work and
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
/// receiver's optimistic eph from starving the candidate search; the upper
/// bound keeps outlier eph from letting the match wander off the road.
pub const GPS_ACCURACY_RANGE_M: RangeInclusive<f64> = 5.0..=30.0;

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

/// One request's worth of points, with provenance and ownership.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// The points this chunk sends, in track order. Includes overlap.
    pub sent: Vec<SentPoint>,
    /// The sub-range of [`sent`](Self::sent) this chunk is authoritative
    /// for. Overlap points outside this range are context only; stitching
    /// takes their match results from the neighboring chunk instead, where
    /// they are more interior.
    pub owned: Range<usize>,
}

impl Chunk {
    /// The subslice of [`sent`](Self::sent) this chunk is authoritative for
    /// (see [`owned`](Self::owned)).
    pub fn owned_sent(&self) -> &[SentPoint] {
        self.sent.get(self.owned.clone()).unwrap_or_default()
    }

    /// The wire request sending this chunk: all sent points (overlap
    /// included), the production attribute filter, and the plan's derived
    /// accuracy when present.
    pub fn request(&self, costing: Costing, gps_accuracy_m: Option<f64>) -> TraceAttributesRequest {
        let mut request =
            TraceAttributesRequest::new(costing, self.sent.iter().map(|s| s.shape_point).collect());
        request.trace_options = gps_accuracy_m.map(|gps_accuracy| TraceOptions {
            gps_accuracy: Some(gps_accuracy),
        });
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

/// Build the request plan for a track's points.
pub fn plan(points: &[NavPoint]) -> RequestPlan {
    let sent = downsample(points);
    let gps_accuracy_m = derive_gps_accuracy(&sent);
    RequestPlan {
        chunks: chunk(&sent),
        gps_accuracy_m,
    }
}

/// Select the points to send: the first point, then every point at least
/// [`MIN_POINT_INTERVAL`] after the previously selected one.
fn downsample(points: &[NavPoint]) -> Vec<SentPoint> {
    let mut sent = Vec::new();
    let mut last_kept = None;
    for (index, point) in points.iter().enumerate() {
        let time = point.tpv.time().utc();
        let keep = match last_kept {
            None => true,
            // `to_std` fails for a negative elapsed time (out-of-order
            // timestamps); such a point is never kept as a new sample.
            Some(last) => time
                .signed_duration_since(last)
                .to_std()
                .is_ok_and(|elapsed| elapsed >= MIN_POINT_INTERVAL),
        };
        if keep {
            last_kept = Some(time);
            sent.push(SentPoint {
                point: PointIdx::new(index),
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

/// Split the sent points into overlapping chunks with ownership ranges.
fn chunk(sent: &[SentPoint]) -> Vec<Chunk> {
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
        // Ownership: the overlap between two chunks is split at its middle;
        // each point belongs to the chunk where it lies further from the
        // chunk edge (the design's "prefer interior" stitching rule).
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
        });
        if is_last {
            return chunks;
        }
        start += step;
    }
}

/// Median eph of the sent points, clamped to [`GPS_ACCURACY_RANGE_M`].
fn derive_gps_accuracy(sent: &[SentPoint]) -> Option<f64> {
    let mut ephs: Vec<f64> = sent.iter().filter_map(|s| s.eph_m).map(f64::from).collect();
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
