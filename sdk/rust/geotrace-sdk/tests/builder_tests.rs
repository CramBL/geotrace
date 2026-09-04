#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use geotrace_sdk::{Angle, DateTime, Duration, Unit, Utc, Velocity};
use geotrace_sdk::{
    Annotation, BuildError, Channel, Constellation, EventMarker, NavFileBuilder, NavFix, Satellite,
    SatelliteReport,
};
use proptest::prelude::*;

fn t(offset_ms: i64) -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed base timestamp is always valid")]
    let base = DateTime::from_timestamp(1_748_000_000, 0).expect("valid");
    base + Duration::milliseconds(offset_ms)
}

fn simple_fix(offset_ms: i64) -> NavFix {
    NavFix::builder()
        .gps_time(t(offset_ms))
        .lat(Angle::degrees(55.0))
        .lon(Angle::degrees(12.0))
        .heading(Angle::degrees(0.0))
        .build()
}

fn simple_report(offset_ms: i64) -> SatelliteReport {
    SatelliteReport::builder()
        .gps_time(t(offset_ms))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .in_fix(true)
                .build(),
        ])
        .build()
}

#[test]
fn satellite_association_within_window() -> Result<(), BuildError> {
    // Report 400 ms from fix is within the 500 ms default window.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(0));
    recorder.add_satellite_report(simple_report(400));
    let nav_file = recorder.finish()?;
    assert!(nav_file.nav_points()[0].satellites.is_some());
    Ok(())
}

#[test]
fn satellite_outside_window_creates_ghost_fix() -> Result<(), BuildError> {
    // Report 600 ms after the only real fix exceeds the 500 ms window.
    // The builder must create a dead-reckoned ghost fix carrying the report -
    // not return an error.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(0));
    recorder.add_satellite_report(simple_report(600));
    let nav_file = recorder.finish()?;

    // Real fix + 1 ghost fix.
    assert_eq!(nav_file.nav_points().len(), 2);

    // Real fix has no associated satellite report (it was outside the window).
    assert!(nav_file.nav_points()[0].satellites.is_none());

    // Ghost fix carries the satellite report.
    assert!(nav_file.nav_points()[1].satellites.is_some());

    // Ghost fix has no heading (rendered as a circle, not an arrow).
    assert!(nav_file.nav_points()[1].fix.heading.is_none());

    Ok(())
}

#[test]
fn satellite_association_tie_breaking() -> Result<(), BuildError> {
    // Two reports at +250 ms and -250 ms from the fix at t=500 ms.
    // The earlier report (t=250) must win even when added last.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(500));
    let later = SatelliteReport::builder()
        .gps_time(t(750))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Glonass)
                .prn(99u32)
                .build(),
        ])
        .build();
    let earlier = SatelliteReport::builder()
        .gps_time(t(250))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .build(),
        ])
        .build();
    recorder.add_satellite_report(later);
    recorder.add_satellite_report(earlier);

    let nav_file = recorder.finish()?;
    let rep = nav_file.nav_points()[0]
        .satellites
        .as_ref()
        .ok_or(BuildError::NoNavFixes)?;
    assert_eq!(rep.tracked[0].constellation, Constellation::Gps);
    Ok(())
}

#[test]
fn satellite_outside_narrow_window_creates_ghost_fix() -> Result<(), BuildError> {
    // Report at 200 ms exceeds the 100 ms custom window → ghost fix, not an error.
    let mut recorder = NavFileBuilder::new()
        .with_satellite_window(Duration::milliseconds(100))
        .open();
    recorder.add_nav_fix(simple_fix(0));
    recorder.add_satellite_report(simple_report(200));
    let nav_file = recorder.finish()?;
    assert_eq!(nav_file.nav_points().len(), 2);
    assert!(nav_file.nav_points()[0].satellites.is_none());
    assert!(nav_file.nav_points()[1].satellites.is_some());
    Ok(())
}

#[test]
fn satellite_association_narrowed_window_within() -> Result<(), BuildError> {
    // Report at 50 ms is within the 100 ms custom window.
    let mut recorder = NavFileBuilder::new()
        .with_satellite_window(Duration::milliseconds(100))
        .open();
    recorder.add_nav_fix(simple_fix(0));
    recorder.add_satellite_report(simple_report(50));
    let nav_file = recorder.finish()?;
    assert!(nav_file.nav_points()[0].satellites.is_some());
    Ok(())
}

