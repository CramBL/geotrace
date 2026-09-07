//! What a recording reports as its total distance: the sum over the tracks
//! that have a geometry, and no measurement at all when none of them has one.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig, segment};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::fixtures::{self, FixKind};
use gt_types::nav_point::NavPoint;
use gt_types::track::{FileSource, TotalDistance};
use uom::si::length::meter;

const LATITUDE_DEGREES: f64 = 55.0;
const FIRST_LONGITUDE_DEGREES: f64 = 12.0;
const LAST_LONGITUDE_DEGREES: f64 = 12.002;

/// A millimetre over a track 128 m long, covering the projection and the
/// haversine round trip.
const METERS_TOLERANCE: f64 = 0.001;

fn fix(seconds: i64, longitude: Longitude, kind: FixKind) -> NavPoint {
    fixtures::nav_point(
        DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(seconds),
        Latitude::new(LATITUDE_DEGREES),
        longitude,
        kind,
    )
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
                Longitude::new(FIRST_LONGITUDE_DEGREES),
                FixKind::WithoutAPosition,
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
            Longitude::new(FIRST_LONGITUDE_DEGREES),
            FixKind::Measured,
        ),
        fix(
            10,
            Longitude::new(LAST_LONGITUDE_DEGREES),
            FixKind::Measured,
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
