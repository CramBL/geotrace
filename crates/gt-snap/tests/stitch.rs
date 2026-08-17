//! Validate stitching of chunk outcomes into a `SnapResult`.

mod support;

use std::fs;

use serde_json::{Value, json};
use support::points;

use gt_snap::fixtures_dir;
use gt_snap::request_plan::{self, CHUNK_POINTS, RequestPlan, SnapParams};
use gt_snap::snapped_track::SHAPE_POLYLINE_PRECISION;
use gt_snap::stitch::{self, ChunkOutcome, SnapWarning, SnapWarningReporter};
use gt_snap::wire::{Costing, SnapPointKind, TraceAttributesResponse};

/// The params every scenario in this file stitches with: default advanced
/// options, auto costing.
fn auto_params() -> SnapParams {
    SnapParams::new(Costing::Auto)
}

/// A synthetic success response with `count` matched points, every one
/// `kind`, all errors `error_m`, no shape (geometry is exercised separately
/// by the fixture test), one shared edge when kind is not unsnapped.
fn uniform_response(
    count: usize,
    kind: SnapPointKind,
    error_m: f64,
) -> Result<ChunkOutcome, String> {
    let unsnapped = kind == SnapPointKind::Unsnapped;
    let matched_points: Vec<Value> = (0..count)
        .map(|i| {
            if unsnapped {
                json!({ "lat": 55.0, "lon": 12.0, "type": kind.to_string() })
            } else {
                json!({
                    "lat": 55.0 + i as f64 * 1e-5,
                    "lon": 12.0,
                    "type": kind.to_string(),
                    "edge_index": 0,
                    "distance_from_trace_point": error_m,
                })
            }
        })
        .collect();
    let edges = if unsnapped {
        json!([])
    } else {
        json!([{ "names": ["Synthetic street"], "road_class": "residential" }])
    };
    let response: TraceAttributesResponse = serde_json::from_value(json!({
        "matched_points": matched_points,
        "edges": edges,
        "confidence_score": 0.9,
        "osm_changeset": 100,
    }))
    .map_err(|err| err.to_string())?;
    Ok(ChunkOutcome::Success(response))
}

/// A two-chunk plan (CHUNK_POINTS + 1 sent points at 1 Hz).
fn two_chunk_plan() -> RequestPlan {
    let plan = request_plan::plan(&points(CHUNK_POINTS + 1));
    assert_eq!(plan.chunks.len(), 2, "precondition");
    plan
}

fn chunk_sizes(plan: &RequestPlan) -> Vec<usize> {
    plan.chunks.iter().map(|c| c.sent.len()).collect()
}

#[test]
fn single_fixture_chunk_stitches_into_result() {
    let body = fs::read_to_string(fixtures_dir().join("partially_snappable.response.json"))
        .expect("fixture");
    let response: TraceAttributesResponse = serde_json::from_str(&body).expect("parse");
    let sent_count = response.snapped_points.len();

    let plan = request_plan::plan(&points(sent_count));
    assert_eq!(plan.chunks.len(), 1);

    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(
        &plan,
        auto_params(),
        &[ChunkOutcome::Success(response)],
        &reporter,
    );

    assert!(reporter.is_empty());
    assert!(!result.partial);
    insta::assert_debug_snapshot!((
        result.kind_counts,
        result.points.len(),
        result.segments.len(),
        result.confidence_score,
        result.osm_changeset,
    ));
}

#[test]
fn owned_ranges_select_results_across_chunks() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    // Chunk 0 reports every point snapped, chunk 1 every point
    // interpolated. Overlap points must take the owning chunk's result.
    let outcomes = [
        uniform_response(sizes[0], SnapPointKind::Snapped, 1.0).expect("outcome"),
        uniform_response(sizes[1], SnapPointKind::Interpolated, 2.0).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    assert_eq!(result.points.len(), plan.sent_point_count());
    assert_eq!(
        result.points.iter().filter(|p| p.follows_gap).count(),
        1,
        "only the first point of an uninterrupted run follows a gap"
    );
    // Every point appears exactly once, ascending by track index.
    let indices: Vec<usize> = result.points.iter().map(|p| p.point.as_usize()).collect();
    let mut sorted = indices.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(indices, sorted);
    // Ownership split: chunk 0 owns its interior plus the first half of the
    // overlap. The counts must reflect the owned ranges, not chunk sizes.
    let expected_snapped = plan.chunks[0].owned.len();
    let expected_interpolated = plan.chunks[1].owned.len();
    assert_eq!(result.kind_counts.snapped, expected_snapped);
    assert_eq!(result.kind_counts.interpolated, expected_interpolated);
    // Errors follow the owning chunk too.
    let first_interpolated = result
        .points
        .iter()
        .find(|p| p.kind == SnapPointKind::Interpolated)
        .expect("interpolated points exist");
    assert_eq!(first_interpolated.error_m, Some(2.0));
}

