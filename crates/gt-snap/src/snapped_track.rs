//! Decode a response's matched shape into snapped-track polyline segments.
//!
//! The server returns the matched geometry as ONE encoded polyline even when
//! the match breaks (captured reality: a 4 km teleport is a plain geometric
//! jump inside the shape). The reliable break evidence lives in the snapped
//! points: explicit route discontinuity flags, and runs of unsnapped points.
//! Each break splits the shape between the edge coverage of the two
//! neighboring point groups, via the edges' `begin_shape_index` /
//! `end_shape_index`.
//!
//! Index gaps between consecutive edges are NOT breaks: captured reality
//! shows non-contiguous edge ranges on unbroken routes, so splitting is
//! driven exclusively by point evidence.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::wire::{SnapPointKind, SnappedPoint, TraceAttributesResponse};

/// Precision of the wire's encoded shape polylines: 6 decimal digits
/// (`trace_attributes` returns "6 digit precision" shapes). Decode and any
/// test-side encode must share this constant or they silently drift.
pub const SHAPE_POLYLINE_PRECISION: u32 = 6;

/// One vertex of the snapped track, degrees.
///
/// Deliberately a named-field struct rather than a tuple or `geo` type: the
/// polyline decoder speaks x/y, and lat/lon transpositions are exactly the
/// bug class named fields prevent.
///
/// Serde derives exist for persisting cached snap results (see
/// [`crate::stitch::SnapResult`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
}

/// A maximal unbroken stretch of the snapped track, ready to draw as one
/// polyline. Breaks between segments render as gaps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnappedTrackSegment {
    pub positions: Vec<Position>,
    /// Which matched edge each vertex run came from, for hover attributes.
    /// Sorted by start; spans of adjacent edges overlap at their shared
    /// boundary vertex (lookups take the first covering span). Vertices of
    /// shape stretches without an edge range are simply uncovered.
    #[serde(default)]
    pub edge_spans: Vec<SnappedEdgeSpan>,
}

/// A run of segment vertices (`start..end`, exclusive) matched to the edge
/// at `edge` (an index into the response's - after stitching, the
/// [`crate::stitch::SnapResult`]'s - edge list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnappedEdgeSpan {
    pub start: usize,
    pub end: usize,
    pub edge: usize,
}

/// Why the snapped track could not be assembled from a response.
///
/// These indicate response inconsistencies (drift between the shape, the
/// points, and the edges), not user-facing conditions; the caller reports
/// them per chunk through the warning reporter.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum SnappedTrackError {
    #[error("undecodable shape polyline: {0}")]
    UndecodableShape(#[from] polyline::errors::PolylineError),
    #[error("point references edge {edge} but the response has {edges} edges")]
    EdgeIndexOutOfBounds { edge: u64, edges: usize },
    #[error("edge {edge} covers shape indices {begin}..={end} but the shape has {points} points")]
    ShapeIndexOutOfBounds {
        edge: u64,
        begin: usize,
        end: usize,
        points: usize,
    },
    #[error("edge {edge} carries no shape index range")]
    MissingShapeRange { edge: u64 },
}

/// Decode the response shape and split it into snapped-track segments.
///
/// A response without a shape (or without any snapped point) yields no
/// segments. Unsnapped points contribute no geometry; a run of them between
/// snapped points is a break, as are the explicit discontinuity flags.
pub fn snapped_track_segments(
    response: &TraceAttributesResponse,
) -> Result<Vec<SnappedTrackSegment>, SnappedTrackError> {
    snapped_track_segments_in(response, 0..response.snapped_points.len())
}

/// Like [`snapped_track_segments`], but considering only the points in
/// `points` (a range of indices into the response's snapped points).
///
/// Stitching uses this to build each chunk's geometry from its owned points
/// only, so overlap regions are not drawn twice.
pub fn snapped_track_segments_in(
    response: &TraceAttributesResponse,
    points: Range<usize>,
) -> Result<Vec<SnappedTrackSegment>, SnappedTrackError> {
    let Some(encoded) = response.shape.as_deref() else {
        return Ok(Vec::new());
    };
    let shape = decode_shape(encoded)?;

    let considered = response.snapped_points.get(points).unwrap_or_default();
    let mut segments = Vec::new();
    for group in point_groups(considered) {
        if let Some((begin, end)) = group_shape_range(&group, response, shape.len())? {
            segments.push(SnappedTrackSegment {
                positions: shape
                    .get(begin..=end)
                    .unwrap_or_default() // range is validated above; defensive only
                    .to_vec(),
                edge_spans: edge_spans_for(begin, end, &response.edges),
            });
        }
    }
    Ok(segments)
}