/// Two reports both within the window of the same fix.
/// The closer report wins and is assigned to the fix. The runner-up must NOT
/// be silently dropped - it must become a ghost fix instead.
///
/// Before the fix, `had_candidate` tracking caused the losing report to be
/// filtered out even though it was never actually assigned anywhere.
#[test]
fn contested_loser_becomes_ghost_fix() -> Result<(), BuildError> {
    // Fix A at t=0, Fix B at t=3 000 ms (well separated so the ghost sits clearly between them).
    // Report R1 at t=400 ms → 400 ms from A (wins), 2 600 ms from B (outside window).
    // Report R2 at t=450 ms → 450 ms from A (loses to R1), 2 550 ms from B (outside window).
    // R2 has no valid fix to attach to. It must become a ghost fix between A and B.
    //
    // Ghost fixes between two real fixes receive the bearing heading, NOT None.
    // We therefore identify each point by the satellite constellation it carries.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(3000))
            .lat(Angle::degrees(55.1))
            .lon(Angle::degrees(12.1))
            .heading(Angle::degrees(0.0))
            .build(),
    );

    // R1 wins the race for fix A.
    let r1 = SatelliteReport::builder()
        .gps_time(t(400))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .in_fix(true)
                .build(),
        ])
        .build();
    // R2 loses to R1 and is too far from fix B - must not be silently dropped.
    let r2 = SatelliteReport::builder()
        .gps_time(t(450))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Glonass)
                .prn(42u32)
                .in_fix(true)
                .build(),
        ])
        .build();
    recorder.add_satellite_report(r1);
    recorder.add_satellite_report(r2);

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // 2 real fixes + 1 ghost for R2 = 3 total.
    assert_eq!(
        points.len(),
        3,
        "contested loser R2 must produce a ghost fix (got {} points)",
        points.len()
    );

    // Fix A is the first point (sorted by time). It must carry R1 (GPS).
    let r1_rep = points[0]
        .satellites
        .as_ref()
        .expect("fix A must have R1 associated");
    assert_eq!(
        r1_rep.tracked[0].constellation,
        Constellation::Gps,
        "fix A must carry R1 (GPS)"
    );

    // Fix B is the last point. It must have no satellite report.
    assert!(
        points[2].satellites.is_none(),
        "fix B must have no satellite report"
    );

    // The ghost is the middle point. It must carry R2 (Glonass).
    let r2_rep = points[1]
        .satellites
        .as_ref()
        .expect("ghost (middle point) must carry R2");
    assert_eq!(
        r2_rep.tracked[0].constellation,
        Constellation::Glonass,
        "ghost fix must carry R2 (Glonass)"
    );

    Ok(())
}

/// Three reports compete for two fixes. The losing middle report must become a ghost.
///
/// Fix A at t=0 ms, Fix B at t=3 000 ms.
/// Report R1 at t=100 ms → assigned to A (100 ms distance).
/// Report R2 at t=200 ms → also closest to A (200 ms), loses. Also outside window of B → ghost.
/// Report R3 at t=2 900 ms → assigned to B (100 ms distance).
///
/// Ghost fixes between real fixes carry the bearing heading (not `None`), so we
/// identify ghost vs real by whether a satellite report is present for fixed constellations.
#[test]
fn multiple_contested_losers_all_become_ghosts() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(3000))
            .lat(Angle::degrees(55.1))
            .lon(Angle::degrees(12.1))
            .heading(Angle::degrees(0.0))
            .build(),
    );

    // R1 wins fix A.
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t(100))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );
    // R2 loses fix A, too far from B → must become ghost between A and B.
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t(200))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Galileo)
                    .prn(2u32)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );
    // R3 wins fix B.
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t(2900))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Beidou)
                    .prn(3u32)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // 2 real fixes + 1 ghost for R2 = 3 total.
    assert_eq!(
        points.len(),
        3,
        "R2 must produce exactly one ghost fix (got {} points)",
        points.len()
    );

    // Points are sorted by time: fix A (t=0), ghost-R2 (t≈200ms), fix B (t=3 000ms).
    // Fix A carries R1 (GPS).
    let r1_rep = points[0].satellites.as_ref().expect("fix A must carry R1");
    assert_eq!(r1_rep.tracked[0].constellation, Constellation::Gps);

    // Ghost (middle, t≈200 ms) carries R2 (Galileo).
    let r2_rep = points[1]
        .satellites
        .as_ref()
        .expect("ghost must carry R2 (Galileo)");
    assert_eq!(r2_rep.tracked[0].constellation, Constellation::Galileo);

    // Fix B carries R3 (Beidou).
    let r3_rep = points[2].satellites.as_ref().expect("fix B must carry R3");
    assert_eq!(r3_rep.tracked[0].constellation, Constellation::Beidou);

    Ok(())
}