#[test]
fn off_network_chunk_becomes_unsnapped_points() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        uniform_response(sizes[0], SnapPointKind::Snapped, 1.0).expect("outcome"),
        ChunkOutcome::OffNetwork,
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    assert!(reporter.is_empty(), "off-network is not a failure");
    assert!(!result.partial);
    assert_eq!(result.points.len(), plan.sent_point_count());
    assert_eq!(
        result.points.iter().filter(|p| p.follows_gap).count(),
        1,
        "an off-network chunk leaves data, so it opens no gap for the next"
    );
    assert_eq!(result.kind_counts.unsnapped, plan.chunks[1].owned.len());
    let unsnapped = result
        .points
        .iter()
        .find(|p| p.kind == SnapPointKind::Unsnapped)
        .expect("unsnapped points exist");
    assert_eq!(unsnapped.error_m, None);
    assert_eq!(unsnapped.snapped, None);
    assert_eq!(unsnapped.edge, None);
}

#[test]
fn failed_chunk_leaves_gap_and_marks_partial() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        ChunkOutcome::Failed("connection reset".to_owned()),
        uniform_response(sizes[1], SnapPointKind::Snapped, 1.0).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    assert!(result.partial);
    assert_eq!(result.points.len(), plan.chunks[1].owned.len());
    assert_eq!(
        reporter.warnings(),
        vec![SnapWarning::ChunkFailed {
            chunk_index: 0,
            detail: "connection reset".to_owned(),
        }]
    );
    // The failed chunk's owned points are absent, not unsnapped: absence of
    // data is not a kind. The first present point is the surviving chunk's
    // first owned point. `owned.start` is chunk-local, resolved to its track
    // index through the plan.
    let expected_first = plan.chunks[1]
        .sent
        .get(plan.chunks[1].owned.start)
        .expect("owned range is valid")
        .point;
    let first_present = result.points.first().expect("some points");
    assert_eq!(first_present.point, expected_first);
    assert!(
        first_present.follows_gap,
        "the failed chunk left no data before it"
    );
}

#[test]
fn point_count_mismatch_fails_the_chunk() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        uniform_response(sizes[0] - 1, SnapPointKind::Snapped, 1.0).expect("outcome"),
        uniform_response(sizes[1], SnapPointKind::Snapped, 1.0).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    assert!(result.partial);
    assert_eq!(
        reporter.warnings(),
        vec![SnapWarning::PointCountMismatch {
            chunk_index: 0,
            sent: sizes[0],
            received: sizes[0] - 1,
        }]
    );
}

#[test]
fn confidence_takes_the_minimum_and_changeset_mismatch_warns() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let make = |count: usize, confidence: f64, changeset: u64| {
        let matched: Vec<Value> = (0..count)
            .map(|_| json!({ "lat": 55.0, "lon": 12.0, "type": "matched", "distance_from_trace_point": 1.0 }))
            .collect();
        let response: TraceAttributesResponse = serde_json::from_value(json!({
            "matched_points": matched,
            "confidence_score": confidence,
            "osm_changeset": changeset,
        }))
        .expect("synthetic response");
        ChunkOutcome::Success(response)
    };
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(
        &plan,
        auto_params(),
        &[make(sizes[0], 0.9, 100), make(sizes[1], 0.4, 200)],
        &reporter,
    );

    assert_eq!(result.confidence_score, Some(0.4));
    assert_eq!(result.osm_changeset, Some(100), "first version wins");
    assert_eq!(
        reporter.warnings(),
        vec![SnapWarning::OsmChangesetMismatch {
            first: 100,
            later: 200,
        }]
    );
}

