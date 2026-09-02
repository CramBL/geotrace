//! Where the map draws an event marker along a stretch of fixes the builder
//! re-placed.
//!
//! The builder places an event marker over the positions it draws the fixes
//! around it at: the recorder interpolated the marker's coordinates over the
//! recorded positions of those same fixes. A fix the receiver dead-reckoned is
//! re-placed between the fixes around it, because the coordinates a receiver
//! writes without a heading are often its own dead reckoning.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::highlight::DataCategory;
use gt_types::markers::EventMarker;
use gt_types::mercator;
use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{FileSource, LoadedFile};
use rstest::rstest;
use uom::si::angle::degree;
use uom::si::f64::Angle;

/// Satellites a receiver with a full solution reports in fix.
const SATELLITES_IN_FIX: u32 = 12;

/// Satellites a receiver reports it can see while none of them is in fix.
const SATELLITES_IN_VIEW: u32 = 4;

/// Two positions this close are one place on the map.
const POSITION_TOLERANCE_METERS: f64 = 0.1;

/// The longitudes alone distinguish the fixes: every fixture here shares this
/// latitude unless it states another.
const LATITUDE_DEGREES: f64 = 55.0;

fn gps_time(millis: i64) -> GpsTime {
    GpsTime::from_utc(utc_time(millis))
}

fn utc_time(millis: i64) -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(millis)
}

/// A satellite report holding `in_fix` satellites in the solution and
/// `in_view` further satellites it sees without using them.
fn satellite_report(millis: i64, in_fix: u32, in_view: u32) -> Satellites {
    let satellites = (0..in_fix + in_view)
        .map(|index| {
            Satellite::new(
                Constellation::Gps,
                index + 1,
                None,
                None,
                None,
                index < in_fix,
            )
        })
        .collect();
    Satellites::new(Some(gps_time(millis)), None, satellites)
}

fn fix(
    millis: i64,
    lat: Latitude,
    lon: Longitude,
    heading: Option<Angle>,
    satellites: Option<Satellites>,
) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(gps_time(millis))
        .lat(lat)
        .lon(lon)
        .maybe_heading(heading)
        .build();
    NavPoint::new(tpv, satellites)
}

fn heading_east() -> Option<Angle> {
    Some(Angle::new::<degree>(90.0))
}

/// A fix the receiver measured: a heading and a full solution behind it.
fn measured_fix(millis: i64, lon: Longitude) -> NavPoint {
    fix(
        millis,
        Latitude::new(LATITUDE_DEGREES),
        lon,
        heading_east(),
        Some(satellite_report(millis, SATELLITES_IN_FIX, 0)),
    )
}

/// An epoch the receiver dead-reckoned: no heading, and a satellite report
/// with nothing in fix.
fn dead_reckoned_fix(millis: i64, lat: Latitude, lon: Longitude) -> NavPoint {
    fix(
        millis,
        lat,
        lon,
        None,
        Some(satellite_report(millis, 0, SATELLITES_IN_VIEW)),
    )
}

fn event_marker(millis: i64, lat: Latitude, lon: Longitude) -> EventMarker {
    EventMarker::new(utc_time(millis), "marker/note".to_owned(), None, lat, lon)
}

fn build(points: &[NavPoint], event_markers: Vec<EventMarker>) -> LoadedFile {
    gt_track_builder::build_loaded_file(
        "ghost_stretch.gtd".to_owned(),
        points,
        &[],
        event_markers,
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ghost_stretch.gtd")),
        FileMeta::default(),
        vec![],
    )
}

/// Where the map draws the fix at `index` of the file's only track.
fn fix_drawn_at(file: &LoadedFile, index: usize) -> Option<(Latitude, Longitude)> {
    file.tracks
        .first()
        .and_then(|track| track.resolved_position_at(index))
}

/// Where the map draws the file's only event marker, read off the spatial
/// index the map hit-tests and culls against.
fn event_marker_drawn_at(file: &LoadedFile) -> Option<(Latitude, Longitude)> {
    let indexed = gt_track_builder::build_global_tree(std::slice::from_ref(file));
    let merc = indexed
        .iter()
        .find(|point| point.category == DataCategory::EventMarker)?
        .merc;
    let (latitude_degrees, longitude_degrees) = mercator::denormalize(merc);
    Some((
        Latitude::new(latitude_degrees),
        Longitude::new(longitude_degrees),
    ))
}

