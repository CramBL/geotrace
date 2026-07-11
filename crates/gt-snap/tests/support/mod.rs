//! Shared fixture construction for the gt-snap integration test binaries.

// Each test binary compiles this module independently and uses a different
// subset, so "unused" here only means "unused by this binary".
#![allow(dead_code, reason = "shared across binaries with different needs")]

use chrono::{DateTime, Utc};

use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::{Latitude, Longitude};

/// 2026-01-01T12:00:00Z, matching the capture harness's fixed base time.
/// (The epoch fallback is unreachable for this valid constant and would
/// fail every time-based assertion loudly if it weren't.)
pub fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default()
}

/// `count` points spaced `step_ms` apart starting at [`base_time`], walking
/// north from 55°N 12°E, each with the eph produced by `eph(i)`.
pub fn points_with(
    count: usize,
    step_ms: i64,
    eph: impl Fn(usize) -> Option<f32>,
) -> Vec<NavPoint> {
    (0..count)
        .map(|i| {
            let time = base_time() + chrono::Duration::milliseconds(i as i64 * step_ms);
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(time))
                .lat(Latitude::new(55.0 + i as f64 * 1e-5))
                .lon(Longitude::new(12.0))
                .maybe_eph_m(eph(i))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect()
}

/// `count` 1 Hz points without eph - the common case.
pub fn points(count: usize) -> Vec<NavPoint> {
    points_with(count, 1000, |_| None)
}
