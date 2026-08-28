//! Where the builder places a generated marker: on the point it marks, as the
//! map draws that point.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::markers::{GeneratedMarker, GeneratedMarkerKind};
use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::{GpsTime, SysTime};
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::FileSource;
use uom::si::angle::degree;
use uom::si::f64::Angle;

/// Every fix of the track shares this latitude.
const LATITUDE_DEGREES: f64 = 55.0;

const SATELLITES_IN_FIX: u32 = 12;

/// 1e-7° is about 1 cm. The great circle between two fixes at one latitude
/// arcs a few 1e-9° poleward at its midpoint, so the drawn epoch does not sit
/// at exactly 55°.
const DEGREES_TOLERANCE: f64 = 1e-7;

fn gps_time(secs: i64) -> GpsTime {
    GpsTime::from_utc(DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(secs))
}

fn system_time(secs: i64, ahead_of_gps: Duration) -> SysTime {
    SysTime::from_utc(DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(secs) + ahead_of_gps)
}

/// A measured fix: heading present and a full solution behind it.
#[expect(
    clippy::expect_used,
    reason = "Test data generation with hardcoded values"
)]
fn measured_fix(secs: i64, lon_degrees: f64, system_clock_ahead: Duration) -> NavPoint {
    let time = gps_time(secs);
    let tpv = TimePositionVelocity::builder()
        .time(time)
        .lat(Latitude::new(LATITUDE_DEGREES))
        .lon(Longitude::new(lon_degrees))
        .heading(Angle::new::<degree>(90.0))
        .sys_time(system_time(secs, system_clock_ahead))
        .build();
    let satellites = (1..=SATELLITES_IN_FIX)
        .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, true))
        .collect();
    NavPoint::new(tpv, Some(Satellites::new(Some(time), None, satellites)))
        .expect("coordinates in range")
}

/// An epoch the receiver dead-reckoned and wrote at the null island: no heading
/// and no satellite report, so the builder redraws it between its neighbours.
#[expect(
    clippy::expect_used,
    reason = "Test data generation with hardcoded values"
)]
fn dead_reckoned_fix_at_the_null_island(secs: i64, system_clock_ahead: Duration) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(gps_time(secs))
        .lat(Latitude::new(0.0))
        .lon(Longitude::new(0.0))
        .sys_time(system_time(secs, system_clock_ahead))
        .build();
    NavPoint::new(tpv, None).expect("coordinates in range")
}

fn generated_markers(points: &[NavPoint]) -> Vec<GeneratedMarker> {
    let file = gt_track_builder::build_loaded_file(
        "markers.gtd".to_owned(),
        points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("markers.gtd")),
        FileMeta::default(),
        vec![],
    );
    file.tracks
        .first()
        .map(|track| track.generated_markers.clone())
        .unwrap_or_default()
}

/// The system clock departs by an hour for the dead-reckoned epoch alone and
/// comes back, which the builder marks as a clock offset excursion at the
/// sample that departed furthest: the dead-reckoned one. That epoch is drawn
/// halfway between its neighbours at 12.003° E, and the marker belongs on it.
/// Placed where the receiver wrote the epoch, the marker sits at the null
/// island while the track it annotates is in Denmark.
#[test]
fn a_clock_excursion_marker_sits_where_its_fix_is_drawn() {
    let steady = Duration::milliseconds(234);
    let departed = Duration::hours(1) + Duration::minutes(9);
    let points = vec![
        measured_fix(1000, 12.0, steady),
        measured_fix(1001, 12.001, steady),
        measured_fix(1002, 12.002, steady),
        dead_reckoned_fix_at_the_null_island(1003, departed),
        measured_fix(1004, 12.004, steady),
        measured_fix(1005, 12.005, steady),
        measured_fix(1006, 12.006, steady),
        measured_fix(1007, 12.007, steady),
    ];

    let markers = generated_markers(&points);

    let [marker] = markers
        .iter()
        .filter(|marker| {
            matches!(
                marker.kind,
                GeneratedMarkerKind::ClockOffsetExcursion { .. }
            )
        })
        .collect::<Vec<_>>()[..]
    else {
        panic!("expected one clock offset excursion, got {markers:?}");
    };
    assert!(
        (marker.lat.as_degrees() - LATITUDE_DEGREES).abs() < DEGREES_TOLERANCE
            && (marker.lon.as_degrees() - 12.003).abs() < DEGREES_TOLERANCE,
        "marker placed at ({}, {}), the fix it marks is drawn at ({LATITUDE_DEGREES}, 12.003)",
        marker.lat.as_degrees(),
        marker.lon.as_degrees()
    );
}