/// When the GPS fix is lost between two real fixes and neither the fixes nor the
/// satellite reports carry enough time information for delta correction, the builder
/// falls back to evenly distributing ghost nav points along the segment.
///
/// This tests the even-distribution fallback: reports supply only `sys_time` and
/// the surrounding real fixes carry no `sys_time`, so no GPS/system-clock delta
/// can be computed.  Each ghost is placed at equal fractional steps (1/4, 2/4, 3/4)
/// regardless of the raw `sys_time` values.
#[test]
fn ghost_points_between_fixes_are_evenly_distributed() -> Result<(), BuildError> {
    // Real fix A: t=0 s, (lat=10.0, lon=20.0) -- no `sys_time`.
    // Real fix B: t=120 s, (lat=10.0, lon=22.0) -- no `sys_time`.
    // Three satellite reports with `sys_time` only, clustered near B:
    //   sys=110 s, sys=115 s, sys=117 s  (all > 500 ms from both fixes → ghost).
    //
    // Because no delta can be computed, even distribution must place the ghosts
    // at lon = 20.5, 21.0, 21.5 (fractions 1/4, 2/4, 3/4).
    let mut recorder = NavFileBuilder::new().open();

    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(10.0))
            .lon(Angle::degrees(20.0))
            .heading(Angle::degrees(90.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(120_000))
            .lat(Angle::degrees(10.0))
            .lon(Angle::degrees(22.0))
            .heading(Angle::degrees(90.0))
            .build(),
    );

    // `sys_time` only -- no `gps_time`, no nav-fix `sys_time` -- triggers even-distribution fallback.
    for offset_ms in [110_000_i64, 115_000, 117_000] {
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(offset_ms))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(Constellation::Gps)
                        .prn(1u32)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;

    // 2 real fixes + 3 ghost fixes = 5 points total.
    let points = nav_file.nav_points();
    assert_eq!(points.len(), 5, "expected 2 real + 3 ghost nav points");

    // Ghost points are sandwiched between the two real fixes in time order.
    // Fix A (index 0) and Fix B (index 4) bracket ghost points at indices 1-3.
    let ghost_lons: Vec<f64> = points[1..=3]
        .iter()
        .map(|p| p.fix.lon.as_degrees())
        .collect();

    // Even distribution: fractions 1/4, 2/4, 3/4 of the [20.0, 22.0] segment.
    let expected_lons = [20.5_f64, 21.0, 21.5];
    for (i, (&actual, &expected)) in ghost_lons.iter().zip(expected_lons.iter()).enumerate() {
        assert!(
            (actual - expected).abs() < 1e-6,
            "ghost point {i}: expected lon {expected:.4}, got {actual:.4}"
        );
    }

    // All ghost points carry satellite reports.
    for (i, p) in points[1..=3].iter().enumerate() {
        assert!(
            p.satellites.is_some(),
            "ghost point {i} is missing its satellite report"
        );
    }

    Ok(())
}

/// The first ghost fix created after the last real fix must be ~1 m ahead of
/// it (a "fix lost" indicator), not 2 m like subsequent dead-reckoned steps.
#[test]
fn first_ghost_after_last_fix_is_1m_ahead() -> Result<(), BuildError> {
    // Single real fix at lat=0, lon=0, heading=0° (north).
    // One satellite report well outside the window → dead-reckoned ghost.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_satellite_report(simple_report(10_000));

    let nav_file = recorder.finish()?;
    assert_eq!(nav_file.nav_points().len(), 2);

    let ghost = &nav_file.nav_points()[1];
    assert!(ghost.satellites.is_some());
    assert!(
        ghost.fix.heading.is_none(),
        "ghost after last fix must have no heading (circle indicator)"
    );

    // heading=0° means due north. Lat increases by ~1/111_320 degrees per metre.
    // The ghost should be ~1 m north (not 2 m).
    let ghost_lat = ghost.fix.lat.as_degrees();
    let one_metre_deg = 1.0_f64 / 111_320.0; // rough but sufficient
    let two_metre_deg = 2.0_f64 / 111_320.0;
    assert!(
        ghost_lat > 0.0,
        "ghost must be north of the fix (lat > 0), got {ghost_lat}"
    );
    assert!(
        ghost_lat < two_metre_deg * 0.9,
        "ghost is ~2 m away ({ghost_lat:.8}°), expected ~1 m ({one_metre_deg:.8}°)"
    );
    assert!(
        ghost_lat > one_metre_deg * 0.5,
        "ghost is far too close ({ghost_lat:.8}°), expected ~1 m ({one_metre_deg:.8}°)"
    );

    Ok(())
}

