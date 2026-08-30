//! A track's `bounding_box` and `merc_bounds` when it circles a pole.
//!
//! The box is the polar cap holding the track: every meridian, and latitudes
//! from the southernmost fix to the pole. No longitude arc frames such a
//! track, which reaches every meridian without crossing any of them twice.

mod support;

use chrono::{DateTime, Duration};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::mercator;
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use uom::si::angle::degree;
use uom::si::f64::Angle;

use support::measured_geometry;

/// 1e-9° is about 0.1 mm.
const DEGREES_TOLERANCE: f64 = 1e-9;

/// One fix at second `t`, in degrees.
fn fix(t: i64, lat: f64, lon: f64) -> NavPoint {
    let time = GpsTime::from_utc(DateTime::UNIX_EPOCH + Duration::seconds(t));
    let tpv = TimePositionVelocity::builder()
        .time(time)
        .lat(Latitude::new(lat))
        .lon(Longitude::new(lon))
        .heading(Angle::new::<degree>(90.0))
        .build();
    NavPoint::new(tpv, None)
}

/// Four fixes at 89.9° N, a quarter turn apart: a receiver carried around the
/// north pole. Its diameter is 22_239.02 m (0.2° over the pole).
fn circumpolar_track() -> vec1::Vec1<NavPoint> {
    vec1::vec1![
        fix(0, 89.9, 0.0),
        fix(60, 89.9, 90.0),
        fix(120, 89.9, 180.0),
        fix(180, 89.9, -90.0),
    ]
}

fn assert_degrees_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < DEGREES_TOLERANCE,
        "expected {expected}°, got {actual}°"
    );
}

#[test]
fn bounding_box_around_the_pole_holds_every_meridian_and_reaches_the_pole() {
    let bounds = measured_geometry(&circumpolar_track())
        .expect("every fix has a recorded position")
        .bounding_box;

    assert!(
        bounds.lon.is_full_circle(),
        "expected every meridian, got {}° from {}°",
        bounds.lon.span_degrees(),
        bounds.lon.start().as_degrees()
    );
    assert_degrees_close(bounds.lat.south().as_degrees(), 89.9);
    assert_degrees_close(bounds.lat.north().as_degrees(), 90.0);
}

/// The cap projects to the world's whole width, which the map culls tracks
/// against. Mercator y grows southwards, which puts the pole at `y_min`.
///
/// Oracle: `mercator::normalize` on the two corners.
#[test]
fn merc_bounds_around_the_pole_span_the_world_with_the_pole_at_y_min() {
    let merc_bounds = measured_geometry(&circumpolar_track())
        .expect("every fix has a recorded position")
        .merc_bounds;
    let pole = mercator::normalize(Latitude::new(90.0), Longitude::new(-180.0));
    let southern_edge = mercator::normalize(Latitude::new(89.9), Longitude::new(180.0));

    assert!(merc_bounds.x_min.abs() < DEGREES_TOLERANCE, "x_min");
    assert!((merc_bounds.x_max - 1.0).abs() < DEGREES_TOLERANCE, "x_max");
    assert!(
        (merc_bounds.y_min - pole.y).abs() < DEGREES_TOLERANCE,
        "y_min"
    );
    assert!(
        (merc_bounds.y_max - southern_edge.y).abs() < DEGREES_TOLERANCE,
        "y_max"
    );
}
