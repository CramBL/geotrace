//! Where the builder draws a ghost fix: the position it interpolates in time
//! between the measured fixes around it.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::mercator;
use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::FileSource;
use uom::si::angle::degree;
use uom::si::f64::Angle;

/// Every point here shares a latitude, so only the longitudes distinguish them.
const LATITUDE_DEGREES: f64 = 55.0;

const SATELLITES_IN_FIX: u32 = 12;

/// The interpolated longitudes are exact in degrees: the tolerance covers only
/// the round trip through the Mercator projection.
const LON_TOLERANCE_DEGREES: f64 = 1e-9;

/// The index every test here writes its ghost fix at.
const GHOST_INDEX: usize = 1;

fn gps_time(millis: i64) -> GpsTime {
    GpsTime::from_utc(DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(millis))
}

/// A measured fix: heading present and a full solution behind it.
#[expect(
    clippy::expect_used,
    reason = "Test data generation with hardcoded values"
)]
fn measured_fix(millis: i64, lon_degrees: f64) -> NavPoint {
    let time = gps_time(millis);
    let tpv = TimePositionVelocity::builder()
        .time(time)
        .lat(Latitude::new(LATITUDE_DEGREES))
        .lon(Longitude::new(lon_degrees))
        .heading(Angle::new::<degree>(90.0))
        .build();
    let satellites = (1..=SATELLITES_IN_FIX)
        .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, true))
        .collect();
    NavPoint::new(tpv, Some(Satellites::new(Some(time), None, satellites)))
        .expect("coordinates in range")
}

/// An epoch the receiver dead-reckoned: no heading and no satellite report, so
/// the builder redraws it between its neighbours.
#[expect(
    clippy::expect_used,
    reason = "Test data generation with hardcoded values"
)]
fn ghost_fix(millis: i64, lon_degrees: f64) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(gps_time(millis))
        .lat(Latitude::new(LATITUDE_DEGREES))
        .lon(Longitude::new(lon_degrees))
        .build();
    NavPoint::new(tpv, None).expect("coordinates in range")
}

/// Longitude in degrees where the map draws the ghost fix of `points`.
fn drawn_ghost_lon_degrees(points: &[NavPoint]) -> Option<f64> {
    let file = gt_track_builder::build_loaded_file(
        "ghosts.gtd".to_owned(),
        points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ghosts.gtd")),
        FileMeta::default(),
        vec![],
    );
    let ghost = file
        .tracks
        .first()
        .and_then(|track| track.points.get(GHOST_INDEX))?;
    Some(mercator::denormalize(ghost.merc()).1)
}

/// The ghost fix sits at t = 0.5 s between measured fixes at t = 0.0 s (lon 0)
/// and t = 1.0 s (lon 1), so it belongs at lon 0.5. A fraction built from whole
/// seconds collapses to 0 and snaps it onto the preceding fix.
#[test]
fn ghost_fix_between_sub_second_fixes_is_placed_halfway() {
    let points = vec![
        measured_fix(0, 0.0),
        ghost_fix(500, 9.0),
        measured_fix(1000, 1.0),
    ];

    let lon = drawn_ghost_lon_degrees(&points).expect("the ghost fix is in the file's only track");

    assert!(
        (lon - 0.5).abs() < LON_TOLERANCE_DEGREES,
        "ghost fix drawn at lon {lon}, expected 0.5"
    );
}

/// Two measured fixes at one instant span no time, so there is no fraction to
/// place the ghost fix at and it keeps the position it was recorded with.
#[test]
fn ghost_fix_between_fixes_at_one_instant_keeps_its_recorded_position() {
    let points = vec![
        measured_fix(1000, 0.0),
        ghost_fix(1000, 9.0),
        measured_fix(1000, 1.0),
    ];

    let lon = drawn_ghost_lon_degrees(&points).expect("the ghost fix is in the file's only track");

    assert!(
        (lon - 9.0).abs() < LON_TOLERANCE_DEGREES,
        "ghost fix drawn at lon {lon}, expected the recorded 9.0"
    );
}

/// The measured fixes are 0.2 deg apart across the date line, so the great
/// circle between them holds every position at |lon| >= 179.9.
#[test]
fn ghost_fix_between_fixes_across_the_antimeridian_stays_between_them() {
    let points = vec![
        measured_fix(0, 179.9),
        ghost_fix(10_000, 179.95),
        measured_fix(20_000, -179.9),
    ];

    let lon = drawn_ghost_lon_degrees(&points).expect("the ghost fix is in the file's only track");

    assert!(
        lon.abs() >= 179.9,
        "ghost fix drawn at lon {lon}, expected it between 179.9 and -179.9 across the date line"
    );
}
