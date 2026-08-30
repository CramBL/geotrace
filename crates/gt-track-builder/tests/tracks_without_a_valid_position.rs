//! A recording in which no fix has a coordinate in range: it segments into
//! tracks by time the way any recording does, and every track it builds has no
//! geometry.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig, segment};
use gt_types::coordinates::{RecordedLatitude, RecordedLongitude};
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{FileSource, TrackGeometry};
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

#[test]
fn two_groups_separated_in_time_build_two_tracks_that_have_no_geometry() {
    let points: Vec<NavPoint> = (0..3)
        .chain(3600..3604)
        .map(fix_without_a_valid_position)
        .collect();

    let file = segment::build_loaded_file(
        "no_position.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("no_position.gtd")),
        FileMeta::default(),
        vec![],
    );

    let fix_counts: Vec<usize> = file.tracks.iter().map(|t| t.points.len()).collect();
    assert_eq!(fix_counts, vec![3, 4]);
    for track in &file.tracks {
        assert_eq!(track.geometry, TrackGeometry::NoValidPosition);
        assert!(track.placed_points().is_none());
        assert_eq!(track.metadata.invalid_position_count, track.points.len());
    }
}
