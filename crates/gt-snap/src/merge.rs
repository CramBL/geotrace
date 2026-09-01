//! Merge per-chunk match results into one [`SnapResult`] for a track.
//!
//! Each chunk of the request plan produced an outcome: a successful match,
//! an off-network rejection (the server's error 444 - not a failure, see
//! [`ChunkOutcome::OffNetwork`]), or a failure that survived the transport's
//! retry. Merging takes each chunk's *owned* point range (overlap points
//! are context, owned by the neighboring chunk where they are more
//! interior), maps results back to track [`PointIdx`]s, and assembles the
//! per-point series, the snapped-track geometry, and the run metadata.
//!
//! Partial failures never abort the run: a failed chunk leaves its points
//! without data, is reported through the [`SnapWarningReporter`], and marks
//! the result partial.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use gt_types::PointIdx;

use crate::request_plan::{Chunk, ChunkContinuity, RequestPlan, SnapParams};
use crate::snapped_track::{self, Position, SnappedTrackError, SnappedTrackSegment};
use crate::wire::{Edge, SnapPointKind, TraceAttributesResponse};

/// The outcome of sending one chunk, as classified by the transport.
#[derive(Debug, Clone, PartialEq)]
pub enum ChunkOutcome {
    /// The server matched the chunk.
    Success(TraceAttributesResponse),
    /// The server rejected the whole chunk with error 444: every point is
    /// off the road network. Captured reality: this arrives instead of
    /// per-point `unmatched`, so merging maps it to all-unsnapped points.
    OffNetwork,
    /// The chunk failed even after the transport's retry. The string is the
    /// transport's rendered error. Its points have no data.
    Failed(String),
}

/// One warning accumulated while merging. Structured per the warning
/// reporter pattern (CODE_STYLE.md). The app determines how to surface them.
///
/// Serde derives exist for persisting a run's warnings with its cached
/// [`SnapResult`]. Snake_case tags keep the stored form stable against
/// variant renames being caught in review.
#[derive(Debug, Clone, PartialEq, strum::EnumCount, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapWarning {
    /// A chunk failed after retry. Its owned points have no snap data.
    ChunkFailed { chunk_index: usize, detail: String },
    /// The server response did not have one matched point per sent point.
    /// Results cannot be mapped back to the track, so the chunk is treated
    /// as failed.
    PointCountMismatch {
        chunk_index: usize,
        sent: usize,
        received: usize,
    },
    /// The chunk's snapped-track geometry was internally inconsistent. The
    /// per-point data is kept but the chunk contributes no geometry.
    Geometry { chunk_index: usize, detail: String },
    /// Chunks were matched against different OSM data versions (the map
    /// updated mid-run). The first version is kept as the result's.
    OsmChangesetMismatch { first: u64, later: u64 },
    /// The server attached warnings to a chunk's response, passed through
    /// verbatim. No live exemplar exists to model them more tightly.
    Server {
        chunk_index: usize,
        warnings: Vec<Value>,
    },
}

/// Accumulates [`SnapWarning`]s across a snap run (interior mutability so
/// the app can share it with the worker thread).
#[derive(Debug, Default)]
pub struct SnapWarningReporter {
    warnings: Mutex<Vec<SnapWarning>>,
}

impl SnapWarningReporter {
    pub fn report(&self, warning: SnapWarning) {
        self.warnings.lock().push(warning);
    }

    /// All warnings reported so far, in report order.
    pub fn warnings(&self) -> Vec<SnapWarning> {
        self.warnings.lock().clone()
    }

    pub fn is_empty(&self) -> bool {
        self.warnings.lock().is_empty()
    }
}

/// Per-kind totals over a run's points with snap data, shown in the
/// side-panel snap status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapKindCounts {
    pub snapped: usize,
    pub interpolated: usize,
    pub unsnapped: usize,
}

impl SnapKindCounts {
    fn count(&mut self, kind: SnapPointKind) {
        match kind {
            SnapPointKind::Snapped => self.snapped += 1,
            SnapPointKind::Interpolated => self.interpolated += 1,
            SnapPointKind::Unsnapped => self.unsnapped += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.snapped + self.interpolated + self.unsnapped
    }
}

/// Snap data for one sent point, addressed by its track index.
///
/// Points of failed chunks have no entry at all - absence of data is not a
/// kind. Unsnapped points have an entry with no error, position, or edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapPoint {
    pub point: PointIdx,
    pub kind: SnapPointKind,
    /// The snap error in meters (`distance_from_trace_point`).
    #[serde(default)]
    pub error_m: Option<f64>,
    /// The snapped position on the road network.
    #[serde(default)]
    pub snapped: Option<Position>,
    /// Index into [`SnapResult::edges`] for hover attributes.
    #[serde(default)]
    pub edge: Option<usize>,
    /// True when no snap data precedes this point: the plan dropped a ghost
    /// run before it, a chunk failed there, or it opens the result. Series
    /// drawn over these points break here.
    #[serde(default)]
    pub follows_gap: bool,
}

