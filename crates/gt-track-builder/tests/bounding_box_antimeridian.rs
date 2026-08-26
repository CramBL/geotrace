//! `TrackMetadata::bounding_box` and `merc_bounds` for a track that crosses
//! the antimeridian, and for the ordinary tracks that must stay unaffected.

use chrono::{TimeZone, Utc};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use rstest::rstest;
use uom::si::angle::degree;
use uom::si::f64::Angle;
use uom::si::length::meter;

use gt_track_builder::segment;

/// 1e-9° is about 0.1 mm.
const DEGREES_TOLERANCE: f64 = 1e-9;

/// One fix at second `t`.
fn fix(t: i64, lat: Latitude, lon: Longitude) -> NavPoint {
    let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
    let tpv = TimePositionVelocity::builder()
        .time(time)
        .lat(lat)
        .lon(lon)
        .heading(Angle::new::<degree>(90.0))
        .build();
    NavPoint::new(tpv, None)
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

fn assert_degrees_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < DEGREES_TOLERANCE,
        "expected {expected}°, got {actual}°"
    );
}

/// The box around a track crossing the antimeridian covers the 1.5° the track
/// actually spans, not the 359.4° between its raw extremes.
#[test]
fn bounding_box_across_the_antimeridian_covers_the_span_the_track_flew() {
    let bounds = segment::compute_track_metadata(0, &antimeridian_track(), &[], &[]).bounding_box;

    assert_degrees_close(bounds.lon.start().as_degrees(), 179.0);
    assert_degrees_close(bounds.lon.span_degrees(), 1.5);
}

/// The side panel centres the map on the middle of `bounding_box` when a track
/// row is double-clicked, so that centre must land on the track.
///
/// Oracle: the great-circle distance from the centre to the nearest fix,
/// against the track's own diameter (166_792.62 m).
#[test]
fn bounding_box_center_across_the_antimeridian_lands_on_the_track() {
    let points = antimeridian_track();
    let meta = segment::compute_track_metadata(0, &points, &[], &[]);
    let (center_lat, center_lon) = meta.bounding_box.center();
    let nearest_m = points
        .iter()
        .map(|p| gt_geo_math::haversine_m(center_lat, center_lon, p.tpv.lat(), p.tpv.lon()))
        .fold(f64::INFINITY, f64::min);
    let diameter_m = meta.point_set_diameter_m.get::<meter>();

    assert!(
        nearest_m <= diameter_m,
        "the map centre ({}, {}) is {nearest_m} m from the nearest fix of a \
         track {diameter_m} m across",
        center_lat.as_degrees(),
        center_lon.as_degrees()
    );
}

/// The map culls tracks by `merc_bounds`, so a track 166.79 km across must
/// claim 1.5° of the world's width and wrap at its eastern edge.
///
/// Oracle: normalized Mercator x is `(lon + 180) / 360`.
#[test]
fn merc_bounds_across_the_antimeridian_wrap_at_the_world_edge() {
    let merc_bounds = segment::compute_track_metadata(0, &antimeridian_track(), &[], &[]).merc_bounds;

    assert!(merc_bounds.crosses_the_antimeridian());
    let width = (1.0 - merc_bounds.x_min) + merc_bounds.x_max;
    assert!(
        (width - 1.5 / 360.0).abs() < DEGREES_TOLERANCE,
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
    let bounds = segment::compute_track_metadata(0, &points, &[], &[]).bounding_box;

    assert_degrees_close(bounds.lon.start().as_degrees(), 12.0);
    assert_degrees_close(bounds.lon.end().as_degrees(), 12.5);
    assert_degrees_close(bounds.lat.south().as_degrees(), 54.9);
    assert_degrees_close(bounds.lat.north().as_degrees(), 55.2);
    for p in &points {
        assert!(
            bounds.contains(p.tpv.lat(), p.tpv.lon()),
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
    let bounds = segment::compute_track_metadata(0, &points, &[], &[]).bounding_box;

    assert_degrees_close(bounds.lon.start().as_degrees(), 151.2);
    assert_degrees_close(bounds.lon.span_degrees(), 0.0);
    assert_degrees_close(bounds.lat.south().as_degrees(), -33.9);
    assert_degrees_close(bounds.lat.north().as_degrees(), -33.9);
}

/// Mercator y grows southwards, so the northernmost latitude must become
/// `y_min`. Oracle: `mercator::normalize` on the two corners.
#[test]
fn merc_bounds_put_the_northern_edge_at_y_min() {
    let points = vec1::vec1![
        fix(0, Latitude::new(55.0), Longitude::new(12.0)),
        fix(60, Latitude::new(56.0), Longitude::new(13.0)),
    ];
    let merc_bounds = segment::compute_track_metadata(0, &points, &[], &[]).merc_bounds;
    let north_west = gt_types::mercator::normalize(Latitude::new(56.0), Longitude::new(12.0));
    let south_east = gt_types::mercator::normalize(Latitude::new(55.0), Longitude::new(13.0));

    assert!((merc_bounds.y_min - north_west.y).abs() < 1e-12, "y_min");
    assert!((merc_bounds.y_max - south_east.y).abs() < 1e-12, "y_max");
    assert!((merc_bounds.x_min - north_west.x).abs() < 1e-12, "x_min");
    assert!((merc_bounds.x_max - south_east.x).abs() < 1e-12, "x_max");
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
        let bounds = segment::compute_track_metadata(0, &points, &[], &[]).bounding_box;
        for p in &points {
            proptest::prop_assert!(bounds.contains(p.tpv.lat(), p.tpv.lon()));
        }
    }
}
