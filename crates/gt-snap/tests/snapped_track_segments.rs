//! Validate snapped-track segment assembly against the captured fixtures and
//! synthetic break scenarios.

use std::fs;

use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

use gt_snap::fixtures_dir;
use gt_snap::snapped_track::{
    self, SHAPE_POLYLINE_PRECISION, SnappedTrackError, SnappedTrackSegment,
};
use gt_snap::wire::{SnapPointKind, TraceAttributesResponse};

fn parse_response(scenario: &str) -> Result<TraceAttributesResponse, String> {
    let path = fixtures_dir().join(format!("{scenario}.response.json"));
    let body =
        fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))?;
    serde_json::from_str(&body).map_err(|err| format!("{scenario}: {err}"))
}

/// A digest of assembled segments, sized for snapshot review: per segment
/// the vertex count, first and last position, and the edge-span coverage
/// (span count plus the covered vertex range and referenced edge range).
fn digest(segments: &[SnappedTrackSegment]) -> Vec<String> {
    segments
        .iter()
        .map(|segment| {
            let ends = match (segment.positions.first(), segment.positions.last()) {
                (Some(first), Some(last)) => format!(
                    "({:.6}, {:.6}) -> ({:.6}, {:.6})",
                    first.lat, first.lon, last.lat, last.lon
                ),
                _ => "empty".to_owned(),
            };
            let spans = match (segment.edge_spans.first(), segment.edge_spans.last()) {
                (Some(first), Some(last)) => format!(
                    "{} spans covering {}..{} via edges {}..={}",
                    segment.edge_spans.len(),
                    first.start,
                    last.end,
                    first.edge,
                    last.edge,
                ),
                _ => "no spans".to_owned(),
            };
            format!("{} positions, {ends}, {spans}", segment.positions.len())
        })
        .collect()
}

/// Edge spans are internally coherent on every fixture: sorted, within the
/// segment's vertices, gapless between consecutive spans (adjacent edges
/// share their boundary vertex), and referencing existing edges.
///
/// Full coverage is a captured-reality pin, not a general guarantee: all
/// four fixtures happen to have contiguous edge shape ranges. The contract
/// explicitly permits uncovered vertices (see
/// [`SnappedTrackSegment::edge_spans`] and
/// `shape_index_gaps_leave_vertices_uncovered`).
#[rstest]
#[case::clean_drive("clean_drive")]
#[case::dense_10hz("dense_10hz")]
#[case::partially_snappable("partially_snappable")]
#[case::teleport_gap("teleport_gap")]
fn edge_spans_cover_segments_coherently(#[case] scenario: &str) {
    let response = parse_response(scenario).expect("fixture parses");
    let segments = snapped_track::snapped_track_segments(&response).expect("segments assemble");
    assert!(!segments.is_empty());
    for segment in &segments {
        assert!(!segment.edge_spans.is_empty(), "every segment has spans");
        let mut previous_end = 0;
        for span in &segment.edge_spans {
            assert!(span.start < span.end, "spans are non-empty");
            assert!(span.end <= segment.positions.len(), "spans stay in bounds");
            assert!(
                span.start <= previous_end,
                "no vertex gap between consecutive spans"
            );
            assert!(
                response.edges.get(span.edge).is_some(),
                "span references an existing edge"
            );
            previous_end = previous_end.max(span.end);
        }
        assert_eq!(
            previous_end,
            segment.positions.len(),
            "captured reality: these fixtures' edge ranges are contiguous, \
             so coverage is total here (not a general guarantee)"
        );
    }
}

/// Every success fixture assembles without error; the digests pin segment
/// counts and endpoints. `teleport_gap` must split into two segments (the
/// break shows only as an unmatched run - no discontinuity flags), and
/// `partially_snappable` splits on its flagged discontinuity.
#[rstest]
#[case::clean_drive("clean_drive")]
#[case::dense_10hz("dense_10hz")]
#[case::partially_snappable("partially_snappable")]
#[case::teleport_gap("teleport_gap")]
fn fixture_segments_assemble(#[case] scenario: &str) {
    let response = parse_response(scenario).expect("fixture");
    let segments = snapped_track::snapped_track_segments(&response).expect("assembles");
    insta::assert_debug_snapshot!(format!("{scenario}_segments"), digest(&segments));
}

#[test]
fn teleport_gap_splits_into_two_segments() {
    let response = parse_response("teleport_gap").expect("fixture");
    let segments = snapped_track::snapped_track_segments(&response).expect("assembles");
    assert_eq!(
        segments.len(),
        2,
        "the 4 km jump must break the snapped track even without discontinuity flags"
    );
}

