#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use geotrace_sdk::{Angle, DateTime, Duration, Unit, Utc};
use geotrace_sdk::{
    Annotation, BuildError, Channel, Constellation, EventMarker, NavFileBuilder, NavFix,
    NavFixTime, Satellite, SatelliteReport,
};
use proptest::prelude::*;
use rstest::rstest;

fn t(offset_ms: i64) -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed base timestamp is always valid")]
    let base = DateTime::from_timestamp(1_748_000_000, 0).expect("valid");
    base + Duration::milliseconds(offset_ms)
}

fn simple_fix(offset_ms: i64) -> NavFix {
    NavFix::builder()
        .time(NavFixTime::Receiver(t(offset_ms)))
        .lat(Angle::degrees(55.0))
        .lon(Angle::degrees(12.0))
        .heading(Angle::degrees(0.0))
        .build()
}

fn simple_report(offset_ms: i64) -> SatelliteReport {
    SatelliteReport::builder()
        .time(NavFixTime::Receiver(t(offset_ms)))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .in_fix(true)
                .build(),
        ])
        .build()
}

fn fix_at_lon(offset_ms: i64, lon_deg: f64) -> NavFix {
    NavFix::builder()
        .time(NavFixTime::Receiver(t(offset_ms)))
        .lat(Angle::degrees(0.0))
        .lon(Angle::degrees(lon_deg))
        .heading(Angle::degrees(90.0))
        .build()
}

#[rstest]
#[case::eastward_a_quarter_of_the_way(179.95, -179.95, t(2500), 179.975)]
#[case::eastward_halfway(179.95, -179.95, t(5000), -180.0)]
#[case::eastward_three_quarters_of_the_way(179.95, -179.95, t(7500), -179.975)]
#[case::westward_a_quarter_of_the_way(-179.95, 179.95, t(2500), -179.975)]
#[case::westward_halfway(-179.95, 179.95, t(5000), -180.0)]
#[case::westward_three_quarters_of_the_way(-179.95, 179.95, t(7500), 179.975)]
#[case::exactly_180_apart_halfway(0.0, 180.0, t(5000), -90.0)]
fn an_annotation_between_two_fixes_takes_the_shortest_arc_in_longitude(
    #[case] first_fix_lon_deg: f64,
    #[case] second_fix_lon_deg: f64,
    #[case] time: DateTime<Utc>,
    #[case] expected_lon_deg: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at_lon(0, first_fix_lon_deg));
    recorder.add_nav_fix(fix_at_lon(10_000, second_fix_lon_deg));
    recorder.add_annotation(Annotation::builder().time(time).label("note").build()?);

    let nav_file = recorder.finish()?;
    let marker = &nav_file.markers()[0];
    assert!(
        (marker.lon.as_degrees() - expected_lon_deg).abs() < 1e-9,
        "lon is {}, expected {expected_lon_deg}",
        marker.lon.as_degrees()
    );
    Ok(())
}

#[rstest]
#[case::at_the_first_fix_time(t(0), 10.0, 20.0)]
#[case::halfway_between_two_fixes(t(500), 11.0, 22.0)]
#[case::at_the_last_fix_time(t(1000), 12.0, 24.0)]
fn an_annotation_within_the_fix_time_span_resolves_to_a_position_in_strict_mode(
    #[case] time: DateTime<Utc>,
    #[case] expected_lat_deg: f64,
    #[case] expected_lon_deg: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t(0)))
            .lat(Angle::degrees(10.0))
            .lon(Angle::degrees(20.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t(1000)))
            .lat(Angle::degrees(12.0))
            .lon(Angle::degrees(24.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_annotation(Annotation::builder().time(time).label("note").build()?);

    let nav_file = recorder.finish()?;
    let marker = &nav_file.markers()[0];
    assert!(
        (marker.lat.as_degrees() - expected_lat_deg).abs() < 1e-10,
        "lat is {}, expected {expected_lat_deg}",
        marker.lat.as_degrees()
    );
    assert!(
        (marker.lon.as_degrees() - expected_lon_deg).abs() < 1e-10,
        "lon is {}, expected {expected_lon_deg}",
        marker.lon.as_degrees()
    );
    Ok(())
}

#[test]
fn an_annotation_at_the_time_of_the_only_fix_resolves_to_that_fix_in_strict_mode()
-> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(0));
    recorder.add_annotation(Annotation::builder().time(t(0)).build()?);

    let nav_file = recorder.finish()?;
    let marker = &nav_file.markers()[0];
    assert!((marker.lat.as_degrees() - 55.0).abs() < 1e-10);
    assert!((marker.lon.as_degrees() - 12.0).abs() < 1e-10);
    Ok(())
}

