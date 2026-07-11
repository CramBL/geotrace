//! Validate stitching of chunk outcomes into a `SnapResult`.

use std::fs;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use gt_snap::fixtures_dir;
use gt_snap::request_plan::{self, CHUNK_POINTS, RequestPlan};
use gt_snap::snapped_track::SHAPE_POLYLINE_PRECISION;
use gt_snap::stitch::{self, ChunkOutcome, SnapWarning, SnapWarningReporter};
use gt_snap::wire::{Costing, SnapPointKind, TraceAttributesResponse};
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::{Latitude, Longitude};

/// 2026-01-01T12:00:00Z (valid constant; the epoch fallback would fail the
/// time-based assertions loudly).
fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default()
}

/// `count` 1 Hz points walking north.
fn points(count: usize) -> Vec<NavPoint> {
    (0..count)
        .map(|i| {
            let time = base_time() + chrono::Duration::seconds(i as i64);
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(time))
                .lat(Latitude::new(55.0 + i as f64 * 1e-5))
                .lon(Longitude::new(12.0))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect()
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
        Costing::Auto,
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
    // Chunk 0 reports every point snapped; chunk 1 reports every point
    // interpolated. Overlap points must take the owning chunk's answer.
    let outcomes = [
        uniform_response(sizes[0], SnapPointKind::Snapped, 1.0).expect("outcome"),
        uniform_response(sizes[1], SnapPointKind::Interpolated, 2.0).expect("outcome"),
    ];
    let reporter = SnapWarningReporter::default();
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

    assert_eq!(result.points.len(), plan.sent_point_count());
    // Every point appears exactly once, ascending by track index.
    let indices: Vec<usize> = result.points.iter().map(|p| p.point.as_usize()).collect();
    let mut sorted = indices.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(indices, sorted);
    // Ownership split: chunk 0 owns its interior plus the first half of the
    // overlap; the counts must reflect the owned ranges, not chunk sizes.
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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

    assert!(reporter.is_empty(), "off-network is not a failure");
    assert!(!result.partial);
    assert_eq!(result.points.len(), plan.sent_point_count());
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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

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
    // first owned point (owned.start is chunk-local; resolve it to its
    // track index through the plan).
    let expected_first = plan.chunks[1]
        .sent
        .get(plan.chunks[1].owned.start)
        .expect("owned range is valid")
        .point;
    let first_present = result.points.first().expect("some points");
    assert_eq!(first_present.point, expected_first);
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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

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
        Costing::Auto,
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
        Costing::Auto,
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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

    // Each chunk contributed one edge; points of the second chunk must
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
    stitch::stitch(&plan, Costing::Auto, &[], &reporter);
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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

    assert!(reporter.is_empty());
    // Both boundary points are snappable, so the cut mid-road is bridged:
    // one segment carrying both chunks' 4 positions.
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments.first().map(|s| s.positions.len()), Some(8));
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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

    // Chunk 0 contributes its segment; chunk 1 contributes none. Nothing
    // was joined and nothing lost.
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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

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
    let result = stitch::stitch(&plan, Costing::Auto, &outcomes, &reporter);

    // The second chunk's leading segment survives intact: nothing to join
    // onto must never consume (and drop) it.
    assert_eq!(result.segments.len(), 1);
    assert_eq!(result.segments.first().map(|s| s.positions.len()), Some(4));
}
