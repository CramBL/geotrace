//! Track building over fixes whose timestamps do not increase. Nothing sorts
//! them: the `.gtd` reader keeps file order, and a recorder using one of the
//! non-Rust SDKs can write a fix stamped before its predecessor.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig, TrackLayoutConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::markers::EventMarker;
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{FileSource, LoadedFile};
use uom::si::angle::degree;
use uom::si::f64::Angle;

/// A measured fix, distinguished only by its timestamp: every one of them
/// shares a position.
fn measured_fix_at(millis: i64) -> NavPoint {
    let time = DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(millis);
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time))
        .lat(Latitude::new(55.0))
        .lon(Longitude::new(12.0))
        .heading(Angle::new::<degree>(90.0))
        .build();
    NavPoint::new(tpv, None)
}

fn build(points: &[NavPoint], event_markers: Vec<EventMarker>) -> LoadedFile {
    gt_track_builder::build_loaded_file(
        "out_of_order.gtd".to_owned(),
        points,
        &[],
        event_markers,
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("out_of_order.gtd")),
        FileMeta::default(),
        vec![],
    )
}

/// An event marker stamped at the exact time of a fix in the track belongs to
/// that track. A time range taken from the first and last fix excludes the
/// middle fix here, which is the latest one.
#[test]
fn event_marker_at_an_out_of_order_fix_time_is_assigned_to_its_track() {
    const SECOND_FIX_MILLIS: i64 = 100_000;

    let points = vec![
        measured_fix_at(0),
        measured_fix_at(SECOND_FIX_MILLIS),
        measured_fix_at(50_000),
    ];
    let marker = EventMarker::new(
        DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(SECOND_FIX_MILLIS),
        "power/boot".to_owned(),
        None,
        Latitude::new(55.0),
        Longitude::new(12.0),
    );

    let file = build(&points, vec![marker]);

    let track = file.tracks.first().expect("one track");
    assert_eq!(
        track.event_markers.len(),
        1,
        "marker at the time of the track's own second fix was orphaned"
    );
}

/// Recorded time is a span of wall clock: it covers every fix and cannot be
/// negative, whatever order the fixes arrived in.
#[test]
fn recorded_time_spans_every_fix_when_one_steps_backwards() {
    let points = vec![measured_fix_at(1_000_000), measured_fix_at(990_000)];

    let file = build(&points, vec![]);

    assert_eq!(file.metadata.total_duration, Duration::seconds(10));
}

#[test]
fn a_backward_time_step_past_the_split_gap_starts_a_new_track() {
    const HOUR_MILLIS: i64 = 3_600_000;

    let points = vec![
        measured_fix_at(HOUR_MILLIS),
        measured_fix_at(HOUR_MILLIS + 1_000),
        measured_fix_at(0),
        measured_fix_at(1_000),
    ];

    let file = build(&points, vec![]);

    let spans: Vec<(i64, i64)> = file
        .tracks
        .iter()
        .map(|track| {
            (
                track.metadata.time_range.start.timestamp_millis(),
                track.metadata.time_range.end.timestamp_millis(),
            )
        })
        .collect();
    assert_eq!(spans, vec![(HOUR_MILLIS, HOUR_MILLIS + 1_000), (0, 1_000)]);
}

proptest::proptest! {
    /// The ranges are contiguous, non-empty, and cover every point exactly
    /// once, whatever order the timestamps arrive in: segmentation partitions
    /// the points.
    #[test]
    fn segment_tracks_partitions_every_point_exactly_once(
        millis in proptest::collection::vec(-1_000_000i64..1_000_000i64, 1..40usize)
    ) {
        let points: Vec<NavPoint> = millis.iter().copied().map(measured_fix_at).collect();

        let ranges = gt_track_builder::segment_tracks(&points, &TrackLayoutConfig::default());

        let mut cursor = 0;
        for range in &ranges {
            proptest::prop_assert_eq!(range.start, cursor);
            proptest::prop_assert!(range.end > range.start);
            cursor = range.end;
        }
        proptest::prop_assert_eq!(cursor, points.len());
    }
}
