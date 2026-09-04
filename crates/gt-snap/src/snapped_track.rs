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

use std::ops::{Range, RangeInclusive};

use gt_types::{Latitude, Longitude, PointIdx};
use serde::{Deserialize, Serialize};

use crate::wire::{Edge, SnapPointKind, SnappedPoint, TraceAttributesResponse};

/// Precision of the wire's encoded shape polylines: 6 decimal digits
/// (`trace_attributes` returns "6 digit precision" shapes). Decode and any
/// test-side encode must share this constant or they silently drift.
pub const SHAPE_POLYLINE_PRECISION: u32 = 6;

/// One vertex of the snapped track, degrees.
///
/// Named fields guard against lat/lon transposition: the polyline decoder
/// speaks x/y.
///
/// Serde derives exist for persisting cached snap results (see
/// [`crate::merge::SnapResult`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub lat: f64,
    pub lon: f64,
}

impl Position {
    /// `None` when the server returned degrees outside their axis' range.
    pub fn coordinates(self) -> Option<(Latitude, Longitude)> {
        Some((
            Latitude::try_new(self.lat).ok()?,
            Longitude::try_new(self.lon).ok()?,
        ))
    }
}

/// A maximal unbroken stretch of the snapped track, ready to draw as one
/// polyline. Breaks between segments render as gaps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnappedTrackSegment {
    pub positions: Vec<Position>,
    /// Which matched edge each vertex run came from, for hover attributes.
    /// Sorted by start. Spans of adjacent edges overlap at their shared
    /// boundary vertex (lookups take the first covering span). Vertices of
    /// shape stretches without an edge range are simply uncovered.
    #[serde(default)]
    pub edge_spans: Vec<SnappedEdgeSpan>,
    /// The recorded point each vertex was matched from, one entry per
    /// position. Empty for a result stored before this field existed: such a
    /// segment cannot be trimmed to a time window.
    #[serde(default)]
    pub recorded_points: Vec<PointIdx>,
}

/// A run of segment vertices (`start..end`, exclusive) matched to the edge
/// at `edge` (an index into the response's - after merging, the
/// [`crate::merge::SnapResult`]'s - edge list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnappedEdgeSpan {
    pub start: usize,
    pub end: usize,
    pub edge: usize,
}

/// Why the snapped track could not be assembled from a response.
///
/// These indicate response inconsistencies (drift between the shape, the
/// points, and the edges), not user-facing conditions. The caller reports
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
/// A response without a shape (or without any snapped point) yields an empty
/// list of segments. Unsnapped points contribute no geometry. A run of them
/// between snapped points is a break, as are the explicit discontinuity flags.
///
/// The vertex attribution identifies the recorded point at each snapped point's
/// own index. That mapping holds for a request that sent the whole track.
pub fn snapped_track_segments(
    response: &TraceAttributesResponse,
) -> Result<Vec<SnappedTrackSegment>, SnappedTrackError> {
    let sent_points: Vec<PointIdx> = (0..response.snapped_points.len())
        .map(PointIdx::new)
        .collect();
    snapped_track_segments_in(response, &sent_points, 0..response.snapped_points.len())
}

/// Like [`snapped_track_segments`], but considering only the points in
/// `points` (a range of indices into the response's snapped points), and
/// attributing vertices via `sent_points`, which identifies the recorded point
/// behind each of the response's snapped points.
///
/// Merging uses this to build each chunk's geometry from its owned points
/// only, so overlap regions are not drawn twice.
pub fn snapped_track_segments_in(
    response: &TraceAttributesResponse,
    sent_points: &[PointIdx],
    points: Range<usize>,
) -> Result<Vec<SnappedTrackSegment>, SnappedTrackError> {
    let Some(encoded) = response.shape.as_deref() else {
        return Ok(Vec::new());
    };
    let shape = decode_shape(encoded)?;

    let considered: Vec<MatchedPoint<'_>> = response
        .snapped_points
        .get(points.clone())
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(offset, point)| MatchedPoint {
            index: points.start + offset,
            point,
        })
        .collect();
    let mut segments = Vec::new();
    for group in point_groups(&considered) {
        if let Some(shape_range) = group_shape_range(&group, response, shape.len())? {
            let positions = shape
                .get(shape_range.vertices())
                .unwrap_or_default() // range is validated above, defensive only
                .to_vec();
            let recorded_points =
                recorded_point_per_vertex(shape_range, &group, &response.edges, sent_points);
            debug_assert!(
                recorded_points.is_empty() || recorded_points.len() == positions.len(),
                "a segment is attributed at every vertex or at none"
            );
            segments.push(SnappedTrackSegment {
                positions,
                edge_spans: edge_spans_for(shape_range, &response.edges),
                recorded_points,
            });
        }
    }
    Ok(segments)
}

/// One of a response's snapped points with its index in that response, which
/// the caller's sent points are 1:1 with.
#[derive(Debug, Clone, Copy)]
struct MatchedPoint<'a> {
    index: usize,
    point: &'a SnappedPoint,
}

