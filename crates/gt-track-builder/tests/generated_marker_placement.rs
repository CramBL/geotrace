//! Where the builder places a generated marker: on the point it marks, as the
//! map draws that point.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::fixtures::{self, FixKind};
use gt_types::markers::{GeneratedMarker, GeneratedMarkerKind};
use gt_types::nav_point::NavPoint;
use gt_types::track::FileSource;

/// Every fix of the track shares this latitude.
const LATITUDE_DEGREES: f64 = 55.0;

/// 1e-7° is about 1 cm. The great circle between two fixes at one latitude
/// arcs a few 1e-9° poleward at its midpoint, so the drawn epoch does not sit
/// at exactly 55°.
const DEGREES_TOLERANCE: f64 = 1e-7;

fn utc_time(secs: i64) -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(secs)
}

/// A measured fix: heading present and a full solution behind it.
fn measured_fix(secs: i64, lon_degrees: f64, system_clock_ahead: Duration) -> NavPoint {
    fixtures::nav_point_with_host_clock(
        utc_time(secs),
        system_clock_ahead,
        Latitude::new(LATITUDE_DEGREES),
        Longitude::new(lon_degrees),
        FixKind::Measured,
    )
}

/// An epoch the receiver dead-reckoned and wrote at the null island: no heading
/// and no satellite report, so the builder redraws it between its neighbours.
fn dead_reckoned_fix_at_the_null_island(secs: i64, system_clock_ahead: Duration) -> NavPoint {
    fixtures::nav_point_with_host_clock(
        utc_time(secs),
        system_clock_ahead,
        Latitude::new(0.0),
        Longitude::new(0.0),
        FixKind::GhostWithoutHeading,
    )
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
