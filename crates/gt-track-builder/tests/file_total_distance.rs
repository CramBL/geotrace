//! What a recording reports as its total distance: the sum over the tracks
//! that have a geometry, and no measurement at all when none of them has one.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig, segment};
use gt_types::coordinates::{Latitude, Longitude, RecordedLatitude, RecordedLongitude};
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{FileSource, TotalDistance};
use uom::si::angle::degree;
use uom::si::f64::Angle;
use uom::si::length::meter;

const LATITUDE_DEGREES: f64 = 55.0;
const FIRST_LONGITUDE_DEGREES: f64 = 12.0;
const LAST_LONGITUDE_DEGREES: f64 = 12.002;

/// A millimetre over a track 128 m long, covering the projection and the
/// haversine round trip.
const METERS_TOLERANCE: f64 = 0.001;

fn fix(seconds: i64, latitude: RecordedLatitude, longitude: RecordedLongitude) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(
            DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(seconds),
        ))
        .lat(latitude)
        .lon(longitude)
        .heading(Angle::new::<degree>(90.0))
        .build();
    NavPoint::new(tpv, None)
}

fn total_distance_of(points: &[NavPoint]) -> TotalDistance {
    segment::build_loaded_file(
        "recording.gtd".to_owned(),
        points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("recording.gtd")),
        FileMeta::default(),
        vec![],
    )
    .metadata
    .total_distance
}

#[test]
fn a_recording_whose_only_track_has_no_geometry_measures_no_distance() {
    let points: Vec<NavPoint> = (0..3)
        .map(|seconds| {
            fix(
                seconds,
                RecordedLatitude::from_degrees(91.0),
                Longitude::new(FIRST_LONGITUDE_DEGREES).into(),
            )
        })
        .collect();

    assert_eq!(total_distance_of(&points), TotalDistance::NoMeasuredTrack);
}

#[test]
fn a_recording_of_one_measured_track_reports_the_length_of_its_polyline() {
    let points = vec![
        fix(
            0,
            Latitude::new(LATITUDE_DEGREES).into(),
            Longitude::new(FIRST_LONGITUDE_DEGREES).into(),
        ),
        fix(
            10,
            Latitude::new(LATITUDE_DEGREES).into(),
            Longitude::new(LAST_LONGITUDE_DEGREES).into(),
        ),
    ];

    let measured = total_distance_of(&points)
        .measured()
        .expect("the track is measured")
        .get::<meter>();

    let expected_m = gt_geo_math::haversine_m(
        Latitude::new(LATITUDE_DEGREES),
        Longitude::new(FIRST_LONGITUDE_DEGREES),
        Latitude::new(LATITUDE_DEGREES),
        Longitude::new(LAST_LONGITUDE_DEGREES),
    );
    assert!(
        (measured - expected_m).abs() < METERS_TOLERANCE,
        "total distance reported as {measured} m, the drawn polyline is {expected_m} m"
    );
}