/// A stretch of a response's decoded shape, both ends inclusive: what one
/// segment is cut from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShapeRange {
    begin: usize,
    end: usize,
}

impl ShapeRange {
    fn vertices(self) -> RangeInclusive<usize> {
        self.begin..=self.end
    }
}

/// The edge spans of a segment cut from `shape`: every response edge whose
/// shape range intersects it, as segment-local vertex runs. See
/// [`SnappedTrackSegment::edge_spans`] for the overlap contract.
fn edge_spans_for(shape: ShapeRange, edges: &[Edge]) -> Vec<SnappedEdgeSpan> {
    let mut spans: Vec<SnappedEdgeSpan> = edges
        .iter()
        .enumerate()
        .filter_map(|(edge, e)| {
            let (Some(edge_begin), Some(edge_end)) = (e.begin_shape_index, e.end_shape_index)
            else {
                return None;
            };
            let lo = edge_begin.max(shape.begin);
            let hi = edge_end.min(shape.end);
            // Lazy `then`: the subtractions underflow for edges entirely
            // outside the segment, so they must not evaluate eagerly.
            (lo <= hi).then(|| SnappedEdgeSpan {
                start: lo - shape.begin,
                end: hi - shape.begin + 1,
                edge,
            })
        })
        .collect();
    spans.sort_unstable_by_key(|span| (span.start, span.end));
    spans
}

/// Where along a response's shape the server placed a matched point: its
/// edge's shape range, indexed by `distance_along_edge`. `None` for a point
/// the server matched to no edge, which is how it reports an interpolated
/// one.
fn shape_position(point: &SnappedPoint, edges: &[Edge]) -> Option<f64> {
    let edge = point
        .edge_index
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| edges.get(index))?;
    let begin = edge.begin_shape_index?;
    let end = edge.end_shape_index?;
    let fraction = point
        .distance_along_edge
        .filter(|fraction| fraction.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    Some(begin as f64 + fraction * end.saturating_sub(begin) as f64)
}

/// The recorded point each vertex of a segment cut from `shape` was matched
/// from.
///
/// Each vertex takes the last group point [`shape_position`] places at or
/// before it, and the vertices before the first placed point take that point.
/// Points sharing a position resolve to the earliest of them: a response
/// without `distance_along_edge` attributes the whole segment to one recorded
/// point.
///
/// The result is empty for a group whose points are placed nowhere.
fn recorded_point_per_vertex(
    shape: ShapeRange,
    group: &[MatchedPoint<'_>],
    edges: &[Edge],
    sent_points: &[PointIdx],
) -> Vec<PointIdx> {
    let mut placed: Vec<(f64, PointIdx)> = group
        .iter()
        .filter_map(|entry| {
            Some((
                shape_position(entry.point, edges)?,
                *sent_points.get(entry.index)?,
            ))
        })
        .collect();
    // A group's edges can appear out of shape-index order: the sort orders by
    // shape position, and its stability lets the dedup keep the earliest point
    // at each position.
    placed.sort_by(|(left, _), (right, _)| left.total_cmp(right));
    placed.dedup_by(|(left, _), (right, _)| left.total_cmp(right).is_eq());

    let mut per_vertex = Vec::with_capacity(shape.vertices().count());
    let mut current = 0;
    for vertex in shape.vertices() {
        while placed
            .get(current + 1)
            .is_some_and(|&(position, _)| position <= vertex as f64)
        {
            current += 1;
        }
        let Some(&(_, recorded)) = placed.get(current) else {
            return Vec::new();
        };
        per_vertex.push(recorded);
    }
    per_vertex
}

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
fn point_groups<'a>(points: &[MatchedPoint<'a>]) -> Vec<Vec<MatchedPoint<'a>>> {
    let mut groups: Vec<Vec<MatchedPoint<'a>>> = Vec::new();
    let mut current: Vec<MatchedPoint<'a>> = Vec::new();
    let mut gap_pending = false;
    for entry in points {
        if entry.point.kind == SnapPointKind::Unsnapped {
            gap_pending = !current.is_empty();
            continue;
        }
        if (gap_pending || entry.point.begin_route_discontinuity) && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        gap_pending = false;
        current.push(*entry);
        if entry.point.end_route_discontinuity {
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
/// Ranges from a group's edges are unioned via min/max. Edges are not
/// required to appear in shape-index order within a group.
fn group_shape_range(
    group: &[MatchedPoint<'_>],
    response: &TraceAttributesResponse,
    shape_points: usize,
) -> Result<Option<ShapeRange>, SnappedTrackError> {
    let mut range: Option<ShapeRange> = None;
    for entry in group {
        let Some(raw_edge_index) = entry.point.edge_index else {
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
            Some(range) => ShapeRange {
                begin: range.begin.min(begin),
                end: range.end.max(end),
            },
            None => ShapeRange { begin, end },
        });
    }
    Ok(range)
}
