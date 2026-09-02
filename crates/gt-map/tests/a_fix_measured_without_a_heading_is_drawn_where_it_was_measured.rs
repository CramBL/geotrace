//! Where the map draws the fix a receiver measured at the turning point of a
//! cul-de-sac, with a full solution behind it and no course to report.
//!
//! The recording runs east up the street, turns at the far end, and comes back
//! west on the other side. The fix at the turn is the tip of the track. It
//! holds twelve satellites in fix and no heading: the receiver stands still
//! there.

mod support;

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{
    FileSource, GpsTime, Latitude, LoadedFile, Longitude, NavPoint, TimePositionVelocity,
};
use uom::si::angle::degree;
use uom::si::f64::Angle;

const SECONDS_BETWEEN_FIXES: i64 = 30;

/// Fixes of each leg, at [`LONGITUDE_STEP_DEGREES`] apart.
const FIXES_PER_LEG: usize = 6;

/// The fix at the turning point, between the two legs.
const TURN_FIX_INDEX: usize = FIXES_PER_LEG;

const FIRST_LONGITUDE_DEGREES: f64 = 12.550;

/// Longitude between consecutive fixes of a leg, about 420 m at these
/// latitudes. The six steps of a leg fill four fifths of the viewport once the
/// map frames the recording.
const LONGITUDE_STEP_DEGREES: f64 = 0.006_69;

/// The latitude of the leg running east, and of the leg running west. They are
/// about 222 m apart, which is a tenth of the viewport.
const EASTBOUND_LATITUDE_DEGREES: f64 = 55.674;
const WESTBOUND_LATITUDE_DEGREES: f64 = 55.676;

const EAST_HEADING_DEGREES: f64 = 90.0;
const WEST_HEADING_DEGREES: f64 = 270.0;

/// Satellites the receiver reports in fix under a full solution.
const SATELLITES_IN_FIX: u32 = 12;

fn time_of(index: usize) -> DateTime<Utc> {
    support::epoch() + Duration::seconds(index as i64 * SECONDS_BETWEEN_FIXES)
}

/// The longitude of the fix at `step` steps up the street.
fn longitude_of(step: usize) -> Longitude {
    Longitude::new(FIRST_LONGITUDE_DEGREES + step as f64 * LONGITUDE_STEP_DEGREES)
}

fn full_solution(index: usize) -> Satellites {
    let satellites = (0..SATELLITES_IN_FIX)
        .map(|prn| Satellite::new(Constellation::Gps, prn + 1, None, None, None, true))
        .collect();
    Satellites::new(Some(GpsTime::from_utc(time_of(index))), None, satellites)
}

fn fix(index: usize, lat: Latitude, lon: Longitude, heading: Option<Angle>) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time_of(index)))
        .lat(lat)
        .lon(lon)
        .maybe_heading(heading)
        .build();
    NavPoint::new(tpv, Some(full_solution(index)))
}

/// The fixes of the leg running east, then the fix at the turn, then the fixes
/// of the leg running west.
fn a_recording_that_turns_at_the_end_of_a_cul_de_sac() -> Vec<LoadedFile> {
    let eastbound = (0..FIXES_PER_LEG).map(|step| {
        fix(
            step,
            Latitude::new(EASTBOUND_LATITUDE_DEGREES),
            longitude_of(step),
            Some(Angle::new::<degree>(EAST_HEADING_DEGREES)),
        )
    });
    let turn = std::iter::once(fix(
        TURN_FIX_INDEX,
        Latitude::new(EASTBOUND_LATITUDE_DEGREES.midpoint(WESTBOUND_LATITUDE_DEGREES)),
        longitude_of(FIXES_PER_LEG),
        None,
    ));
    let westbound = (0..FIXES_PER_LEG).map(|step| {
        fix(
            TURN_FIX_INDEX + 1 + step,
            Latitude::new(WESTBOUND_LATITUDE_DEGREES),
            longitude_of(FIXES_PER_LEG - 1 - step),
            Some(Angle::new::<degree>(WEST_HEADING_DEGREES)),
        )
    });

    let points: Vec<NavPoint> = eastbound.chain(turn).chain(westbound).collect();
    vec![gt_track_builder::build_loaded_file(
        "cul_de_sac.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("cul_de_sac.gtd")),
        FileMeta::default(),
        vec![],
    )]
}

#[test]
fn snapshot_a_fix_measured_without_a_heading_is_drawn_at_the_tip_of_the_track() {
    let files = a_recording_that_turns_at_the_end_of_a_cul_de_sac();
    let mut harness = support::rendered_map(files);
    harness.snapshot_loose("fix_measured_without_a_heading");
}