/// The merged result of one snap run over one track.
///
/// Serde derives exist for the persistent cache in the recording history
/// database. Optional and collection fields carry `#[serde(default)]` so
/// fields added later decode absent from older stored results. The schema is
/// pinned by a snapshot test (`stored_result_schema`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapResult {
    /// Per sent point with data, ascending by track index. Sent points of
    /// failed chunks are absent.
    #[serde(default)]
    pub points: Vec<SnapPoint>,
    /// The snapped-track geometry, split at discontinuities, unmatched
    /// runs, and failed chunks.
    #[serde(default)]
    pub segments: Vec<SnappedTrackSegment>,
    /// Edge attributes referenced by [`SnapPoint::edge`], concatenated
    /// across chunks.
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default)]
    pub kind_counts: SnapKindCounts,
    /// Most conservative (lowest) confidence across the successful chunks.
    #[serde(default)]
    pub confidence_score: Option<f64>,
    /// OSM data version of the first successful chunk. A mid-run version
    /// change is reported as a warning.
    #[serde(default)]
    pub osm_changeset: Option<u64>,
    /// The parameters the run was requested with, as provenance: staleness
    /// against the current settings compares these.
    pub params: SnapParams,
    /// The `gps_accuracy` the run actually sent: the clamped override when
    /// set, else the eph-derived value. `None` = server default.
    #[serde(default)]
    pub gps_accuracy_sent_m: Option<f64>,
    /// True when at least one chunk failed and left a gap.
    #[serde(default)]
    pub partial: bool,
}

/// Merge chunk outcomes into a [`SnapResult`].
///
/// `outcomes` must be 1:1 with `plan.chunks` - a mismatch is a programming
/// error in the transport, not a recoverable condition.
pub fn merge(
    plan: &RequestPlan,
    params: SnapParams,
    outcomes: &[ChunkOutcome],
    reporter: &SnapWarningReporter,
) -> SnapResult {
    assert_eq!(
        plan.chunks.len(),
        outcomes.len(),
        "one outcome per planned chunk"
    );

    let mut result = SnapResult {
        points: Vec::new(),
        segments: Vec::new(),
        edges: Vec::new(),
        kind_counts: SnapKindCounts::default(),
        confidence_score: None,
        osm_changeset: None,
        gps_accuracy_sent_m: params.gps_accuracy_sent_m(plan.gps_accuracy_m),
        params,
        partial: false,
    };

    let mut previous = PreviousChunk::default();
    for (chunk_index, (chunk, outcome)) in plan.chunks.iter().zip(outcomes).enumerate() {
        previous = merge_chunk(
            &mut result,
            ChunkMergeInput {
                chunk,
                chunk_index,
                outcome,
                previous,
            },
            reporter,
        );
    }

    result
}

/// What a chunk leaves for the one after it.
#[derive(Clone, Copy, Default)]
struct PreviousChunk {
    /// Its trailing geometry segment runs into the next chunk's leading one,
    /// so the two can be joined into a single road.
    geometry_continues: bool,
    /// It produced snap points for the fixes right before the next chunk, so
    /// no gap separates the two.
    produced_points: bool,
}

struct ChunkMergeInput<'a> {
    chunk: &'a Chunk,
    chunk_index: usize,
    outcome: &'a ChunkOutcome,
    previous: PreviousChunk,
}