#[test]
fn server_warnings_are_passed_through() {
    let plan = request_plan::plan(&points(10));
    let response: TraceAttributesResponse = serde_json::from_value(json!({
        "matched_points": (0..10).map(|_| json!({ "lat": 55.0, "lon": 12.0, "type": "matched" })).collect::<Vec<_>>(),
        "warnings": [{ "message": "synthetic deprecation" }],
    }))
    .expect("synthetic response");
    let reporter = SnapWarningReporter::default();
    stitch::stitch(
        &plan,
        auto_params(),
        &[ChunkOutcome::Success(response)],
        &reporter,
    );

    let warnings = reporter.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        warnings.first(),
        Some(SnapWarning::Server { chunk_index: 0, .. })
    ));
}

#[test]
fn edge_references_survive_cross_chunk_concatenation() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        uniform_response(sizes[0], SnapPointKind::Snapped, 1.0).expect("outcome"),
        uniform_response(sizes[1], SnapPointKind::Snapped, 1.0).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    // Each chunk contributed one edge. Points of the second chunk must
    // reference the second edge, not the first.
    assert_eq!(result.edges.len(), 2);
    let last = result.points.last().expect("points exist");
    assert_eq!(last.edge, Some(1));
    let first = result.points.first().expect("points exist");
    assert_eq!(first.edge, Some(0));
}

#[test]
#[should_panic(expected = "one outcome per planned chunk")]
fn outcome_count_mismatch_is_a_bug() {
    let plan = two_chunk_plan();
    let reporter = SnapWarningReporter::default();
    stitch::stitch(&plan, auto_params(), &[], &reporter);
}

/// A success response carrying real geometry: `count` matched points all on
/// one edge whose shape is a 4-position encoded polyline.
fn shaped_response(count: usize) -> Result<ChunkOutcome, String> {
    let line: geo_types::LineString<f64> =
        vec![(12.0, 55.0), (12.0, 55.001), (12.0, 55.002), (12.0, 55.003)].into();
    let shape = polyline::encode_coordinates(line, SHAPE_POLYLINE_PRECISION)
        .map_err(|err| err.to_string())?;
    let matched_points: Vec<Value> = (0..count)
        .map(|_| {
            json!({
                "lat": 55.0, "lon": 12.0, "type": "matched",
                "edge_index": 0,
                "distance_from_trace_point": 1.0,
            })
        })
        .collect();
    let response: TraceAttributesResponse = serde_json::from_value(json!({
        "shape": shape,
        "matched_points": matched_points,
        "edges": [{ "begin_shape_index": 0, "end_shape_index": 3 }],
    }))
    .map_err(|err| err.to_string())?;
    Ok(ChunkOutcome::Success(response))
}

/// A response whose shape string is garbage, so geometry assembly errors
/// while the per-point data stays valid.
fn garbage_shape_response(count: usize) -> Result<ChunkOutcome, String> {
    let matched_points: Vec<Value> = (0..count)
        .map(|_| json!({ "lat": 55.0, "lon": 12.0, "type": "matched", "edge_index": 0, "distance_from_trace_point": 1.0 }))
        .collect();
    let response: TraceAttributesResponse = serde_json::from_value(json!({
        "shape": "!!!not-a-polyline\u{7f}",
        "matched_points": matched_points,
        "edges": [{ "begin_shape_index": 0, "end_shape_index": 3 }],
    }))
    .map_err(|err| err.to_string())?;
    Ok(ChunkOutcome::Success(response))
}