/// A marker and the fix stamped at its time are drawn at one place. An event
/// marker holds the position the recorder interpolated for it from the
/// coordinates the receiver wrote (`interpolate_event_markers` in the Rust
/// SDK's builder), and the builder re-places the dead-reckoned fixes those
/// coordinates come from.
#[test]
fn an_event_marker_is_drawn_where_the_fix_at_its_time_is_drawn() {
    const MARKER_MILLIS: i64 = 10_000;
    let recorded_latitude = Latitude::new(55.2);
    let recorded_longitude = Longitude::new(0.5);

    let points = vec![
        measured_fix(0, Longitude::new(0.0)),
        dead_reckoned_fix(MARKER_MILLIS, recorded_latitude, recorded_longitude),
        measured_fix(20_000, Longitude::new(1.0)),
    ];
    let file = build(
        &points,
        vec![event_marker(
            MARKER_MILLIS,
            recorded_latitude,
            recorded_longitude,
        )],
    );

    let (marker_latitude, marker_longitude) =
        event_marker_drawn_at(&file).expect("the marker is assigned to the track");
    let (fix_latitude, fix_longitude) =
        fix_drawn_at(&file, 1).expect("the dead-reckoned fix is drawn between its anchors");
    let offset_m = gt_geo_math::haversine_m(
        marker_latitude,
        marker_longitude,
        fix_latitude,
        fix_longitude,
    );

    assert!(
        offset_m < POSITION_TOLERANCE_METERS,
        "the marker is drawn {offset_m} m from the fix stamped at its own time"
    );
}

/// A marker stamped at the last fix of its track is drawn where that last fix
/// is drawn: it has no fix after it to be placed between.
#[test]
fn an_event_marker_at_a_dead_reckoned_last_fix_is_drawn_where_that_fix_is_drawn() {
    const MARKER_MILLIS: i64 = 20_000;
    let recorded_latitude = Latitude::new(LATITUDE_DEGREES);
    let recorded_longitude = Longitude::new(0.5);

    let points = vec![
        measured_fix(0, Longitude::new(0.0)),
        measured_fix(10_000, Longitude::new(0.001)),
        dead_reckoned_fix(MARKER_MILLIS, recorded_latitude, recorded_longitude),
    ];
    let file = build(
        &points,
        vec![event_marker(
            MARKER_MILLIS,
            recorded_latitude,
            recorded_longitude,
        )],
    );

    let (marker_latitude, marker_longitude) =
        event_marker_drawn_at(&file).expect("the marker is assigned to the track");
    let (fix_latitude, fix_longitude) =
        fix_drawn_at(&file, 2).expect("the dead-reckoned fix is drawn at its one anchor");
    let offset_m = gt_geo_math::haversine_m(
        marker_latitude,
        marker_longitude,
        fix_latitude,
        fix_longitude,
    );

    assert!(
        offset_m < POSITION_TOLERANCE_METERS,
        "the marker is drawn {offset_m} m from the last fix of its track"
    );
}

/// A marker among fixes the receiver measured keeps the coordinates the
/// recording holds for it: the recorder interpolated them over those same
/// positions. The cases are a marker between two fixes and one stamped at the
/// track's last fix, which has no fix after it.
#[rstest]
#[case::between_two_fixes(5_000)]
#[case::at_the_last_fix(10_000)]
fn an_event_marker_among_measured_fixes_is_drawn_at_its_recorded_coordinates(
    #[case] marker_millis: i64,
) {
    let recorded_latitude = Latitude::new(LATITUDE_DEGREES);
    let recorded_longitude = Longitude::new(0.5);

    let points = vec![
        measured_fix(0, Longitude::new(0.0)),
        measured_fix(10_000, Longitude::new(1.0)),
    ];
    let file = build(
        &points,
        vec![event_marker(
            marker_millis,
            recorded_latitude,
            recorded_longitude,
        )],
    );

    let (marker_latitude, marker_longitude) =
        event_marker_drawn_at(&file).expect("the marker is assigned to the track");
    let offset_m = gt_geo_math::haversine_m(
        marker_latitude,
        marker_longitude,
        recorded_latitude,
        recorded_longitude,
    );

    assert!(
        offset_m < POSITION_TOLERANCE_METERS,
        "the marker is drawn {offset_m} m from the coordinates the recording holds for it"
    );
}
