//! Shared fixture construction for the gt-snap integration test binaries.

// Each test binary compiles this module independently and uses a different
// subset, so "unused" here only means "unused by this binary".
#![allow(dead_code, reason = "shared across binaries with different needs")]

use chrono::{DateTime, Utc};

use gt_snap::request_plan::{self, RequestPlan};
use gt_test_utils::fixtures::{self, FixKind, NavPointSpec};
use gt_types::nav_point::NavPoint;

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