#[test]
fn continuous_chunks_join_into_one_segment() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        shaped_response(sizes[0]).expect("outcome"),
        shaped_response(sizes[1]).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    assert!(reporter.is_empty());
    // Both boundary points are snappable, so the cut mid-road is bridged:
    // one segment carrying both chunks' 4 positions.
    assert_eq!(result.segments.len(), 1);
    let segment = result.segments.first().expect("one segment");
    assert_eq!(segment.positions.len(), 8);

    // The joined segment's edge spans came along: the second chunk's span
    // is shifted onto the merged vertex sequence and references the
    // result-global (rebased) edge, exactly like the per-point references.
    let last_span = segment.edge_spans.last().expect("spans joined");
    assert_eq!(last_span.end, segment.positions.len());
    assert!(last_span.start >= 4, "second chunk's span was offset");
    assert_eq!(last_span.edge, result.edges.len() - 1);
    assert!(
        result.edges.get(last_span.edge).is_some(),
        "span references a result-global edge"
    );
}

#[test]
fn off_network_boundary_keeps_segments_split() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        shaped_response(sizes[0]).expect("outcome"),
        ChunkOutcome::OffNetwork,
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    // Chunk 0 contributes its segment, chunk 1 contributes none.
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments.first().map(|s| s.positions.len()), Some(4));
}

#[test]
fn geometry_error_reports_warning_and_never_welds_across() {
    let plan = request_plan::plan(&points(2 * CHUNK_POINTS));
    assert_eq!(plan.chunks.len(), 3, "precondition");
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        shaped_response(sizes[0]).expect("outcome"),
        garbage_shape_response(sizes[1]).expect("outcome"),
        shaped_response(sizes[2]).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    // The middle chunk's geometry failed: one Geometry warning, its points
    // still present and kinded, and the third chunk must NOT weld onto the
    // first chunk's (non-adjacent) segment.
    assert_eq!(result.points.len(), plan.sent_point_count());
    let warnings = reporter.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(matches!(
        warnings.first(),
        Some(SnapWarning::Geometry { chunk_index: 1, .. })
    ));
    assert_eq!(result.segments.len(), 2, "no weld across the failed chunk");
    assert!(!result.partial, "geometry failure is not a data gap");
}

/// The chunks either side of a dropped ghost run match two disconnected
/// drives, so their geometry stays split even though both boundaries are
/// snappable - the snapped track shows the break instead of a road through
/// the stretch the receiver never measured.
#[test]
fn chunks_across_a_ghost_gap_keep_their_geometry_split() {
    let plan = request_plan::plan(&support::points_with_ghosts_at(20, &[10, 11, 12]));
    assert_eq!(plan.chunks.len(), 2, "precondition");
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        shaped_response(sizes[0]).expect("outcome"),
        shaped_response(sizes[1]).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    assert!(reporter.is_empty());
    assert_eq!(result.segments.len(), 2);
    for segment in &result.segments {
        assert_eq!(segment.positions.len(), 4);
    }
    // The per-point series breaks at the same place: only the first point
    // of each stretch follows a gap.
    let gap_starts: Vec<usize> = result
        .points
        .iter()
        .filter(|point| point.follows_gap)
        .map(|point| point.point.as_usize())
        .collect();
    assert_eq!(gap_starts, vec![0, 13]);
}

/// An off-network chunk opening a stretch marks its first point as
/// following the gap too: the break is a property of the plan, not of what
/// the server returned.
#[test]
fn an_off_network_chunk_after_a_ghost_gap_follows_the_gap() {
    let plan = request_plan::plan(&support::points_with_ghosts_at(20, &[10, 11, 12]));
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        uniform_response(sizes[0], SnapPointKind::Snapped, 1.0).expect("outcome"),
        ChunkOutcome::OffNetwork,
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    let gap_starts: Vec<usize> = result
        .points
        .iter()
        .filter(|point| point.follows_gap)
        .map(|point| point.point.as_usize())
        .collect();
    assert_eq!(gap_starts, vec![0, 13]);
}

#[test]
fn shapeless_previous_chunk_loses_no_geometry() {
    let plan = two_chunk_plan();
    let sizes = chunk_sizes(&plan);
    let outcomes = [
        // A successful chunk with matched points but no shape at all:
        // contributes zero segments, and must not arm joining.
        uniform_response(sizes[0], SnapPointKind::Snapped, 1.0).expect("outcome"),
        shaped_response(sizes[1]).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, auto_params(), &outcomes, &reporter);

    // The second chunk's leading segment survives intact: nothing to join
    // onto must never consume (and drop) it.
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments.first().map(|s| s.positions.len()), Some(4));
}
