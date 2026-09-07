//! A track's `bounding_box` and `merc_bounds`: over an ordinary track, over
//! one crossing the antimeridian, and over one circling a pole.
//!
//! The box around a track that circles a pole is the polar cap holding it:
//! every meridian, and latitudes from the southernmost fix to the pole. No
//! longitude arc frames such a track, which reaches every meridian without
//! crossing any of them twice.

mod support;

use chrono::{DateTime, Duration};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::fixtures::{self, FixKind};
use gt_types::mercator;
use gt_types::nav_point::NavPoint;
use rstest::rstest;
use uom::si::length::meter;

/// One fix at second `t`.
fn fix(t: i64, lat: Latitude, lon: Longitude) -> NavPoint {
    fixtures::nav_point(
        DateTime::UNIX_EPOCH + Duration::seconds(t),
        lat,
        lon,
        FixKind::Measured,
    )
}

/// An eastbound equatorial track stepping over the antimeridian:
/// 179.0° E, 179.5° E, 179.9° W, 179.5° W. It spans 1.5° of longitude,
/// 166.79 km at the equator.
fn antimeridian_track() -> vec1::Vec1<NavPoint> {
    vec1::vec1![
        fix(0, Latitude::new(0.0), Longitude::new(179.0)),
        fix(60, Latitude::new(0.0), Longitude::new(179.5)),
        fix(120, Latitude::new(0.0), Longitude::new(-179.9)),
        fix(180, Latitude::new(0.0), Longitude::new(-179.5)),
    ]
}

/// The box around a track crossing the antimeridian covers the 1.5° the track
/// actually spans, not the 359.4° between its raw extremes.
#[test]
fn bounding_box_across_the_antimeridian_covers_the_span_the_track_flew() {
    let bounds = support::measured_geometry(&antimeridian_track())
        .expect("every fix has a recorded position")
        .bounding_box;

    support::assert_degrees_close(bounds.lon.start().as_degrees(), 179.0);
    support::assert_degrees_close(bounds.lon.span_degrees(), 1.5);
}

/// The centre of the box must land on the track: the side panel centres the
/// map on it when a track row is double-clicked.
///
/// Oracle: the great-circle distance from the centre to the nearest fix,
/// against the track's own diameter (166_792.62 m).
#[test]
fn bounding_box_center_across_the_antimeridian_lands_on_the_track() {
    let points = antimeridian_track();
    let geometry = support::measured_geometry(&points).expect("every fix has a recorded position");
    let (center_lat, center_lon) = geometry.bounding_box.center();
    let nearest_m = geometry
        .resolved_positions
        .iter()
        .map(|resolved| {
            let (latitude, longitude) = resolved.coordinates();
            gt_geo_math::haversine_m(center_lat, center_lon, latitude, longitude)
        })
        .fold(f64::INFINITY, f64::min);
    let diameter_m = geometry.point_set_diameter_m.get::<meter>();

    assert!(
        nearest_m <= diameter_m,
        "the map centre ({}, {}) is {nearest_m} m from the nearest fix of a \
         track {diameter_m} m across",
        center_lat.as_degrees(),
        center_lon.as_degrees()
    );
}

/// A track 166.79 km across must claim 1.5° of the world's width and wrap at
/// its eastern edge: the map culls tracks by `merc_bounds`.
///
/// Oracle: normalized Mercator x is `(lon + 180) / 360`.
#[test]
fn merc_bounds_across_the_antimeridian_wrap_at_the_world_edge() {
    let merc_bounds = support::measured_geometry(&antimeridian_track())
        .expect("every fix has a recorded position")
        .merc_bounds;

    assert!(merc_bounds.crosses_the_antimeridian());
    let width = (1.0 - merc_bounds.x_min) + merc_bounds.x_max;
    assert!(
        (width - 1.5 / 360.0).abs() < support::DEGREES_TOLERANCE,
        "expected {} of the world's width, got {width} (merc x {} to {})",
        1.5 / 360.0,
        merc_bounds.x_min,
        merc_bounds.x_max
    );
}

/// A track away from the antimeridian gets the tight box, and every fix is
/// inside it.
#[test]
fn bounding_box_of_a_local_track_is_tight_and_holds_every_fix() {
    let points = vec1::vec1![
        fix(0, Latitude::new(55.0), Longitude::new(12.0)),
        fix(60, Latitude::new(55.2), Longitude::new(12.5)),
        fix(120, Latitude::new(54.9), Longitude::new(12.1)),
    ];
    let geometry = support::measured_geometry(&points).expect("every fix has a recorded position");
    let bounds = geometry.bounding_box;

    support::assert_degrees_close(bounds.lon.start().as_degrees(), 12.0);
    support::assert_degrees_close(bounds.lon.end().as_degrees(), 12.5);
    support::assert_degrees_close(bounds.lat.south().as_degrees(), 54.9);
    support::assert_degrees_close(bounds.lat.north().as_degrees(), 55.2);
    for resolved in &geometry.resolved_positions {
        let (latitude, longitude) = resolved.coordinates();
        assert!(
            bounds.contains(latitude, longitude),
            "a fix lies outside the box"
        );
    }
}

