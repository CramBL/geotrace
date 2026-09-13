//! Fixtures for the gt-snap tests: nav points from the capture harness's base
//! time, their request plan, and readers for the captured responses under
//! `tests/captures/`.
//!
//! The integration test binaries reach it as `gt_snap::test_util`, through the
//! `test-util` feature gt-snap's dev-dependency on itself enables.

use std::fs;

use chrono::{DateTime, Utc};
use geo_types::LineString;
use gt_test_utils::fixtures::{self, FixKind, NavPointSpec};
use gt_types::nav_point::NavPoint;

use crate::request_plan::{self, RequestPlan};
use crate::snapped_track::SHAPE_POLYLINE_PRECISION;
use crate::wire::TraceAttributesResponse;

/// 2026-01-01T12:00:00Z, matching the capture harness's fixed base time.
/// (The epoch fallback is unreachable for this valid constant and would
/// fail every time-based assertion loudly if it weren't.)
pub fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default()
}

/// `count` points from [`base_time`] spaced `step_ms` apart, each built from
/// the spec produced by `spec(i)`.
pub fn points_with_spec(
    count: usize,
    step_ms: i64,
    spec: impl Fn(usize) -> NavPointSpec,
) -> Vec<NavPoint> {
    fixtures::nav_points_from_specs(base_time(), count, step_ms, spec)
}

/// `count` real fixes spaced `step_ms` apart, each with the eph produced by
/// `eph(i)`.
pub fn points_with(
    count: usize,
    step_ms: i64,
    eph: impl Fn(usize) -> Option<f32>,
) -> Vec<NavPoint> {
    points_with_spec(count, step_ms, |i| NavPointSpec {
        fix: FixKind::Measured,
        eph_m: eph(i),
    })
}

/// `count` 1 Hz real fixes without eph - the common case.
pub fn points(count: usize) -> Vec<NavPoint> {
    points_with(count, 1000, |_| None)
}

/// `count` 1 Hz points where the indices in `ghosts` are heading-less ghost
/// fixes and the rest are real.
pub fn points_with_ghosts_at(count: usize, ghosts: &[usize]) -> Vec<NavPoint> {
    points_with_spec(count, 1000, |i| {
        if ghosts.contains(&i) {
            NavPointSpec {
                fix: FixKind::GhostWithoutHeading,
                ..Default::default()
            }
        } else {
            NavPointSpec::default()
        }
    })
}

/// The request plan for `points` taken as a track of their own.
pub fn plan_of(points: &[NavPoint]) -> RequestPlan {
    let track = gt_test_utils::loaded_track_with_points(points.to_vec());
    request_plan::plan(track.placed_points().unwrap_or_default())
}

pub fn read_capture(name: &str) -> Result<String, String> {
    let path = crate::captures_dir().join(name);
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

pub fn captured_response(scenario: &str) -> Result<TraceAttributesResponse, String> {
    let body = read_capture(&format!("{scenario}.response.json"))?;
    serde_json::from_str(&body).map_err(|err| format!("{scenario}: {err}"))
}

/// Four positions about 110 m apart along the 12°E meridian, encoded at
/// [`SHAPE_POLYLINE_PRECISION`].
pub fn four_position_shape() -> Result<String, String> {
    let line: LineString<f64> =
        vec![(12.0, 55.0), (12.0, 55.001), (12.0, 55.002), (12.0, 55.003)].into();
    polyline::encode_coordinates(line, SHAPE_POLYLINE_PRECISION).map_err(|err| err.to_string())
}