#[test]
fn an_annotation_between_two_fixes_in_host_clock_order_is_placed_between_them()
-> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t(10_000)))
            .lat(Angle::degrees(56.0))
            .lon(Angle::degrees(16.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Both {
                gps: t(12_000),
                sys: t(8_000),
            })
            .lat(Angle::degrees(54.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_annotation(Annotation::builder().time(t(9_000)).label("note").build()?);

    let nav_file = recorder.finish()?;
    let marker = &nav_file.markers()[0];
    assert!(
        (marker.lat.as_degrees() - 55.0).abs() < 1e-10,
        "lat is {}, expected 55",
        marker.lat.as_degrees()
    );
    assert!(
        (marker.lon.as_degrees() - 14.0).abs() < 1e-10,
        "lon is {}, expected 14",
        marker.lon.as_degrees()
    );
    Ok(())
}

#[test]
fn an_annotation_at_a_host_time_two_fixes_share_is_placed_on_the_earlier_by_receiver_time()
-> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Both {
                gps: t(13_000),
                sys: t(9_000),
            })
            .lat(Angle::degrees(56.0))
            .lon(Angle::degrees(16.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Both {
                gps: t(12_000),
                sys: t(9_000),
            })
            .lat(Angle::degrees(54.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_annotation(Annotation::builder().time(t(9_000)).label("note").build()?);

    let nav_file = recorder.finish()?;
    let marker = &nav_file.markers()[0];
    assert!(
        (marker.lat.as_degrees() - 54.0).abs() < 1e-10,
        "lat is {}, expected 54",
        marker.lat.as_degrees()
    );
    assert!(
        (marker.lon.as_degrees() - 12.0).abs() < 1e-10,
        "lon is {}, expected 12",
        marker.lon.as_degrees()
    );
    Ok(())
}

#[test]
fn an_annotation_one_microsecond_after_the_last_fix_is_outside_the_range_in_strict_mode() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(0));
    recorder.add_nav_fix(simple_fix(1000));
    recorder.add_annotation(
        Annotation::builder()
            .time(t(1000) + Duration::microseconds(1))
            .build()
            .expect("an annotation without a label is accepted"),
    );

    assert!(matches!(
        recorder.finish(),
        Err(BuildError::AnnotationsOutsideRange { count: 1 })
    ));
}

#[test]
fn annotation_before_first_fix_lenient() -> Result<(), Box<dyn std::error::Error>> {
    // Annotation before the first fix is clamped to the first fix position.
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t(1000)))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_annotation(Annotation::builder().time(t(0)).build()?);
    let nav_file = recorder.finish()?;
    let m = &nav_file.markers()[0];
    assert!((m.lat.as_degrees() - 55.0).abs() < 1e-10);
    assert!((m.lon.as_degrees() - 12.0).abs() < 1e-10);
    Ok(())
}

#[test]
fn annotation_after_last_fix_lenient() -> Result<(), Box<dyn std::error::Error>> {
    // Annotation after the last fix is clamped to the last fix position.
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t(0)))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_annotation(Annotation::builder().time(t(5000)).build()?);
    let nav_file = recorder.finish()?;
    let m = &nav_file.markers()[0];
    assert!((m.lat.as_degrees() - 55.0).abs() < 1e-10);
    assert!((m.lon.as_degrees() - 12.0).abs() < 1e-10);
    Ok(())
}

#[test]
fn strict_mode_fails_on_an_annotation_before_the_first_fix_and_not_on_orphan_reports() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(0));
    recorder.add_satellite_report(simple_report(2000));
    recorder.add_satellite_report(simple_report(3000));
    recorder.add_annotation(
        Annotation::builder()
            .time(t(-1000))
            .build()
            .expect("an annotation without a label is accepted"),
    );

    assert!(matches!(
        recorder.finish(),
        Err(BuildError::AnnotationsOutsideRange { count: 1 })
    ));
}

#[test]
fn no_nav_fixes_with_annotations_lenient() {
    // NoNavFixes is returned even in lenient mode - positions cannot be interpolated at all.
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    recorder.add_annotation(
        Annotation::builder()
            .time(t(0))
            .build()
            .expect("an annotation without a label is accepted"),
    );
    assert!(matches!(recorder.finish(), Err(BuildError::NoNavFixes)));
}

