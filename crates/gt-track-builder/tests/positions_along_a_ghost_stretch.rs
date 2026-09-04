//! Where the map draws the fixes of a stretch the builder re-placed, and the
//! event markers stamped along it.
//!
//! The builder re-places a fix the receiver dead-reckoned between the fixes
//! around it, and leaves a fix the receiver measured at the position it
//! measured. Satellites in fix are what tells the two apart. An event marker
//! is placed over the positions the fixes around it are drawn at: the recorder
//! interpolated the marker's coordinates over the recorded positions of those
//! same fixes.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, FixPlacementRule, SegmentationConfig};
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

/// Two longitudes this close are one meridian to within a millimetre, which
/// covers the great circle's departure from a straight line over the steps
/// these fixtures take.
const POSITION_TOLERANCE_DEGREES: f64 = 1e-8;

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

/// A fix a receiver reports while it stands still: a full solution behind it,
/// and no heading, because a receiver at rest has no course to report.
fn fix_without_a_heading(millis: i64, lon: Longitude) -> NavPoint {
    fix(
        millis,
        Latitude::new(LATITUDE_DEGREES),
        lon,
        None,
        Some(satellite_report(millis, SATELLITES_IN_FIX, 0)),
    )
}

/// A fix the receiver reports with a course but nothing in fix, which
/// [`NavPoint::is_ghost_fix`] calls a ghost and the map draws hollow.
fn fix_with_a_heading_and_nothing_in_fix(millis: i64, lon: Longitude) -> NavPoint {
    fix(
        millis,
        Latitude::new(LATITUDE_DEGREES),
        lon,
        heading_east(),
        Some(satellite_report(millis, 0, SATELLITES_IN_VIEW)),
    )
}

fn event_marker(millis: i64, lat: Latitude, lon: Longitude) -> EventMarker {
    EventMarker::new(utc_time(millis), "marker/note".to_owned(), None, lat, lon)
}

fn build_under(
    points: &[NavPoint],
    event_markers: Vec<EventMarker>,
    rule: FixPlacementRule,
) -> LoadedFile {
    gt_track_builder::build_loaded_file(
        "ghost_stretch.gtd".to_owned(),
        points,
        &[],
        event_markers,
        vec![],
        &[],
        &SegmentationConfig {
            fix_placement_rule: rule,
            ..SegmentationConfig::default()
        },
        FileSource::GtdPath(PathBuf::from("ghost_stretch.gtd")),
        FileMeta::default(),
        vec![],
    )
}

fn build(points: &[NavPoint], event_markers: Vec<EventMarker>) -> LoadedFile {
    build_under(points, event_markers, FixPlacementRule::default())
}

/// Where the map draws the fix at `index` of the file's only track.
fn fix_drawn_at(file: &LoadedFile, index: usize) -> Option<(Latitude, Longitude)> {
    file.tracks
        .first()
        .and_then(|track| track.resolved_position_at(index))
}

/// Where the map draws the fix at `index` of the only track built from
/// `points`.
fn drawn_at(points: &[NavPoint], index: usize) -> Option<(Latitude, Longitude)> {
    fix_drawn_at(&build(points, vec![]), index)
}

fn drawn_longitude_degrees(points: &[NavPoint], index: usize) -> Option<f64> {
    Some(drawn_at(points, index)?.1.as_degrees())
}