/// The unfiltered fixture carries every edge attribute; segment assembly
/// must work there too (it is the same clean drive).
#[test]
fn unfiltered_fixture_assembles_like_filtered() {
    let filtered = parse_response("clean_drive").expect("fixture");
    let unfiltered = parse_response("clean_drive_unfiltered").expect("fixture");
    assert_eq!(
        snapped_track::snapped_track_segments(&filtered).expect("filtered assembles"),
        snapped_track::snapped_track_segments(&unfiltered).expect("unfiltered assembles"),
    );
}

/// A response with a valid 4-position synthetic shape and the given points
/// and edges. The shape is encoded with the wire's precision constant, so
/// encode and decode cannot drift apart.
fn synthetic_response(points: &Value, edges: &Value) -> Result<TraceAttributesResponse, String> {
    // Four positions spaced ~110 m apart along a meridian.
    let line: geo_types::LineString<f64> =
        vec![(12.0, 55.0), (12.0, 55.001), (12.0, 55.002), (12.0, 55.003)].into();
    let shape = polyline::encode_coordinates(line, SHAPE_POLYLINE_PRECISION)
        .map_err(|err| err.to_string())?;
    serde_json::from_value(json!({
        "shape": shape,
        "matched_points": points,
        "edges": edges,
    }))
    .map_err(|err| err.to_string())
}

fn point(kind: SnapPointKind, edge_index: Option<u64>) -> Value {
    json!({
        "lat": 55.0, "lon": 12.0, "type": kind.to_string(),
        "edge_index": edge_index,
    })
}

fn flagged(point: Value, flag: &str) -> Value {
    let mut point = point;
    if let Value::Object(map) = &mut point {
        map.insert(flag.to_owned(), json!(true));
    }
    point
}

/// Two edges covering disjoint halves of the synthetic shape.
fn two_edges() -> Value {
    json!([
        { "begin_shape_index": 0, "end_shape_index": 1 },
        { "begin_shape_index": 2, "end_shape_index": 3 },
    ])
}

/// Split behavior over synthetic point sequences: expected segment count and
/// per-segment position counts.
#[rstest]
#[case::end_flag_splits(
    json!([
        flagged(point(SnapPointKind::Snapped, Some(0)), "end_route_discontinuity"),
        point(SnapPointKind::Snapped, Some(1)),
    ]),
    two_edges(),
    vec![2, 2]
)]
#[case::begin_flag_splits(
    json!([
        point(SnapPointKind::Snapped, Some(0)),
        flagged(point(SnapPointKind::Snapped, Some(1)), "begin_route_discontinuity"),
    ]),
    two_edges(),
    vec![2, 2]
)]
#[case::unsnapped_run_splits(
    json!([
        point(SnapPointKind::Snapped, Some(0)),
        point(SnapPointKind::Unsnapped, None),
        point(SnapPointKind::Snapped, Some(1)),
    ]),
    two_edges(),
    vec![2, 2]
)]
#[case::leading_and_trailing_unsnapped_trimmed(
    json!([
        point(SnapPointKind::Unsnapped, None),
        point(SnapPointKind::Snapped, Some(0)),
        point(SnapPointKind::Interpolated, Some(0)),
        point(SnapPointKind::Unsnapped, None),
    ]),
    json!([{ "begin_shape_index": 0, "end_shape_index": 3 }]),
    vec![4]
)]
#[case::all_unsnapped_yields_nothing(
    json!([point(SnapPointKind::Unsnapped, None), point(SnapPointKind::Unsnapped, None)]),
    json!([]),
    vec![]
)]
fn synthetic_split_behavior(
    #[case] points: Value,
    #[case] edges: Value,
    #[case] expected_position_counts: Vec<usize>,
) {
    let response = synthetic_response(&points, &edges).expect("synthetic response");
    let segments = snapped_track::snapped_track_segments(&response).expect("assembles");
    let position_counts: Vec<usize> = segments.iter().map(|s| s.positions.len()).collect();
    assert_eq!(position_counts, expected_position_counts);
}