/// The edge spans of a segment covering shape indices `begin..=end`: every
/// response edge whose shape range intersects it, as segment-local vertex
/// runs. See [`SnappedTrackSegment::edge_spans`] for the overlap contract.
fn edge_spans_for(begin: usize, end: usize, edges: &[crate::wire::Edge]) -> Vec<SnappedEdgeSpan> {
    let mut spans: Vec<SnappedEdgeSpan> = edges
        .iter()
        .enumerate()
        .filter_map(|(edge, e)| {
            let (Some(edge_begin), Some(edge_end)) = (e.begin_shape_index, e.end_shape_index)
            else {
                return None;
            };
            let lo = edge_begin.max(begin);
            let hi = edge_end.min(end);
            // Lazy `then`: the subtractions underflow for edges entirely
            // outside the segment, so they must not evaluate eagerly.
            (lo <= hi).then(|| SnappedEdgeSpan {
                start: lo - begin,
                end: hi - begin + 1,
                edge,
            })
        })
        .collect();
    spans.sort_unstable_by_key(|span| (span.start, span.end));
    spans
}

/// Decode the wire polyline into positions.
fn decode_shape(encoded: &str) -> Result<Vec<Position>, SnappedTrackError> {
    let line = polyline::decode_polyline(encoded, SHAPE_POLYLINE_PRECISION)?;
    Ok(line
        .coords()
        .map(|coord| Position {
            lat: coord.y,
            lon: coord.x,
        })
        .collect())
}

/// Group consecutive snapped/interpolated points into unbroken stretches.
///
/// A new group starts after a run of unsnapped points, before a point with
/// `begin_route_discontinuity`, and after a point with
/// `end_route_discontinuity`.
fn point_groups(points: &[SnappedPoint]) -> Vec<Vec<&SnappedPoint>> {
    let mut groups: Vec<Vec<&SnappedPoint>> = Vec::new();
    let mut current: Vec<&SnappedPoint> = Vec::new();
    let mut gap_pending = false;
    for point in points {
        if point.kind == SnapPointKind::Unsnapped {
            gap_pending = !current.is_empty();
            continue;
        }
        if (gap_pending || point.begin_route_discontinuity) && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        gap_pending = false;
        current.push(point);
        if point.end_route_discontinuity {
            groups.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

/// The shape index range covered by a group's edges, or `None` for a group
/// with no edge association at all.
///
/// Ranges from a group's edges are unioned via min/max; edges are not
/// required to appear in shape-index order within a group.
fn group_shape_range(
    group: &[&SnappedPoint],
    response: &TraceAttributesResponse,
    shape_points: usize,
) -> Result<Option<(usize, usize)>, SnappedTrackError> {
    let mut range: Option<(usize, usize)> = None;
    for point in group {
        let Some(raw_edge_index) = point.edge_index else {
            continue;
        };
        let edge = usize::try_from(raw_edge_index)
            .ok()
            .and_then(|index| response.edges.get(index));
        let Some(edge) = edge else {
            return Err(SnappedTrackError::EdgeIndexOutOfBounds {
                edge: raw_edge_index,
                edges: response.edges.len(),
            });
        };
        let (Some(begin), Some(end)) = (edge.begin_shape_index, edge.end_shape_index) else {
            return Err(SnappedTrackError::MissingShapeRange {
                edge: raw_edge_index,
            });
        };
        if end >= shape_points || begin > end {
            return Err(SnappedTrackError::ShapeIndexOutOfBounds {
                edge: raw_edge_index,
                begin,
                end,
                points: shape_points,
            });
        }
        range = Some(match range {
            Some((lo, hi)) => (lo.min(begin), hi.max(end)),
            None => (begin, end),
        });
    }
    Ok(range)
}
