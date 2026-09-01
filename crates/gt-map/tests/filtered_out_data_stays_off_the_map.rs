//! What the global filter keeps off the map: the snapped-track layer of a
//! recording the filter rejects, the snapped vertices and error whiskers its
//! time window hides, and the fixes a fit frames.

mod support;

use std::sync::Arc;

use chrono::Duration;
use gt_filter::GlobalFilter;
use gt_types::mercator::MercPoint;
use gt_types::{LoadedFile, LoadedTrack, PointIdx};
use gt_ui_types::{SnappedSegment, SnappedTrackGeometry, SnappedTracks, WhiskerAnchor};
use support::{
    CENTER_LON, Frame, HeadlessMap, WALKING_STEP_DEGREES, a_recording_of, epoch, track0,
    window_ending_at,
};

/// Longitude between consecutive fixes of a track recorded in one spot, about
/// 6 cm. A fit over such a track reaches the maximum zoom, which is where the
/// error whiskers draw.
const STANDING_STEP_DEGREES: f64 = 0.000_001;

/// Ten metres north in normalized Mercator at this latitude, offsetting the
/// snapped geometry so it is its own ink beside the recorded track.
const SNAPPED_OFFSET_MERC_Y: f64 = -1.5e-6;

/// Where the map draws the fixes of track 0, in normalized Mercator.
fn drawn_positions(files: &[LoadedFile]) -> Vec<MercPoint> {
    files
        .first()
        .and_then(|file| file.tracks.first())
        .and_then(LoadedTrack::placed_points)
        .map(|placed| placed.iter().map(|point| point.merc()).collect())
        .unwrap_or_default()
}

/// Snapped road geometry for track 0: one polyline beside the recorded fixes
/// in `fixes`, one vertex per fix.
fn snapped_polyline_over(files: &[LoadedFile], fixes: std::ops::Range<usize>) -> SnappedTracks {
    let points: Vec<MercPoint> = drawn_positions(files)
        .get(fixes.clone())
        .unwrap_or_default()
        .iter()
        .map(|point| MercPoint {
            x: point.x,
            y: point.y + SNAPPED_OFFSET_MERC_Y,
        })
        .collect();
    let mut snapped = SnappedTracks::default();
    snapped.insert(
        track0(),
        Arc::new(SnappedTrackGeometry {
            segments: vec![SnappedSegment {
                points,
                recorded_points: fixes.map(PointIdx::new).collect(),
                edge_spans: Vec::new(),
            }],
            edges: Vec::new(),
            whiskers: Vec::new(),
        }),
    );
    snapped
}

/// Error whiskers for track 0: one per recorded fix in `fixes`, reaching from
/// the fix to a snapped position north of it.
fn whiskers_over(files: &[LoadedFile], fixes: std::ops::Range<usize>) -> SnappedTracks {
    let whiskers: Vec<WhiskerAnchor> = drawn_positions(files)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| fixes.contains(index))
        .map(|(index, point)| WhiskerAnchor {
            point: PointIdx::new(index),
            snapped: MercPoint {
                x: point.x,
                y: point.y + SNAPPED_OFFSET_MERC_Y,
            },
        })
        .collect();
    let mut snapped = SnappedTracks::default();
    snapped.insert(
        track0(),
        Arc::new(SnappedTrackGeometry {
            segments: Vec::new(),
            edges: Vec::new(),
            whiskers,
        }),
    );
    snapped
}

/// Shapes one frame paints over the recordings in `files`, under `filter`,
/// with `snapped` handed to the map.
fn shapes_with(
    files: &[LoadedFile],
    filter: GlobalFilter,
    snapped: Option<&SnappedTracks>,
) -> usize {
    HeadlessMap::new(files, filter).draw(&Frame {
        snapped_tracks: snapped,
        ..Frame::default()
    })
}

/// A recording the filter rejects puts nothing on the map, and the road
/// geometry it was snapped to is part of that nothing: it is the same
/// recording, drawn beside itself.
#[rstest::rstest]
#[case::the_time_window_is_disjoint_from_the_recording(GlobalFilter {
    time_start: Some(epoch() + Duration::hours(5)),
    ..GlobalFilter::default()
})]
#[case::the_recording_is_shorter_than_the_minimum_duration(GlobalFilter {
    min_duration: Some(Duration::hours(5)),
    ..GlobalFilter::default()
})]
fn a_snapped_track_of_a_filtered_out_recording_is_not_drawn(#[case] filter: GlobalFilter) {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let snapped = snapped_polyline_over(&files, 0..30);

    assert_eq!(
        shapes_with(&files, filter, Some(&snapped)),
        shapes_with(&files, filter, None),
        "the snapped track of a filtered-out recording put ink on the map"
    );
}

/// The time window ends the recorded track at the last fix it keeps, and the
/// snapped track beside it ends there too: the map draws the same ink for the
/// whole snapped geometry as for the stretch the window keeps.
#[test]
fn a_snapped_track_is_not_drawn_past_the_end_of_the_time_window() {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let filter = window_ending_at(14);

    assert_eq!(
        shapes_with(&files, filter, Some(&snapped_polyline_over(&files, 0..30))),
        shapes_with(&files, filter, Some(&snapped_polyline_over(&files, 0..15))),
        "the snapped track was drawn past the end of the time window"
    );
}

/// An error whisker reaches from a recorded fix to where that fix was
/// snapped.
#[test]
fn an_error_whisker_of_a_fix_outside_the_time_window_is_not_drawn() {
    let files = a_recording_of(30, STANDING_STEP_DEGREES);
    let filter = window_ending_at(14);

    assert_eq!(
        shapes_with(&files, filter, Some(&whiskers_over(&files, 0..30))),
        shapes_with(&files, filter, Some(&whiskers_over(&files, 0..15))),
        "a whisker was drawn at a fix the time window hides"
    );
}

/// This guards the oracle the cases above rely on: a recording the filter
/// keeps still draws its snapped track.
#[test]
fn a_snapped_track_of_a_kept_recording_is_drawn() {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let snapped = snapped_polyline_over(&files, 0..30);

    assert!(
        shapes_with(&files, GlobalFilter::default(), Some(&snapped))
            > shapes_with(&files, GlobalFilter::default(), None),
        "the snapped track of a kept recording put no ink on the map"
    );
}

/// A fit frames the fixes the time window keeps: fix 4 is the last of them,
/// and fix 10 lies well outside it.
#[test]
fn a_fit_frames_the_fixes_inside_the_time_window() {
    let files = a_recording_of(30, WALKING_STEP_DEGREES);
    let mut map = HeadlessMap::new(&files, window_ending_at(4));

    map.draw(&Frame::default());

    let framed = map.framed().expect("the map has drawn a frame");
    assert!(
        framed.lon_max > CENTER_LON + 4.0 * WALKING_STEP_DEGREES
            && framed.lon_max < CENTER_LON + 10.0 * WALKING_STEP_DEGREES,
        "the fit framed up to {}° E, and the window ends at fix 4",
        framed.lon_max
    );
}