#[test]
fn unsorted_insertion() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    // Insert in reverse chronological order. finish() must sort correctly.
    for i in (0..5).rev() {
        recorder.add_nav_fix(simple_fix(i * 1000));
        recorder.add_satellite_report(simple_report(i * 1000 + 100));
    }
    let nav_file = recorder.finish()?;
    let times: Vec<_> = nav_file
        .nav_points()
        .iter()
        .map(|p| p.fix.gps_time())
        .collect();
    let mut sorted = times.clone();
    sorted.sort();
    assert_eq!(times, sorted);
    assert!(nav_file.nav_points().iter().all(|p| p.satellites.is_some()));
    Ok(())
}

proptest! {
    /// GPS times in the output are always monotonically non-decreasing regardless
    /// of insertion order and the mix of gps-only vs sys-time satellite reports.
    #[test]
    fn nav_point_gps_times_are_monotonic(
        fix_offsets_ms in prop::collection::vec(1_i64..=30_000_i64, 1..=8_usize),
        report_offsets_ms in prop::collection::vec(1_i64..=30_000_i64, 0..=8_usize),
        insert_reversed in proptest::bool::ANY,
    ) {
        // Build cumulative monotonic timestamps from inter-arrival deltas.
        let mut gps_ms: Vec<i64> = fix_offsets_ms
            .iter()
            .scan(0_i64, |acc, &d| { *acc += d; Some(*acc) })
            .collect();
        let mut sat_ms: Vec<i64> = report_offsets_ms
            .iter()
            .scan(0_i64, |acc, &d| { *acc += d; Some(*acc) })
            .collect();

        if insert_reversed {
            gps_ms.reverse();
            sat_ms.reverse();
        }

        let mut recorder = NavFileBuilder::new().open();
        for &ms in &gps_ms {
            recorder.add_nav_fix(simple_fix(ms));
        }
        for &ms in &sat_ms {
            recorder.add_satellite_report(simple_report(ms));
        }

        if let Ok(nav_file) = recorder.finish() {
            let times: Vec<_> = nav_file
                .nav_points()
                .iter()
                .map(|p| p.fix.gps_time())
                .collect();
            for w in times.windows(2) {
                prop_assert!(
                    w[0] <= w[1],
                    "GPS times not monotonic: {:?} > {:?}",
                    w[0],
                    w[1]
                );
            }
        }
    }
}

#[test]
fn add_dispatches_to_the_matching_typed_method() -> Result<(), BuildError> {
    // Same data, two fixes bracketing the annotation/event so both land in range.
    let annotation = || {
        Annotation::builder()
            .time(t(500))
            .label("mid")
            .build()
            .expect("the marker label fits its field")
    };
    let marker = || {
        EventMarker::builder()
            .variant_path("power/boot")
            .sys_time(t(500))
            .annotation("cold start")
            .build()
            .expect("valid event marker")
    };
    let channel = || {
        Channel::builder()
            .name("incline")
            .unit(Unit::DEG)
            .times(vec![t(0)])
            .values(vec![1.5])
            .build()
            .expect("valid channel")
    };

    // Built the explicit way.
    let mut typed = NavFileBuilder::new().open();
    typed
        .add_nav_fix(simple_fix(0))
        .add_nav_fix(simple_fix(1000))
        .add_satellite_report(simple_report(100))
        .add_annotation(annotation())
        .add_event_marker(marker())
        .add_channel(channel());
    let typed = typed.finish()?;

    // Built via the type-dispatched add().
    let mut via_add = NavFileBuilder::new().open();
    via_add
        .add(simple_fix(0))
        .add(simple_fix(1000))
        .add(simple_report(100))
        .add(annotation())
        .add(marker())
        .add(channel());
    let via_add = via_add.finish()?;

    // add() must produce exactly the same file as the typed methods.
    assert_eq!(via_add, typed);
    // Guard against a vacuous match: every add() arm actually contributed.
    assert_eq!(via_add.nav_points().len(), 2);
    assert!(via_add.nav_points().iter().any(|p| p.satellites.is_some()));
    assert_eq!(via_add.markers().len(), 1);
    assert_eq!(via_add.event_markers().len(), 1);
    assert_eq!(via_add.channels().len(), 1);
    Ok(())
}
