//! Where the map draws dead-reckoned track stretches and event markers stamped
//! inside them.
//!
//! The builder draws dead-reckoned fixes between measured fixes, and the map
//! dashes edges connecting them. An event marker inside dead-reckoning holds
//! coordinates the recorder interpolated over the dead-reckoned ones, 222 m
//! north of the line the receiver measured.

use std::ops::Range;
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_map::test_util::{self, MapScene};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::fixtures::FixKind;
use gt_types::{EventMarker, FileSource, Latitude, LoadedFile, Longitude, NavPoint, mercator};

fn time_of(index: usize) -> DateTime<Utc> {
    test_util::epoch() + Duration::seconds(index as i64 * SECONDS_BETWEEN_FIXES)
}

fn longitude_of(index: usize) -> Longitude {
    Longitude::new(FIRST_LONGITUDE_DEGREES + index as f64 * LONGITUDE_STEP_DEGREES)
}

fn dead_reckoned_latitude() -> Latitude {
    Latitude::new(MEASURED_LATITUDE_DEGREES + DEAD_RECKONED_OFFSET_DEGREES)
}

/// A fix the receiver measured: a course of 90°, and the twelve satellites in
/// fix that anchor the stretch it dead-reckoned.
fn measured_fix(index: usize) -> NavPoint {
    gt_types::fixtures::nav_point(
        time_of(index),
        Latitude::new(MEASURED_LATITUDE_DEGREES),
        longitude_of(index),
        FixKind::Measured,
    )
}

/// A fix the receiver dead-reckoned: no heading, no satellite report, and
/// coordinates north of the line it measured.
fn dead_reckoned_fix(index: usize) -> NavPoint {
    gt_types::fixtures::nav_point(
        time_of(index),
        dead_reckoned_latitude(),
        longitude_of(index),
        FixKind::GhostWithoutHeading,
    )
}

fn dead_reckoned_fix_with_heading(index: usize) -> NavPoint {
    gt_types::fixtures::nav_point(
        time_of(index),
        dead_reckoned_latitude(),
        longitude_of(index),
        FixKind::GhostWithoutSatellitesInFix,
    )
}

fn a_recording_with_dead_reckoned_fixes_with_headings() -> Vec<LoadedFile> {
    let points: Vec<NavPoint> = (0..FIX_COUNT)
        .map(|index| match DEAD_RECKONED_FIXES.contains(&index) {
            true => dead_reckoned_fix_with_heading(index),
            false => measured_fix(index),
        })
        .collect();
    vec![gt_track_builder::build_loaded_file(
        "heading_ghost_stretch.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("heading_ghost_stretch.gtd")),
        FileMeta::default(),
        vec![],
    )]
}

fn a_recording_with_an_event_marker_among_dead_reckoned_fixes() -> Vec<LoadedFile> {
    let points: Vec<NavPoint> = (0..FIX_COUNT)
        .map(|index| match DEAD_RECKONED_FIXES.contains(&index) {
            true => dead_reckoned_fix(index),
            false => measured_fix(index),
        })
        .collect();
    let marker = EventMarker::new(
        time_of(MARKER_FIX_INDEX),
        "power/boot".to_owned(),
        None,
        dead_reckoned_latitude(),
        longitude_of(MARKER_FIX_INDEX),
    );
    vec![gt_track_builder::build_loaded_file(
        "ghost_stretch.gtd".to_owned(),
        &points,
        &[],
        vec![marker],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ghost_stretch.gtd")),
        FileMeta::default(),
        vec![],
    )]
}

#[test]
fn snapshot_an_event_marker_among_dead_reckoned_fixes_is_drawn_on_the_dashed_track() {
    let files = a_recording_with_an_event_marker_among_dead_reckoned_fixes();
    let marker = files
        .first()
        .and_then(|file| file.tracks.first())
        .and_then(|track| track.event_markers.first())
        .expect("the recording has the event marker");
    assert_eq!(
        Some(marker.resolved_position.merc()),
        test_util::drawn_positions(&files)
            .get(MARKER_FIX_INDEX)
            .copied(),
        "the marker is drawn where the map draws the fix it is stamped at"
    );
    assert_ne!(
        marker.resolved_position.merc(),
        mercator::normalize(marker.lat, marker.lon),
        "the recorder wrote the marker off the line the receiver measured"
    );

    let mut map = MapScene::of(files).render();
    map.snapshot("event_marker_among_dead_reckoned_fixes");
}

#[test]
fn snapshot_hidden_ghost_fixes_leave_a_gap_in_the_trackline() {
    let files = a_recording_with_an_event_marker_among_dead_reckoned_fixes();
    let mut map = MapScene::of(files)
        .draw_state(|state| {
            state
                .display_mask
                .set_visible(gt_ui_types::DisplayCategory::GhostFixes, false);
        })
        .render();
    map.snapshot("ghost_fixes_hidden_leave_a_gap");
}

#[test]
fn snapshot_solo_ghost_fixes_draws_only_dead_reckoned_ink() {
    let files = a_recording_with_an_event_marker_among_dead_reckoned_fixes();
    let mut map = MapScene::of(files)
        .draw_state(|state| {
            state
                .display_mask
                .solo(gt_ui_types::DisplayCategory::GhostFixes);
        })
        .render();
    map.snapshot("ghost_fixes_soloed");
}

#[test]
fn snapshot_dead_reckoned_fixes_with_headings_draw_dashed_red() {
    let files = a_recording_with_dead_reckoned_fixes_with_headings();
    let mut map = MapScene::of(files).render();
    map.snapshot("dead_reckoned_fixes_with_headings_draw_dashed_red");
}

#[test]
fn snapshot_hidden_ghost_fixes_with_headings_leave_a_gap() {
    let files = a_recording_with_dead_reckoned_fixes_with_headings();
    let mut map = MapScene::of(files)
        .draw_state(|state| {
            state
                .display_mask
                .set_visible(gt_ui_types::DisplayCategory::GhostFixes, false);
        })
        .render();
    map.snapshot("ghost_fixes_with_headings_hidden_leave_a_gap");
}

/// Fixes of the recording, one every [`SECONDS_BETWEEN_FIXES`].
const FIX_COUNT: usize = 21;

const SECONDS_BETWEEN_FIXES: i64 = 10;

/// The fixes the receiver dead-reckoned, between the measured ones at each
/// end.
const DEAD_RECKONED_FIXES: Range<usize> = 6..15;

/// The fix whose time the event marker is stamped at, in the middle of the
/// dead-reckoned stretch.
const MARKER_FIX_INDEX: usize = 10;

/// The latitude the receiver measured every fix at.
const MEASURED_LATITUDE_DEGREES: f64 = 55.676;

const FIRST_LONGITUDE_DEGREES: f64 = 12.560;

/// Longitude between consecutive fixes, about 31 m at this latitude. The
/// twenty steps fill four fifths of the viewport once the map frames the
/// recording.
const LONGITUDE_STEP_DEGREES: f64 = 0.000_5;

/// How far north of the measured line the receiver's dead reckoning wrote its
/// coordinates, about 222 m.
const DEAD_RECKONED_OFFSET_DEGREES: f64 = 0.002;
