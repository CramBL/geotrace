//! Distance between consecutive fixes across the antimeridian and at the
//! poles, and the point-set diameter derived from the same positions.
//!
//! The oracles are hand-computed great-circle lengths on the sphere `geo`'s
//! [`geo::Haversine`] measures on (the GRS80 mean radius, 6_371_008.8 m), plus
//! the invariant that a point set's diameter is never shorter than the longest
//! segment between two of its consecutive points.

use chrono::DateTime;
use gt_geo_math::{
    haversine_km, haversine_m, path_distance_km, point_set_diameter_m, segment_distances_m,
};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::time_types::GpsTime;
use gt_types::{NavPoint, TimePositionVelocity};

/// Mean earth radius [`geo::Haversine`] measures on, in metres.
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Length of one degree of great circle on that sphere, in metres:
/// `6_371_008.8 * π / 180`.
const DEGREE_M: f64 = 111_195.080_233_532_92;

/// Metres of slack allowed against a hand-computed great-circle length.
const TOLERANCE_M: f64 = 0.1;

fn point(lat: Latitude, lon: Longitude) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(DateTime::UNIX_EPOCH))
        .lat(lat)
        .lon(lon)
        .build();
    NavPoint::new(tpv, None)
}

/// A track running east over the antimeridian: 179.0° E, then 179.0° W, then
/// 179.9° W, all on the equator. Its two extreme fixes are 2° apart, which is
/// both the diameter of the set and the length of its first segment.
#[test]
fn diameter_across_the_antimeridian_is_at_least_the_longest_segment() {
    let points = [
        point(Latitude::new(0.0), Longitude::new(179.0)),
        point(Latitude::new(0.0), Longitude::new(-179.0)),
        point(Latitude::new(0.0), Longitude::new(-179.9)),
    ];
    let diameter_m = point_set_diameter_m(&points);
    let longest_segment_m = segment_distances_m(&points).fold(0.0_f64, f64::max);

    assert!(
        diameter_m >= longest_segment_m - TOLERANCE_M,
        "diameter {diameter_m} m is shorter than the longest segment {longest_segment_m} m"
    );
}

/// A circumpolar track at 89.9° N visiting 0°, 180° and 90° W. The 0°/180°
/// pair passes over the pole 0.2° apart and is the diameter of the set.
#[test]
fn diameter_of_a_circumpolar_track_spans_the_pole() {
    let points = [
        point(Latitude::new(89.9), Longitude::new(0.0)),
        point(Latitude::new(89.9), Longitude::new(180.0)),
        point(Latitude::new(89.9), Longitude::new(-90.0)),
    ];
    let diameter_m = point_set_diameter_m(&points);
    let expected_m = 0.2 * DEGREE_M;

    assert!(
        (diameter_m - expected_m).abs() < TOLERANCE_M,
        "expected {expected_m} m over the pole, got {diameter_m} m"
    );
}

/// 179.9° E to 179.9° W is 0.2° of longitude on the equator, not 359.8°:
/// 22_239.02 m, not 40_008_988 m.
#[test]
fn haversine_m_across_the_antimeridian_takes_the_short_way() {
    let d = haversine_m(
        Latitude::new(0.0),
        Longitude::new(179.9),
        Latitude::new(0.0),
        Longitude::new(-179.9),
    );
    let expected_m = 0.2 * DEGREE_M;

    assert!(
        (d - expected_m).abs() < TOLERANCE_M,
        "expected {expected_m} m, got {d} m"
    );
}

/// 180° E and 180° W are the same meridian.
#[test]
fn haversine_m_at_both_signs_of_180_is_zero() {
    let d = haversine_m(
        Latitude::new(12.5),
        Longitude::new(180.0),
        Latitude::new(12.5),
        Longitude::new(-180.0),
    );

    assert!(d < TOLERANCE_M, "expected 0 m, got {d} m");
}

/// Walking 179.0° E to 180.0° to 179.0° W sums to the direct 2° span: the
/// antimeridian inside the track adds nothing.
#[test]
fn path_distance_km_across_the_antimeridian_equals_the_direct_span() {
    let points = [
        point(Latitude::new(0.0), Longitude::new(179.0)),
        point(Latitude::new(0.0), Longitude::new(180.0)),
        point(Latitude::new(0.0), Longitude::new(-179.0)),
    ];
    let direct_km = haversine_km(
        Latitude::new(0.0),
        Longitude::new(179.0),
        Latitude::new(0.0),
        Longitude::new(-179.0),
    );
    let walked_km = path_distance_km(&points);

    assert!(
        (walked_km - direct_km).abs() < TOLERANCE_M / 1_000.0,
        "expected {direct_km} km, got {walked_km} km"
    );
}

/// Two fixes either side of the north pole, 0.2° apart over the top. An
/// equirectangular reading of the same numbers would call this 180.2°.
#[test]
fn haversine_m_over_the_north_pole_is_the_meridian_gap() {
    let d = haversine_m(
        Latitude::new(89.9),
        Longitude::new(0.0),
        Latitude::new(89.9),
        Longitude::new(180.0),
    );
    let expected_m = 0.2 * DEGREE_M;

    assert!(
        (d - expected_m).abs() < TOLERANCE_M,
        "expected {expected_m} m, got {d} m"
    );
}

/// At the pole itself longitude carries no distance.
#[test]
fn haversine_m_at_the_north_pole_ignores_longitude() {
    let d = haversine_m(
        Latitude::new(90.0),
        Longitude::new(0.0),
        Latitude::new(90.0),
        Longitude::new(-171.25),
    );

    assert!(d < TOLERANCE_M, "expected 0 m, got {d} m");
}

/// Pole to pole is half the circumference, `π * 6_371_008.8`.
#[test]
fn haversine_m_pole_to_pole_is_half_the_circumference() {
    let d = haversine_m(
        Latitude::new(90.0),
        Longitude::new(0.0),
        Latitude::new(-90.0),
        Longitude::new(0.0),
    );
    let expected_m = std::f64::consts::PI * EARTH_RADIUS_M;

    assert!(
        (d - expected_m).abs() < TOLERANCE_M,
        "expected {expected_m} m, got {d} m"
    );
}

/// A stationary receiver at the pole repeating the same fix: every segment is
/// exactly zero and none of them is NaN.
#[test]
fn repeated_polar_fixes_give_finite_zero_segments() {
    let points = [
        point(Latitude::new(90.0), Longitude::new(0.0)),
        point(Latitude::new(90.0), Longitude::new(0.0)),
        point(Latitude::new(90.0), Longitude::new(0.0)),
    ];

    for (i, m) in segment_distances_m(&points).enumerate() {
        assert!(m.is_finite() && m < TOLERANCE_M, "segment {i} is {m} m");
    }
}

/// Every track's diameter is at least as long as its longest single segment,
/// on a shape that stays clear of the antimeridian and the poles.
#[test]
fn diameter_is_at_least_the_longest_segment_on_a_local_track() {
    let points = [
        point(Latitude::new(55.60), Longitude::new(12.90)),
        point(Latitude::new(55.62), Longitude::new(12.94)),
        point(Latitude::new(55.61), Longitude::new(12.99)),
        point(Latitude::new(55.58), Longitude::new(12.95)),
    ];
    let diameter_m = point_set_diameter_m(&points);
    let longest_segment_m = segment_distances_m(&points).fold(0.0_f64, f64::max);

    assert!(
        diameter_m >= longest_segment_m - TOLERANCE_M,
        "diameter {diameter_m} m is shorter than the longest segment {longest_segment_m} m"
    );
}