/// Fixes at a single position give a degenerate box on that position, not an
/// empty or inverted one.
#[rstest]
#[case::a_single_fix(1)]
#[case::repeated_fixes(3)]
fn bounding_box_of_fixes_at_one_position_is_degenerate(#[case] fix_count: i64) {
    let points: Vec<NavPoint> = (0..fix_count)
        .map(|t| fix(t, Latitude::new(-33.9), Longitude::new(151.2)))
        .collect();
    let points = vec1::Vec1::try_from_vec(points).expect("at least one fix");
    let bounds = support::measured_geometry(&points)
        .expect("every fix has a recorded position")
        .bounding_box;

    support::assert_degrees_close(bounds.lon.start().as_degrees(), 151.2);
    support::assert_degrees_close(bounds.lon.span_degrees(), 0.0);
    support::assert_degrees_close(bounds.lat.south().as_degrees(), -33.9);
    support::assert_degrees_close(bounds.lat.north().as_degrees(), -33.9);
}

/// Mercator y grows southwards, so the northernmost latitude must become
/// `y_min`. Oracle: `mercator::normalize` on the two corners.
#[test]
fn merc_bounds_put_the_northern_edge_at_y_min() {
    let points = vec1::vec1![
        fix(0, Latitude::new(55.0), Longitude::new(12.0)),
        fix(60, Latitude::new(56.0), Longitude::new(13.0)),
    ];
    let merc_bounds = support::measured_geometry(&points)
        .expect("every fix has a recorded position")
        .merc_bounds;
    let north_west = mercator::normalize(Latitude::new(56.0), Longitude::new(12.0));
    let south_east = mercator::normalize(Latitude::new(55.0), Longitude::new(13.0));

    assert!((merc_bounds.y_min - north_west.y).abs() < 1e-12, "y_min");
    assert!((merc_bounds.y_max - south_east.y).abs() < 1e-12, "y_max");
    assert!((merc_bounds.x_min - north_west.x).abs() < 1e-12, "x_min");
    assert!((merc_bounds.x_max - south_east.x).abs() < 1e-12, "x_max");
}

/// Four fixes at 89.9° N, a quarter turn apart: a receiver carried around the
/// north pole. Its diameter is 22_239.02 m (0.2° over the pole).
fn circumpolar_track() -> vec1::Vec1<NavPoint> {
    vec1::vec1![
        fix(0, Latitude::new(89.9), Longitude::new(0.0)),
        fix(60, Latitude::new(89.9), Longitude::new(90.0)),
        fix(120, Latitude::new(89.9), Longitude::new(180.0)),
        fix(180, Latitude::new(89.9), Longitude::new(-90.0)),
    ]
}

#[test]
fn bounding_box_around_the_pole_holds_every_meridian_and_reaches_the_pole() {
    let bounds = support::measured_geometry(&circumpolar_track())
        .expect("every fix has a recorded position")
        .bounding_box;

    assert!(
        bounds.lon.is_full_circle(),
        "expected every meridian, got {}° from {}°",
        bounds.lon.span_degrees(),
        bounds.lon.start().as_degrees()
    );
    support::assert_degrees_close(bounds.lat.south().as_degrees(), 89.9);
    support::assert_degrees_close(bounds.lat.north().as_degrees(), 90.0);
}

/// The cap projects to the world's whole width, which the map culls tracks
/// against. Both of its Mercator edges are the northern edge of the world:
/// the cap lies past `mercator::MAX_LATITUDE_DEGREES` from edge to edge.
#[test]
fn merc_bounds_around_the_pole_span_the_world_and_lie_on_its_northern_edge() {
    let merc_bounds = support::measured_geometry(&circumpolar_track())
        .expect("every fix has a recorded position")
        .merc_bounds;
    let northern_edge = mercator::normalize(
        Latitude::new(mercator::MAX_LATITUDE_DEGREES),
        Longitude::new(0.0),
    );

    assert!(
        merc_bounds.x_min.abs() < support::DEGREES_TOLERANCE,
        "x_min"
    );
    assert!(
        (merc_bounds.x_max - 1.0).abs() < support::DEGREES_TOLERANCE,
        "x_max"
    );
    assert!(
        (merc_bounds.y_min - northern_edge.y).abs() < support::DEGREES_TOLERANCE,
        "y_min {}",
        merc_bounds.y_min
    );
    assert!(
        (merc_bounds.y_max - northern_edge.y).abs() < support::DEGREES_TOLERANCE,
        "y_max {}",
        merc_bounds.y_max
    );
}

proptest::proptest! {
    /// The box always holds every fix of a track that stays clear of the
    /// antimeridian - the property the renderers' O(1) culling relies on.
    #[test]
    fn bounding_box_holds_every_fix_of_a_local_track(
        lats in proptest::collection::vec(-85.0_f64..85.0, 1..20),
        lons in proptest::collection::vec(-179.0_f64..179.0, 1..20),
    ) {
        let n = lats.len().min(lons.len());
        let points: Vec<NavPoint> = (0..n)
            .filter_map(|i| {
                Some(fix(
                    i as i64,
                    Latitude::new(*lats.get(i)?),
                    Longitude::new(*lons.get(i)?),
                ))
            })
            .collect();
        let points = vec1::Vec1::try_from_vec(points).expect("at least one fix");
        let geometry = support::measured_geometry(&points).expect("every fix has a recorded position");
        for resolved in &geometry.resolved_positions {
            let (latitude, longitude) = resolved.coordinates();
            proptest::prop_assert!(geometry.bounding_box.contains(latitude, longitude));
        }
    }
}