/// A shape-index gap between two edges (legal per the wire format - see
/// the module doc on non-contiguous edge ranges) leaves the gapped
/// vertices uncovered by every span: the documented "uncovered" branch of
/// the span contract, exercised positively since no fixture captures it.
#[test]
fn shape_index_gaps_leave_vertices_uncovered() {
    let points = json!([
        point(SnapPointKind::Snapped, Some(0)),
        point(SnapPointKind::Snapped, Some(1)),
    ]);
    // Edge 0 covers shape 0..=1, edge 1 covers 3..=3: vertex 2 is gapped.
    let edges = json!([
        { "begin_shape_index": 0, "end_shape_index": 1 },
        { "begin_shape_index": 3, "end_shape_index": 3 },
    ]);
    let response = synthetic_response(&points, &edges).expect("synthetic response");
    let segments = snapped_track::snapped_track_segments(&response).expect("assembles");

    let segment = segments.first().expect("one segment");
    assert_eq!(segments.len(), 1, "an index gap alone is not a break");
    assert_eq!(segment.positions.len(), 4);
    let covered = |vertex: usize| {
        segment
            .edge_spans
            .iter()
            .any(|span| span.start <= vertex && vertex < span.end)
    };
    assert!(covered(0) && covered(1) && covered(3));
    assert!(!covered(2), "the gapped vertex stays uncovered");
}

#[test]
fn no_shape_yields_no_segments() {
    let response: TraceAttributesResponse =
        serde_json::from_value(json!({ "matched_points": [] })).expect("parse");
    assert_eq!(
        snapped_track::snapped_track_segments(&response).expect("assembles"),
        Vec::new()
    );
}

/// Inconsistent responses: expected error per malformed points/edges pair.
#[rstest]
#[case::edge_reference_out_of_bounds(
    json!([point(SnapPointKind::Snapped, Some(7))]),
    json!([]),
    SnappedTrackError::EdgeIndexOutOfBounds { edge: 7, edges: 0 }
)]
#[case::shape_index_beyond_shape(
    json!([point(SnapPointKind::Snapped, Some(0))]),
    json!([{ "begin_shape_index": 0, "end_shape_index": 99 }]),
    SnappedTrackError::ShapeIndexOutOfBounds { edge: 0, begin: 0, end: 99, points: 4 }
)]
#[case::missing_shape_range(
    json!([point(SnapPointKind::Snapped, Some(0))]),
    json!([{}]),
    SnappedTrackError::MissingShapeRange { edge: 0 }
)]
fn inconsistent_responses_error(
    #[case] points: Value,
    #[case] edges: Value,
    #[case] expected: SnappedTrackError,
) {
    let response = synthetic_response(&points, &edges).expect("synthetic response");
    assert_eq!(
        snapped_track::snapped_track_segments(&response),
        Err(expected)
    );
}

/// A garbage shape string from the (untrusted) server must surface as
/// `UndecodableShape`, never a panic.
#[test]
fn garbage_shape_is_an_error() {
    let response: TraceAttributesResponse = serde_json::from_value(json!({
        "shape": "!!!not-a-polyline\u{7f}",
        "matched_points": [point(SnapPointKind::Snapped, Some(0))],
        "edges": [{ "begin_shape_index": 0, "end_shape_index": 0 }],
    }))
    .expect("parse");
    assert!(matches!(
        snapped_track::snapped_track_segments(&response),
        Err(SnappedTrackError::UndecodableShape(_))
            | Err(SnappedTrackError::ShapeIndexOutOfBounds { .. })
    ));
}

proptest! {
    /// The decoder consumes untrusted network bytes: any string - random
    /// garbage or a mutilated real polyline - must produce `Ok` or `Err`,
    /// never a panic. (The real captured shapes are exercised by the fixture
    /// tests above; this covers everything else.)
    #[test]
    fn arbitrary_shape_strings_never_panic(shape in ".{0,256}") {
        let response: TraceAttributesResponse = serde_json::from_value(json!({
            "shape": shape,
            "matched_points": [point(SnapPointKind::Snapped, Some(0))],
            "edges": [{ "begin_shape_index": 0, "end_shape_index": 0 }],
        }))
        .expect("parse");
        snapped_track::snapped_track_segments(&response).ok();
    }

    /// Truncating a real captured polyline at any byte boundary must not
    /// panic either.
    #[test]
    fn truncated_real_shape_never_panics(cut in 0usize..200) {
        let response = parse_response("clean_drive").expect("fixture");
        let Some(shape) = response.shape.as_deref() else {
            return Ok(());
        };
        let cut = cut.min(shape.len());
        // Slice at a char boundary (polylines are ASCII, but stay safe).
        let truncated: String = shape.chars().take(cut).collect();
        let response: TraceAttributesResponse = serde_json::from_value(json!({
            "shape": truncated,
            "matched_points": [point(SnapPointKind::Snapped, Some(0))],
            "edges": [{ "begin_shape_index": 0, "end_shape_index": 0 }],
        }))
        .expect("parse");
        snapped_track::snapped_track_segments(&response).ok();
    }
}
