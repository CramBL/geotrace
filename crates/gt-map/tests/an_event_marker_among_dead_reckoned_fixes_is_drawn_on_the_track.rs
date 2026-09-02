//! Where the map draws an event marker stamped inside a stretch of fixes the
//! receiver dead-reckoned.
//!
//! The builder draws such a fix between the fixes with a satellite in fix
//! around it, and the map dashes the edges into it. The marker holds the
//! coordinates the recorder interpolated over the dead-reckoned ones, 222 m
//! north of the line the receiver measured.

mod support;

use std::ops::Range;
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{
    EventMarker, FileSource, GpsTime, Latitude, LoadedFile, Longitude, NavPoint,
    TimePositionVelocity,
};
use uom::si::angle::degree;
use uom::si::f64::Angle;

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

/// The course the receiver reports along the measured stretches.
const EAST_HEADING_DEGREES: f64 = 90.0;

/// Satellites the receiver reports in fix under a full solution.
const SATELLITES_IN_FIX: u32 = 12;

fn time_of(index: usize) -> DateTime<Utc> {
    support::epoch() + Duration::seconds(index as i64 * SECONDS_BETWEEN_FIXES)
}

fn longitude_of(index: usize) -> Longitude {
    Longitude::new(FIRST_LONGITUDE_DEGREES + index as f64 * LONGITUDE_STEP_DEGREES)
}

fn dead_reckoned_latitude() -> Latitude {
    Latitude::new(MEASURED_LATITUDE_DEGREES + DEAD_RECKONED_OFFSET_DEGREES)
}

fn full_solution(index: usize) -> Satellites {
    let satellites = (0..SATELLITES_IN_FIX)
        .map(|index| Satellite::new(Constellation::Gps, index + 1, None, None, None, true))
        .collect();
    Satellites::new(Some(GpsTime::from_utc(time_of(index))), None, satellites)
}

/// A fix the receiver measured: a heading, and the satellites in fix that
/// anchor the stretch it dead-reckoned.
fn measured_fix(index: usize) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time_of(index)))
        .lat(Latitude::new(MEASURED_LATITUDE_DEGREES))
        .lon(longitude_of(index))
        .heading(Angle::new::<degree>(EAST_HEADING_DEGREES))
        .build();
    NavPoint::new(tpv, Some(full_solution(index)))
}

/// A fix the receiver dead-reckoned: no heading, and coordinates north of the
/// line it measured.
fn dead_reckoned_fix(index: usize) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time_of(index)))
        .lat(dead_reckoned_latitude())
        .lon(longitude_of(index))
        .build();
    NavPoint::new(tpv, None)
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
    let mut harness = support::rendered_map(files);
    harness.snapshot_loose("event_marker_among_dead_reckoned_fixes");
}
