//! What the query window's map buttons frame: the camera box a reveal flies
//! to, against the recordings the global filter keeps.

mod support;

use chrono::Duration;
use gt_filter::GlobalFilter;
use gt_types::TrackRef;
use gt_ui_types::{DrawLayer, MatchRevealTarget, QueryMatches, TrackRanges};
use support::{
    CENTER_LON, Frame, HeadlessMap, WALKING_STEP_DEGREES, a_recording_of, track0, window_ending_at,
};

/// How far apart two longitudes may read and still be the same camera, well
/// under the 0.001° a fix moves.
const LONGITUDE_TOLERANCE_DEGREES: f64 = 1e-9;

/// One completed run whose single draw layer covers `points` of `track`.
fn a_run_matching(track: TrackRef, points: std::ops::Range<usize>) -> QueryMatches {
    QueryMatches {
        draws: vec![DrawLayer {
            color: 0,
            ranges: TrackRanges::from_iter([(track, vec![points])]),
        }],
        run: 1,
        ..QueryMatches::default()
    }
}

/// The query window's map buttons frame what the run drew. A match whose
/// points the time window hides is drawn nowhere. The camera stays on the
/// recording.
#[rstest::rstest]
#[case::the_whole_run(MatchRevealTarget::WholeRun)]
#[case::one_match(MatchRevealTarget::OneMatch { track: track0(), points: 20..30 })]
fn revealing_matches_does_not_frame_the_points_the_time_window_hides(
    #[case] target: MatchRevealTarget,
) {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let matches = a_run_matching(track0(), 20..30);
    // Every matched fix is outside the window: the window keeps the first ten
    // fixes, from before the run was made.
    let filter = window_ending_at(9);
    let mut map = HeadlessMap::new(&files, filter);

    map.draw(&Frame {
        query_matches: Some(&matches),
        ..Frame::default()
    });
    map.draw(&Frame {
        query_matches: Some(&matches),
        reveal: Some(target),
        ..Frame::default()
    });

    let framed = map.framed().expect("the map has drawn two frames");
    let first_matched_lon = CENTER_LON + 20.0 * WALKING_STEP_DEGREES;
    assert!(
        framed.lon_max < first_matched_lon,
        "the reveal framed up to {}° E, past the fixes the time window hides",
        framed.lon_max
    );
}

/// The filter rejects a recording for reasons other than the time window, here
/// `min_duration`. A reveal of matches on such a recording frames nothing, and
/// the camera stays where it was.
#[rstest::rstest]
#[case::the_whole_run(MatchRevealTarget::WholeRun)]
#[case::one_match(MatchRevealTarget::OneMatch { track: track0(), points: 20..30 })]
fn revealing_matches_of_a_recording_under_the_minimum_duration_leaves_the_camera_where_it_was(
    #[case] target: MatchRevealTarget,
) {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let matches = a_run_matching(track0(), 20..30);
    // `min_duration` filters the whole track out, although every one of its
    // fixes is inside the time window: the recording runs for 29 minutes.
    let filter = GlobalFilter {
        min_duration: Some(Duration::hours(5)),
        ..GlobalFilter::default()
    };
    let mut map = HeadlessMap::new(&files, filter);

    map.draw(&Frame {
        query_matches: Some(&matches),
        ..Frame::default()
    });
    let before = map.framed().expect("the map has drawn a frame");
    map.draw(&Frame {
        query_matches: Some(&matches),
        reveal: Some(target),
        ..Frame::default()
    });
    let after = map.framed().expect("the map has drawn two frames");

    assert!(
        (after.lon_min - before.lon_min).abs() < LONGITUDE_TOLERANCE_DEGREES
            && (after.lon_max - before.lon_max).abs() < LONGITUDE_TOLERANCE_DEGREES,
        "the reveal framed {}° E to {}° E, off the recording the filter rejects",
        after.lon_min,
        after.lon_max
    );
}