/// Append one chunk's points and geometry to `result`, returning what it
/// leaves for the next chunk.
fn merge_chunk(
    result: &mut SnapResult,
    ChunkMergeInput {
        chunk,
        chunk_index,
        outcome,
        previous,
    }: ChunkMergeInput<'_>,
    reporter: &SnapWarningReporter,
) -> PreviousChunk {
    // A chunk opening a stretch is separated from the previous one by
    // dropped ghost fixes: joining their geometry would draw a road
    // across the stretch the receiver never measured.
    let join_previous =
        previous.geometry_continues && chunk.continuity == ChunkContinuity::ContinuesPrevious;
    let follows_gap =
        !previous.produced_points || chunk.continuity == ChunkContinuity::OpensStretch;

    let response = match outcome {
        ChunkOutcome::Success(response) => response,
        ChunkOutcome::OffNetwork => {
            for (local, sent) in chunk.owned_sent().iter().enumerate() {
                result.kind_counts.count(SnapPointKind::Unsnapped);
                result.points.push(SnapPoint {
                    point: sent.point,
                    kind: SnapPointKind::Unsnapped,
                    error_m: None,
                    snapped: None,
                    edge: None,
                    follows_gap: follows_gap && local == 0,
                });
            }
            return PreviousChunk {
                geometry_continues: false,
                produced_points: !chunk.owned_sent().is_empty(),
            };
        }
        ChunkOutcome::Failed(detail) => {
            reporter.report(SnapWarning::ChunkFailed {
                chunk_index,
                detail: detail.clone(),
            });
            result.partial = true;
            return PreviousChunk::default();
        }
    };

    if response.snapped_points.len() != chunk.sent.len() {
        reporter.report(SnapWarning::PointCountMismatch {
            chunk_index,
            sent: chunk.sent.len(),
            received: response.snapped_points.len(),
        });
        result.partial = true;
        return PreviousChunk::default();
    }

    if !response.warnings.is_empty() {
        reporter.report(SnapWarning::Server {
            chunk_index,
            warnings: response.warnings.clone(),
        });
    }

    merge_metadata(result, response, reporter);
    let edge_base = result.edges.len();
    result.edges.extend(response.edges.iter().cloned());

    for (local, sent) in chunk.owned_sent().iter().enumerate() {
        let Some(matched) = response.snapped_points.get(chunk.owned.start + local) else {
            continue; // count validated above, defensive only
        };
        result.kind_counts.count(matched.kind);
        result.points.push(SnapPoint {
            point: sent.point,
            kind: matched.kind,
            error_m: matched.distance_from_trace_point,
            snapped: (matched.kind != SnapPointKind::Unsnapped).then_some(Position {
                lat: matched.lat,
                lon: matched.lon,
            }),
            edge: matched
                .edge_index
                .and_then(|e| usize::try_from(e).ok())
                .map(|e| e + edge_base),
            follows_gap: follows_gap && local == 0,
        });
    }

    let (starts_snappable, ends_snappable) = owned_boundary_snappable(chunk, response);
    let sent_points: Vec<PointIdx> = chunk.sent.iter().map(|sent| sent.point).collect();
    let contributed_geometry =
        match snapped_track::snapped_track_segments_in(response, &sent_points, chunk.owned.clone())
        {
            Ok(mut segments) => {
                // Chunk-local edge references become result-global, like
                // the per-point edge indices below.
                for segment in &mut segments {
                    for span in &mut segment.edge_spans {
                        span.edge += edge_base;
                    }
                }
                let contributed = !segments.is_empty();
                let mut segments = segments.into_iter();
                // The cut between two chunks falls mid-road: the
                // previous chunk's trailing segment and this chunk's
                // leading one are the same street. Join them so the
                // snapped track shows no artificial gap at chunk cuts.
                // The let-chain only consumes `next` once `prev` exists,
                // so no segment is ever silently dropped.
                if join_previous
                    && starts_snappable
                    && let Some(prev) = result.segments.last_mut()
                    && let Some(next) = segments.next()
                {
                    let offset = prev.positions.len();
                    prev.positions.extend(next.positions);
                    prev.recorded_points.extend(next.recorded_points);
                    prev.edge_spans
                        .extend(next.edge_spans.into_iter().map(|span| {
                            snapped_track::SnappedEdgeSpan {
                                start: span.start + offset,
                                end: span.end + offset,
                                edge: span.edge,
                            }
                        }));
                }
                result.segments.extend(segments);
                contributed
            }
            Err(err) => {
                reporter.report(SnapWarning::Geometry {
                    chunk_index,
                    detail: geometry_detail(&err),
                });
                false
            }
        };

    PreviousChunk {
        geometry_continues: contributed_geometry && ends_snappable,
        produced_points: !chunk.owned_sent().is_empty(),
    }
}

/// Whether the chunk's first and last owned points were snappable in this
/// response - the boundary continuity used to join geometry across chunk
/// cuts. Both are false for a chunk with no owned points.
fn owned_boundary_snappable(chunk: &Chunk, response: &TraceAttributesResponse) -> (bool, bool) {
    let snappable_at = |local: usize| {
        response
            .snapped_points
            .get(chunk.owned.start + local)
            .is_some_and(|p| p.kind != SnapPointKind::Unsnapped)
    };
    let count = chunk.owned_sent().len();
    let starts = count > 0 && snappable_at(0);
    let ends = count > 0 && count.checked_sub(1).is_some_and(snappable_at);
    (starts, ends)
}

fn merge_metadata(
    result: &mut SnapResult,
    response: &TraceAttributesResponse,
    reporter: &SnapWarningReporter,
) {
    if let Some(confidence) = response.confidence_score {
        result.confidence_score = Some(match result.confidence_score {
            Some(existing) => existing.min(confidence),
            None => confidence,
        });
    }
    match (result.osm_changeset, response.osm_changeset) {
        (None, Some(changeset)) => result.osm_changeset = Some(changeset),
        (Some(first), Some(later)) if first != later => {
            reporter.report(SnapWarning::OsmChangesetMismatch { first, later });
        }
        _ => {}
    }
}

/// Render a geometry error for the warning payload.
///
/// [`SnappedTrackError`] is not `Clone` (it carries the polyline decode
/// error), so the warning stores the display form.
fn geometry_detail(err: &SnappedTrackError) -> String {
    format!("{err:#}")
}