/// Orphan satellite reports that arrive before the first real fix cannot be
/// placed on the map (no reference position) and must be silently dropped.
#[test]
fn orphan_reports_before_first_fix_are_dropped() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(10_000)); // fix at t=10 s
    recorder.add_satellite_report(simple_report(0)); // report at t=0, before the fix

    let nav_file = recorder.finish()?;

    // Only the real fix. The pre-fix report is dropped.
    assert_eq!(
        nav_file.nav_points().len(),
        1,
        "pre-fix orphan report must be dropped"
    );
    assert!(nav_file.nav_points()[0].satellites.is_none());

    Ok(())
}

#[test]
fn annotation_interpolation_mid_interval() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(10.0))
            .lon(Angle::degrees(20.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(1000))
            .lat(Angle::degrees(12.0))
            .lon(Angle::degrees(24.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_annotation(Annotation::builder().time(t(500)).label("mid").build()?);
    let nav_file = recorder.finish()?;
    let m = &nav_file.markers()[0];
    assert!((m.lat.as_degrees() - 11.0).abs() < 1e-10);
    assert!((m.lon.as_degrees() - 22.0).abs() < 1e-10);
    Ok(())
}

#[test]
fn annotation_before_first_fix_strict() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(1000));
    recorder.add_annotation(
        Annotation::builder()
            .time(t(0))
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
            .gps_time(t(1000))
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
            .gps_time(t(0))
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
fn annotation_out_of_range_strict_error() {
    // Only annotation errors are returned. Satellite-association issues become ghost fixes.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(simple_fix(0));
    // These two reports are well outside the window → ghost fixes, not errors.
    recorder.add_satellite_report(simple_report(2000));
    recorder.add_satellite_report(simple_report(3000));
    // This annotation is before the first fix → error in strict mode.
    recorder.add_annotation(
        Annotation::builder()
            .time(t(-1000))
            .build()
            .expect("an annotation without a label is accepted"),
    );

    let err = recorder.finish().expect_err("should fail");
    assert!(matches!(
        err,
        BuildError::AnnotationsOutsideRange { count: 1 }
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
        .map(|p| p.fix.gps_time)
        .collect();
    let mut sorted = times.clone();
    sorted.sort();
    assert_eq!(times, sorted);
    assert!(nav_file.nav_points().iter().all(|p| p.satellites.is_some()));
    Ok(())
}

#[test]
fn speed_none_propagates() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .speed(Velocity::meter_per_second(15.0))
            .build(),
    );
    let nav_file = recorder.finish()?;
    assert_eq!(
        nav_file.nav_points()[0]
            .fix
            .speed
            .map(|v| v.as_meters_per_second()),
        Some(15.0)
    );
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
                .map(|p| p.fix.gps_time)
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

/// Every stringlike iterable spells the same component list: a literal array
/// of `&str`, a `vec!` of `&str`, and an owned `Vec<String>` build equal
/// channels.
#[test]
fn channel_components_accept_any_stringlike_iterable() -> Result<(), Box<dyn std::error::Error>> {
    let times = vec![t(0)];
    let values = vec![1.0, 2.0, 3.0];

    let from_array = Channel::builder()
        .name("accel")
        .components(["x", "y", "z"])
        .times(times.clone())
        .values(values.clone())
        .build()?;
    let from_vec_of_str = Channel::builder()
        .name("accel")
        .components(vec!["x", "y", "z"])
        .times(times.clone())
        .values(values.clone())
        .build()?;
    let from_owned = Channel::builder()
        .name("accel")
        .components(vec!["x".to_owned(), "y".to_owned(), "z".to_owned()])
        .times(times)
        .values(values)
        .build()?;

    assert_eq!(from_array, from_vec_of_str);
    assert_eq!(from_array, from_owned);
    assert_eq!(from_array.components(), &["x", "y", "z"]);
    Ok(())
}
