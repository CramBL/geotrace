//! What the geometry in `TrackMetadata` measures for a track holding a ghost
//! fix: the polyline the map draws, and not the coordinates the receiver wrote
//! for an epoch it never measured.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{FileSource, TrackMetadata};
use uom::si::angle::degree;
use uom::si::f64::Angle;
use uom::si::length::{kilometer, meter};

/// Every fix of the track shares this latitude.
const LATITUDE_DEGREES: f64 = 55.0;

const FIRST_LON_DEGREES: f64 = 12.0;
const LAST_LON_DEGREES: f64 = 12.002;

const SATELLITES_IN_FIX: u32 = 12;

/// 1e-9° is about 0.1 mm.
const DEGREES_TOLERANCE: f64 = 1e-9;

/// A millimetre over a track 128 m long: the interpolated ghost fix lands on
/// the great circle between the measured fixes, so only the projection and the
/// haversine round trip cost anything.
const METERS_TOLERANCE: f64 = 0.001;

fn gps_time(secs: i64) -> GpsTime {
    GpsTime::from_utc(DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(secs))
}

/// A measured fix: heading present and a full solution behind it.
fn measured_fix(secs: i64, lon_degrees: f64) -> NavPoint {
    let time = gps_time(secs);
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
}

/// An epoch the receiver dead-reckoned and wrote at the null island: no heading
/// and no satellite report, so the builder redraws it between its neighbours.
fn ghost_fix_at_the_null_island(secs: i64) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(gps_time(secs))
        .lat(Latitude::new(0.0))
        .lon(Longitude::new(0.0))
        .build();
    NavPoint::new(tpv, None)
}

/// Metadata of a track of two measured fixes 0.002° of longitude apart, with a
/// dead-reckoned epoch halfway between them in time.
fn metadata_of_a_track_with_a_ghost_fix() -> TrackMetadata {
    let points = vec![
        measured_fix(0, FIRST_LON_DEGREES),
        ghost_fix_at_the_null_island(10),
        measured_fix(20, LAST_LON_DEGREES),
    ];
    let file = gt_track_builder::build_loaded_file(
        "ghost.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ghost.gtd")),
        FileMeta::default(),
        vec![],
    );
    file.tracks
        .first()
        .map(|track| track.metadata)
        .expect("the fixes form one track")
}

/// Distance between the two measured fixes, which is the length of the arc the
/// builder places the ghost fix on.
fn measured_fix_separation_m() -> f64 {
    gt_geo_math::haversine_m(
        Latitude::new(LATITUDE_DEGREES),
        Longitude::new(FIRST_LON_DEGREES),
        Latitude::new(LATITUDE_DEGREES),
        Longitude::new(LAST_LON_DEGREES),
    )
}

/// The ghost fix is drawn on the great circle between the measured fixes, so
/// the two legs of the drawn polyline sum to the arc between them. Measured
/// where the receiver wrote the ghost fix, the same track is 12 425 km long.
#[test]
fn track_distance_matches_the_polyline_the_map_draws() {
    let distance_m = metadata_of_a_track_with_a_ghost_fix()
        .distance_km
        .get::<kilometer>()
        * 1_000.0;

    let expected_m = measured_fix_separation_m();
    assert!(
        (distance_m - expected_m).abs() < METERS_TOLERANCE,
        "track length reported as {distance_m} m, the drawn polyline is {expected_m} m"
    );
}

/// The ghost fix sits between the measured fixes, so the widest pair of the
/// drawn track is the measured pair.
#[test]
fn point_set_diameter_spans_the_drawn_track() {
    let diameter_m = metadata_of_a_track_with_a_ghost_fix()
        .point_set_diameter_m
        .get::<meter>();

    let expected_m = measured_fix_separation_m();
    assert!(
        (diameter_m - expected_m).abs() < METERS_TOLERANCE,
        "track diameter reported as {diameter_m} m, the drawn track is {expected_m} m across"
    );
}

/// The box drives "zoom to track" and viewport culling, so it covers the
/// 0.002° the drawn track spans and never reaches down to the null island.
#[test]
fn bounding_box_covers_where_the_track_is_drawn() {
    let bounds = metadata_of_a_track_with_a_ghost_fix().bounding_box;

    assert!(
        (bounds.lat.south().as_degrees() - LATITUDE_DEGREES).abs() < DEGREES_TOLERANCE,
        "the box reaches south to {}°, the drawn track stays at {LATITUDE_DEGREES}°",
        bounds.lat.south().as_degrees()
    );
    assert!(
        (bounds.lon.span_degrees() - (LAST_LON_DEGREES - FIRST_LON_DEGREES)).abs()
            < DEGREES_TOLERANCE,
        "the box spans {}° of longitude, the drawn track spans {}°",
        bounds.lon.span_degrees(),
        LAST_LON_DEGREES - FIRST_LON_DEGREES
    );
}
