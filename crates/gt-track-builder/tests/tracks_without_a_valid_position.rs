//! A recording in which no fix has a coordinate in range: it segments into
//! tracks by time the way any recording does, and every track it builds has no
//! geometry.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig, segment};
use gt_types::coordinates::{Latitude, Longitude, RecordedLatitude, RecordedLongitude};
use gt_types::markers::EventMarker;
use gt_types::nav_point::{NavPoint, ResolvedPosition};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{FileSource, LoadedFile, TrackGeometry};
use uom::si::angle::degree;
use uom::si::f64::Angle;

/// A fix the receiver stamped but wrote no usable latitude for.
fn fix_without_a_valid_position(seconds: i64) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(
            DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(seconds),
        ))
        .lat(RecordedLatitude::from_degrees(91.0))
        .lon(RecordedLongitude::from_degrees(12.0))
        .heading(Angle::new::<degree>(90.0))
        .build();
    NavPoint::new(tpv, None)
}

fn build(points: &[NavPoint], event_markers: Vec<EventMarker>) -> LoadedFile {
    segment::build_loaded_file(
        "no_position.gtd".to_owned(),
        points,
        &[],
        event_markers,
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("no_position.gtd")),
        FileMeta::default(),
        vec![],
    )
}

#[test]
fn two_groups_separated_in_time_build_two_tracks_that_have_no_geometry() {
    let points: Vec<NavPoint> = (0..3)
        .chain(3600..3604)
        .map(fix_without_a_valid_position)
        .collect();

    let file = build(&points, vec![]);

    let fix_counts: Vec<usize> = file.tracks.iter().map(|t| t.points.len()).collect();
    assert_eq!(fix_counts, vec![3, 4]);
    for track in &file.tracks {
        assert_eq!(track.geometry, TrackGeometry::NoValidPosition);
        assert!(track.placed_points().is_none());
        assert_eq!(track.metadata.invalid_position_count, track.points.len());
    }
}

/// An event marker on such a track keeps the coordinates the recording holds
/// for it: the track has no drawn fix to place the marker against.
#[test]
fn an_event_marker_on_a_track_with_no_geometry_keeps_its_recorded_coordinates() {
    const MARKER_LATITUDE_DEGREES: f64 = 55.0;
    const MARKER_LONGITUDE_DEGREES: f64 = 12.0;

    let points: Vec<NavPoint> = (0..3).map(fix_without_a_valid_position).collect();
    let marker = EventMarker::new(
        DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(1),
        "marker/note".to_owned(),
        None,
        Latitude::new(MARKER_LATITUDE_DEGREES),
        Longitude::new(MARKER_LONGITUDE_DEGREES),
    );

    let file = build(&points, vec![marker]);

    let track = file.tracks.first().expect("one track");
    let marker = track.event_markers.first().expect("the marker's track");
    assert_eq!(
        marker.resolved_position,
        ResolvedPosition::measured(
            Latitude::new(MARKER_LATITUDE_DEGREES),
            Longitude::new(MARKER_LONGITUDE_DEGREES),
        )
    );
}