/// The longitude the fix at `index` is drawn at under `rule`.
fn drawn_longitude_degrees_under(
    points: &[NavPoint],
    index: usize,
    rule: FixPlacementRule,
) -> Option<f64> {
    Some(
        fix_drawn_at(&build_under(points, vec![], rule), index)?
            .1
            .as_degrees(),
    )
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

/// A receiver reports satellites in fix whenever it measured the position it
/// wrote. It leaves the heading out whenever it has no course to report.
#[test]
fn a_fix_with_satellites_in_fix_keeps_its_measured_position() {
    let points = vec![
        fix_without_a_heading(0, Longitude::new(0.0)),
        fix_without_a_heading(1_000, Longitude::new(5.0)),
        fix_without_a_heading(10_000, Longitude::new(10.0)),
    ];

    let longitude = drawn_longitude_degrees(&points, 1).expect("the fix is drawn");

    assert!(
        (longitude - 5.0).abs() < POSITION_TOLERANCE_DEGREES,
        "the fix with {SATELLITES_IN_FIX} satellites in fix measured at lon 5.0 is drawn at \
         lon {longitude}"
    );
}

/// The first fix of a stop is where the receiver came to rest, and it is drawn
/// there. Every fix of a stop has no heading: a receiver stops reporting a
/// course as soon as it stands still.
#[test]
fn the_first_fix_after_the_receiver_stops_is_drawn_where_it_stopped() {
    const STOP_LONGITUDE_DEGREES: f64 = 0.010;
    let stop_longitude = Longitude::new(STOP_LONGITUDE_DEGREES);
    let points = vec![
        measured_fix(0, Longitude::new(0.000)),
        measured_fix(10_000, Longitude::new(0.005)),
        fix_without_a_heading(20_000, stop_longitude),
        fix_without_a_heading(30_000, stop_longitude),
        fix_without_a_heading(40_000, stop_longitude),
    ];

    let longitude = drawn_longitude_degrees(&points, 2).expect("the fix is drawn");

    assert!(
        (longitude - STOP_LONGITUDE_DEGREES).abs() < POSITION_TOLERANCE_DEGREES,
        "the first fix of the stop is drawn at lon {longitude}, back along the road from the \
         lon {STOP_LONGITUDE_DEGREES} it was measured at"
    );
}

/// A fix whose share of its anchors' time span is negative is placed between
/// them. A fix stamped before the one ahead of it keeps its place in the
/// recording: nothing sorts the fixes.
#[test]
fn a_ghost_fix_stamped_before_its_anchors_is_placed_between_them() {
    const FIRST_ANCHOR_LONGITUDE_DEGREES: f64 = 0.000;
    const SECOND_ANCHOR_LONGITUDE_DEGREES: f64 = 0.001;
    let points = vec![
        measured_fix(300_000, Longitude::new(FIRST_ANCHOR_LONGITUDE_DEGREES)),
        dead_reckoned_fix(
            1_000,
            Latitude::new(LATITUDE_DEGREES),
            Longitude::new(FIRST_ANCHOR_LONGITUDE_DEGREES),
        ),
        measured_fix(300_500, Longitude::new(SECOND_ANCHOR_LONGITUDE_DEGREES)),
    ];

    let longitude = drawn_longitude_degrees(&points, 1).expect("the fix is drawn");

    assert!(
        (FIRST_ANCHOR_LONGITUDE_DEGREES..=SECOND_ANCHOR_LONGITUDE_DEGREES).contains(&longitude),
        "the dead-reckoned fix is drawn at lon {longitude}, outside the \
         lon {FIRST_ANCHOR_LONGITUDE_DEGREES} to {SECOND_ANCHOR_LONGITUDE_DEGREES} its anchors \
         span"
    );
}

/// The builder leaves this fix at the coordinates it was recorded with. A
/// receiver that loses its solution keeps reporting a course from its own dead
/// reckoning, and [`NavPoint::is_ghost_fix`] draws the fix hollow.
#[test]
fn a_fix_with_a_heading_and_nothing_in_fix_keeps_its_recorded_position() {
    let points = vec![
        measured_fix(0, Longitude::new(0.0)),
        fix_with_a_heading_and_nothing_in_fix(10_000, Longitude::new(5.0)),
        measured_fix(20_000, Longitude::new(1.0)),
    ];

    let longitude = drawn_longitude_degrees(&points, 1).expect("the fix is drawn");

    assert!(
        (longitude - 5.0).abs() < POSITION_TOLERANCE_DEGREES,
        "the fix recorded at lon 5.0 is drawn at lon {longitude}"
    );
}

/// Every fix keeps the coordinates it was written with. A recording without a
/// satellite report holds nothing that distinguishes a measured fix from a
/// dead-reckoned one.
#[test]
fn a_track_without_a_satellite_report_keeps_every_recorded_position() {
    let recorded_longitudes = [0.0, 5.0, 10.0];
    let points: Vec<NavPoint> = [0, 1_000, 2_000]
        .into_iter()
        .zip(recorded_longitudes)
        .map(|(millis, longitude_degrees)| {
            fix(
                millis,
                Latitude::new(LATITUDE_DEGREES),
                Longitude::new(longitude_degrees),
                None,
                None,
            )
        })
        .collect();

    let drawn: Vec<f64> = (0..points.len())
        .map(|index| drawn_longitude_degrees(&points, index))
        .collect::<Option<Vec<f64>>>()
        .expect("every fix is drawn");

    assert_eq!(drawn, recorded_longitudes.to_vec());
}

/// A dead-reckoned fix ahead of every anchor is drawn at the one anchor it
/// has, with no span to sit inside. The SDK clamps an event marker recorded
/// before the first fix the same way.
#[test]
fn a_ghost_fix_before_every_anchor_is_placed_at_the_first_anchor() {
    const FIRST_ANCHOR_LONGITUDE_DEGREES: f64 = 0.0;
    let points = vec![
        dead_reckoned_fix(0, Latitude::new(LATITUDE_DEGREES), Longitude::new(9.0)),
        measured_fix(10_000, Longitude::new(FIRST_ANCHOR_LONGITUDE_DEGREES)),
        measured_fix(20_000, Longitude::new(1.0)),
    ];

    let longitude = drawn_longitude_degrees(&points, 0).expect("the fix is drawn");

    assert!(
        (longitude - FIRST_ANCHOR_LONGITUDE_DEGREES).abs() < POSITION_TOLERANCE_DEGREES,
        "the leading dead-reckoned fix is drawn at lon {longitude}"
    );
}

/// A dead-reckoned fix halfway in time between two anchors on opposite sides
/// of a pole is placed over the pole, on the shorter great circle between
/// them.
#[test]
fn a_ghost_fix_between_anchors_on_opposite_sides_of_a_pole_is_placed_between_them() {
    const ANCHOR_LATITUDE_DEGREES: f64 = 89.9;
    let anchor_latitude = Latitude::new(ANCHOR_LATITUDE_DEGREES);
    let points = vec![
        fix(
            0,
            anchor_latitude,
            Longitude::new(0.0),
            heading_east(),
            Some(satellite_report(0, SATELLITES_IN_FIX, 0)),
        ),
        dead_reckoned_fix(10_000, Latitude::new(0.0), Longitude::new(0.0)),
        fix(
            20_000,
            anchor_latitude,
            Longitude::new(180.0),
            heading_east(),
            Some(satellite_report(20_000, SATELLITES_IN_FIX, 0)),
        ),
    ];

    let latitude = drawn_at(&points, 1)
        .expect("the fix is drawn")
        .0
        .as_degrees();

    assert!(
        latitude >= ANCHOR_LATITUDE_DEGREES,
        "the dead-reckoned fix is drawn at lat {latitude}, below the \
         lat {ANCHOR_LATITUDE_DEGREES} both of its anchors sit at"
    );
}

/// A recording stored before the placement rule changed opens with the
/// geometry it was stored with. Under
/// [`FixPlacementRule::MissingHeading`] a fix with satellites in fix and no
/// heading is drawn between the fixes around it, a tenth of the way from the
/// one at lon 0 towards the one at lon 10.
#[test]
fn the_missing_heading_rule_draws_a_fix_with_satellites_in_fix_between_its_neighbours() {
    let points = vec![
        fix_without_a_heading(0, Longitude::new(0.0)),
        fix_without_a_heading(1_000, Longitude::new(5.0)),
        fix_without_a_heading(10_000, Longitude::new(10.0)),
    ];

    let longitude = drawn_longitude_degrees_under(&points, 1, FixPlacementRule::MissingHeading)
        .expect("the fix is drawn");

    assert!(
        (0.0..2.0).contains(&longitude),
        "the fix measured at lon 5.0 is drawn at lon {longitude}, not a tenth of the way from \
         lon 0.0 towards lon 10.0"
    );
}

/// The same recording keeps the placement it was stored with along the arc:
/// under [`FixPlacementRule::MissingHeading`] a fix stamped before both its
/// anchors is drawn west of the pair, on the great circle through them
/// continued past the first.
#[test]
fn the_missing_heading_rule_draws_a_fix_stamped_before_its_anchors_outside_them() {
    const FIRST_ANCHOR_LONGITUDE_DEGREES: f64 = 0.000;
    const SECOND_ANCHOR_LONGITUDE_DEGREES: f64 = 0.001;
    let points = vec![
        measured_fix(300_000, Longitude::new(FIRST_ANCHOR_LONGITUDE_DEGREES)),
        dead_reckoned_fix(
            1_000,
            Latitude::new(LATITUDE_DEGREES),
            Longitude::new(FIRST_ANCHOR_LONGITUDE_DEGREES),
        ),
        measured_fix(300_500, Longitude::new(SECOND_ANCHOR_LONGITUDE_DEGREES)),
    ];

    let longitude = drawn_longitude_degrees_under(&points, 1, FixPlacementRule::MissingHeading)
        .expect("the fix is drawn");

    assert!(
        longitude < FIRST_ANCHOR_LONGITUDE_DEGREES,
        "the dead-reckoned fix is drawn at lon {longitude}, inside the \
         lon {FIRST_ANCHOR_LONGITUDE_DEGREES} to {SECOND_ANCHOR_LONGITUDE_DEGREES} its anchors \
         span"
    );
}
