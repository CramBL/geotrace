//! Shared fixture construction for the gt-track-builder integration test
//! binaries.

// Each test binary compiles this module independently and uses a different
// subset, so "unused" here only means "unused by this binary".
#![allow(dead_code, reason = "shared across binaries with different needs")]

use gt_track_builder::segment;
use gt_types::nav_point::NavPoint;
use gt_types::track::MeasuredTrackGeometry;

/// The geometry of `points` taken as a track of their own, `None` when the
/// builder places none of them.
pub fn measured_geometry(points: &[NavPoint]) -> Option<MeasuredTrackGeometry> {
    segment::measure_track_geometry(points, segment::FixPlacementRule::default())
        .measured()
        .cloned()
}

/// 1e-9° is about 0.1 mm.
pub const DEGREES_TOLERANCE: f64 = 1e-9;

pub fn assert_degrees_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < DEGREES_TOLERANCE,
        "expected {expected}°, got {actual}°"
    );
}
