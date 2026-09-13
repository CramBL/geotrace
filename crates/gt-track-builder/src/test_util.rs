//! Fixtures and assertions for the gt-track-builder tests.
//!
//! The crate's own tests reach it as `crate::test_util`. The integration test
//! binaries reach it as `gt_track_builder::test_util`, through the `test-util`
//! feature gt-track-builder's dev-dependency on itself enables.

use chrono::{DateTime, Duration, Utc};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::fixtures::{self, FixKind, SatelliteCounts};
use gt_types::satellites::Satellites;
use gt_types::time_types::GpsTime;
use gt_types::track::MeasuredTrackGeometry;
use gt_types::{NavPoint, PlacedPoints};
use uom::si::angle::degree;
use uom::si::f64::Angle;

use crate::segment::{self, FixPlacementRule};

/// The geometry of `points` taken as a track of their own, `None` when the
/// builder places none of them.
pub fn measured_geometry(points: &[NavPoint]) -> Option<MeasuredTrackGeometry> {
    segment::measure_track_geometry(points, FixPlacementRule::default())
        .measured()
        .cloned()
}

pub fn assert_degrees_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < DEGREES_TOLERANCE,
        "expected {expected}°, got {actual}°"
    );
}

/// Calls `read` with `points` taken as a track of their own, each fix beside
/// where the builder places it. `read` takes `None` for a track whose every
/// fix the builder leaves unplaced. The placement borrows the geometry, which
/// lives for the call alone.
pub fn with_placed_points_of<R>(
    points: &[NavPoint],
    read: impl FnOnce(Option<PlacedPoints<'_>>) -> R,
) -> R {
    let geometry = segment::measure_track_geometry(points, FixPlacementRule::default());
    read(
        geometry
            .measured()
            .and_then(|measured| PlacedPoints::new(points, &measured.resolved_positions)),
    )
}

/// A fix at 55°N 12°E `second` seconds past the Unix epoch, with a heading and
/// no satellite report.
pub fn fix_without_a_satellite_report(second: i64) -> NavPoint {
    fix_without_a_satellite_report_at(
        second,
        Latitude::new(FIXTURE_LATITUDE_DEGREES),
        Longitude::new(FIXTURE_LONGITUDE_DEGREES),
    )
}

/// [`fix_without_a_satellite_report`] at a position of the caller's choosing.
pub fn fix_without_a_satellite_report_at(second: i64, lat: Latitude, lon: Longitude) -> NavPoint {
    fixtures::nav_point_heading(
        time_at_second(second),
        lat,
        lon,
        Some(Angle::new::<degree>(EASTWARD_HEADING_DEGREES)),
        FixKind::GhostWithoutHeading,
    )
}

/// A fix at 55°N 12°E `second` seconds past the Unix epoch by the receiver's
/// clock, with a host clock `host_ahead` past the receiver's, no heading and
/// no satellite report.
pub fn fix_with_host_clock_ahead(second: i64, host_ahead: Duration) -> NavPoint {
    fixtures::nav_point_with_host_clock(
        time_at_second(second),
        host_ahead,
        Latitude::new(FIXTURE_LATITUDE_DEGREES),
        Longitude::new(FIXTURE_LONGITUDE_DEGREES),
        FixKind::GhostWithoutHeading,
    )
}

/// A fix at 55°N 12°E `second` seconds past the Unix epoch, reporting one GPS
/// satellite in the fix.
pub fn fix_with_a_satellite_in_fix(second: i64) -> NavPoint {
    fix_with_one_satellite(
        second,
        SatelliteCounts {
            in_fix: 1,
            in_view_only: 0,
        },
    )
}

/// A fix at 55°N 12°E `second` seconds past the Unix epoch, reporting one GPS
/// satellite in view and none in the fix.
pub fn fix_with_a_satellite_in_view_only(second: i64) -> NavPoint {
    fix_with_one_satellite(
        second,
        SatelliteCounts {
            in_fix: 0,
            in_view_only: 1,
        },
    )
}

/// A report of `in_fix` GPS satellites in the fix, and of one satellite in
/// view where `in_fix` is zero, so that a fix with nothing in its fix still
/// has a report.
pub fn satellite_report_of(in_fix: u32) -> Satellites {
    fixtures::satellite_report(
        None,
        SatelliteCounts {
            in_fix,
            in_view_only: u32::from(in_fix == 0),
        },
    )
}

pub fn time_at_second(second: i64) -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(second)
}

fn fix_with_one_satellite(second: i64, counts: SatelliteCounts) -> NavPoint {
    let time = time_at_second(second);
    fixtures::nav_point_with_report(
        time,
        Latitude::new(FIXTURE_LATITUDE_DEGREES),
        Longitude::new(FIXTURE_LONGITUDE_DEGREES),
        fixtures::satellite_report(Some(GpsTime::from_utc(time)), counts),
    )
}

/// 1e-9° is about 0.1 mm.
pub const DEGREES_TOLERANCE: f64 = 1e-9;

const FIXTURE_LATITUDE_DEGREES: f64 = 55.0;

const FIXTURE_LONGITUDE_DEGREES: f64 = 12.0;

const EASTWARD_HEADING_DEGREES: f64 = 90.0;
