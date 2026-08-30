//! Distance between consecutive fixes across the antimeridian and at the
//! poles, the point-set diameter derived from the same positions, and the
//! positions interpolated between them.
//!
//! The oracles are hand-computed great-circle lengths on the sphere `geo`'s
//! [`geo::Haversine`] measures on (the GRS80 mean radius, 6_371_008.8 m), plus
//! the invariant that a point set's diameter is never shorter than the longest
//! segment between two of its consecutive points.

use gt_geo_math::GreatCircleArc;
use gt_types::coordinates::{Latitude, Longitude};

/// Mean earth radius [`geo::Haversine`] measures on, in metres.
const EARTH_RADIUS_M: f64 = 6_371_008.8;

/// Length of one degree of great circle on that sphere, in metres:
/// `6_371_008.8 * π / 180`.
const DEGREE_M: f64 = 111_195.080_233_532_92;

/// Metres of slack allowed against a hand-computed great-circle length.
const TOLERANCE_M: f64 = 0.1;

/// A track running east over the antimeridian: 179.0° E, then 179.0° W, then
/// 179.9° W, all on the equator. Its two extreme fixes are 2° apart, which is
/// both the diameter of the set and the length of its first segment.
#[test]
fn diameter_across_the_antimeridian_is_at_least_the_longest_segment() {
    let positions = [
        (Latitude::new(0.0), Longitude::new(179.0)),
        (Latitude::new(0.0), Longitude::new(-179.0)),
        (Latitude::new(0.0), Longitude::new(-179.9)),
    ];
    let diameter_m = gt_geo_math::point_set_diameter_m(&positions);
    let longest_segment_m = gt_geo_math::segment_distances_m(&positions).fold(0.0_f64, f64::max);

    assert!(
        diameter_m >= longest_segment_m - TOLERANCE_M,
        "diameter {diameter_m} m is shorter than the longest segment {longest_segment_m} m"
    );
}

/// A circumpolar track at 89.9° N visiting 0°, 180° and 90° W. The 0°/180°
/// pair passes over the pole 0.2° apart and is the diameter of the set.
#[test]
fn diameter_of_a_circumpolar_track_spans_the_pole() {
    let positions = [
        (Latitude::new(89.9), Longitude::new(0.0)),
        (Latitude::new(89.9), Longitude::new(180.0)),
        (Latitude::new(89.9), Longitude::new(-90.0)),
    ];
    let diameter_m = gt_geo_math::point_set_diameter_m(&positions);
    let expected_m = 0.2 * DEGREE_M;

    assert!(
        (diameter_m - expected_m).abs() < TOLERANCE_M,
        "expected {expected_m} m over the pole, got {diameter_m} m"
    );
}

/// Three fixes evenly spaced around the equator: no rotation of the
/// longitude/latitude grid holds a set that wide, and every pair of them is
/// 120° apart.
#[test]
fn diameter_of_a_set_wider_than_a_hemisphere_spans_its_widest_pair() {
    let positions = [
        (Latitude::new(0.0), Longitude::new(0.0)),
        (Latitude::new(0.0), Longitude::new(120.0)),
        (Latitude::new(0.0), Longitude::new(-120.0)),
    ];
    let diameter_m = gt_geo_math::point_set_diameter_m(&positions);
    let expected_m = 120.0 * DEGREE_M;

    assert!(
        (diameter_m - expected_m).abs() < TOLERANCE_M,
        "expected {expected_m} m, got {diameter_m} m"
    );
}

/// Two antipodal fixes: their directions cancel exactly, so the set has no
/// mean direction to rotate onto. They are half a circumference apart.
#[test]
fn diameter_of_an_antipodal_pair_is_half_the_circumference() {
    let positions = [
        (Latitude::new(0.0), Longitude::new(0.0)),
        (Latitude::new(0.0), Longitude::new(180.0)),
    ];
    let diameter_m = gt_geo_math::point_set_diameter_m(&positions);
    let expected_m = std::f64::consts::PI * EARTH_RADIUS_M;

    assert!(
        (diameter_m - expected_m).abs() < TOLERANCE_M,
        "expected {expected_m} m, got {diameter_m} m"
    );
}

/// 179.9° E to 179.9° W is 0.2° of longitude on the equator, not 359.8°:
/// 22_239.02 m, not 40_008_988 m.
#[test]
fn haversine_m_across_the_antimeridian_takes_the_short_way() {
    let d = gt_geo_math::haversine_m(
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
    let d = gt_geo_math::haversine_m(
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
    let positions = [
        (Latitude::new(0.0), Longitude::new(179.0)),
        (Latitude::new(0.0), Longitude::new(180.0)),
        (Latitude::new(0.0), Longitude::new(-179.0)),
    ];
    let direct_km = gt_geo_math::haversine_km(
        Latitude::new(0.0),
        Longitude::new(179.0),
        Latitude::new(0.0),
        Longitude::new(-179.0),
    );
    let walked_km = gt_geo_math::path_distance_km(&positions);

    assert!(
        (walked_km - direct_km).abs() < TOLERANCE_M / 1_000.0,
        "expected {direct_km} km, got {walked_km} km"
    );
}

/// Two fixes either side of the north pole, 0.2° apart over the top. An
/// equirectangular reading of the same numbers would call this 180.2°.
#[test]
fn haversine_m_over_the_north_pole_is_the_meridian_gap() {
    let d = gt_geo_math::haversine_m(
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
    let d = gt_geo_math::haversine_m(
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
    let d = gt_geo_math::haversine_m(
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
    let positions = [
        (Latitude::new(90.0), Longitude::new(0.0)),
        (Latitude::new(90.0), Longitude::new(0.0)),
        (Latitude::new(90.0), Longitude::new(0.0)),
    ];

    for (i, m) in gt_geo_math::segment_distances_m(&positions).enumerate() {
        assert!(m.is_finite() && m < TOLERANCE_M, "segment {i} is {m} m");
    }
}

/// Every track's diameter is at least as long as its longest single segment,
/// on a shape that stays clear of the antimeridian and the poles.
#[test]
fn diameter_is_at_least_the_longest_segment_on_a_local_track() {
    let positions = [
        (Latitude::new(55.60), Longitude::new(12.90)),
        (Latitude::new(55.62), Longitude::new(12.94)),
        (Latitude::new(55.61), Longitude::new(12.99)),
        (Latitude::new(55.58), Longitude::new(12.95)),
    ];
    let diameter_m = gt_geo_math::point_set_diameter_m(&positions);
    let longest_segment_m = gt_geo_math::segment_distances_m(&positions).fold(0.0_f64, f64::max);

    assert!(
        diameter_m >= longest_segment_m - TOLERANCE_M,
        "diameter {diameter_m} m is shorter than the longest segment {longest_segment_m} m"
    );
}

/// Halfway from 179.9° E to 179.9° W on the equator lies on the antimeridian,
/// 0.1° from each end of the arc.
#[test]
fn great_circle_arc_across_the_antimeridian_takes_the_short_way() {
    let start = (Latitude::new(0.0), Longitude::new(179.9));
    let end = (Latitude::new(0.0), Longitude::new(-179.9));
    let (mid_lat, mid_lon) = GreatCircleArc { start, end }.position_at_ratio(0.5);

    let expected_m = 0.1 * DEGREE_M;
    let from_start_m = gt_geo_math::haversine_m(start.0, start.1, mid_lat, mid_lon);
    let from_end_m = gt_geo_math::haversine_m(end.0, end.1, mid_lat, mid_lon);

    assert!(
        (from_start_m - expected_m).abs() < TOLERANCE_M,
        "midpoint at {mid_lat:?}, {mid_lon:?} is {from_start_m} m from the start, expected {expected_m} m"
    );
    assert!(
        (from_end_m - expected_m).abs() < TOLERANCE_M,
        "midpoint at {mid_lat:?}, {mid_lon:?} is {from_end_m} m from the end, expected {expected_m} m"
    );
}

/// 89.9° N at 0° and at 180° lie either side of the north pole, so the great
/// circle between them runs over the pole and its midpoint is the pole itself.
#[test]
fn great_circle_arc_between_fixes_either_side_of_the_pole_runs_over_it() {
    let start = (Latitude::new(89.9), Longitude::new(0.0));
    let end = (Latitude::new(89.9), Longitude::new(180.0));
    let (mid_lat, mid_lon) = GreatCircleArc { start, end }.position_at_ratio(0.5);

    let from_pole_m =
        gt_geo_math::haversine_m(Latitude::new(90.0), Longitude::new(0.0), mid_lat, mid_lon);

    assert!(
        from_pole_m < TOLERANCE_M,
        "the midpoint at {mid_lat:?}, {mid_lon:?} is {from_pole_m} m from the pole"
    );
}
