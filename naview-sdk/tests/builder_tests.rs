use naview_sdk::{Angle, DateTime, Duration, Utc, Velocity, degree, meter_per_second};
use naview_sdk::{
    Annotation, BuildError, Constellation, NavFileBuilder, NavFix, Satellite, SatelliteReport,
};

fn t(offset_ms: i64) -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed base timestamp is always valid")]
    let base = DateTime::from_timestamp(1_748_000_000, 0).expect("valid");
    base + Duration::milliseconds(offset_ms)
}

fn simple_fix(offset_ms: i64) -> NavFix {
    NavFix::builder()
        .gps_time(t(offset_ms))
        .lat(Angle::new::<degree>(55.0))
        .lon(Angle::new::<degree>(12.0))
        .heading(Angle::new::<degree>(0.0))
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

// ─── Satellite association ──────────────────────────────────────────────────

#[test]
fn satellite_association_within_window() -> Result<(), BuildError> {
    // Report 400 ms from fix is within the 500 ms default window.
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(400));
    let nav_file = b.finish()?;
    assert!(nav_file.nav_points()[0].satellites.is_some());
    Ok(())
}

#[test]
fn satellite_outside_window_creates_ghost_fix() -> Result<(), BuildError> {
    // Report 600 ms after the only real fix exceeds the 500 ms window.
    // The builder must create a dead-reckoned ghost fix carrying the report —
    // not return an error.
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(600));
    let nav_file = b.finish()?;

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
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(500));
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
    b.add_satellite_report(later);
    b.add_satellite_report(earlier);

    let nav_file = b.finish()?;
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
    let mut b = NavFileBuilder::new().with_satellite_window(Duration::milliseconds(100));
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(200));
    let nav_file = b.finish()?;
    assert_eq!(nav_file.nav_points().len(), 2);
    assert!(nav_file.nav_points()[0].satellites.is_none());
    assert!(nav_file.nav_points()[1].satellites.is_some());
    Ok(())
}

#[test]
fn satellite_association_narrowed_window_within() -> Result<(), BuildError> {
    // Report at 50 ms is within the 100 ms custom window.
    let mut b = NavFileBuilder::new().with_satellite_window(Duration::milliseconds(100));
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(50));
    let nav_file = b.finish()?;
    assert!(nav_file.nav_points()[0].satellites.is_some());
    Ok(())
}

// ─── Ghost fix interpolation ────────────────────────────────────────────────

/// When the GPS fix is lost between two real fixes and neither the fixes nor the
/// satellite reports carry enough time information for delta correction, the builder
/// falls back to evenly distributing ghost nav points along the segment.
///
/// This tests the even-distribution fallback: reports supply only `sys_time` and
/// the surrounding real fixes carry no `sys_time`, so no GPS/system-clock delta
/// can be computed.  Each ghost is placed at equal fractional steps (1/4, 2/4, 3/4)
/// regardless of the raw sys_time values.
#[test]
fn ghost_points_between_fixes_are_evenly_distributed() -> Result<(), BuildError> {
    // Real fix A: t=0 s, (lat=10.0, lon=20.0) -- no sys_time.
    // Real fix B: t=120 s, (lat=10.0, lon=22.0) -- no sys_time.
    // Three satellite reports with sys_time only, clustered near B:
    //   sys=110 s, sys=115 s, sys=117 s  (all > 500 ms from both fixes → ghost).
    //
    // Because no delta can be computed, even distribution must place the ghosts
    // at lon = 20.5, 21.0, 21.5 (fractions 1/4, 2/4, 3/4).
    let mut b = NavFileBuilder::new();

    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::new::<degree>(10.0))
            .lon(Angle::new::<degree>(20.0))
            .heading(Angle::new::<degree>(90.0))
            .build(),
    );
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(120_000))
            .lat(Angle::new::<degree>(10.0))
            .lon(Angle::new::<degree>(22.0))
            .heading(Angle::new::<degree>(90.0))
            .build(),
    );

    // sys_time only -- no gps_time, no nav-fix sys_time -- triggers even-distribution fallback.
    for offset_ms in [110_000_i64, 115_000, 117_000] {
        b.add_satellite_report(
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

    let nav_file = b.finish()?;

    // 2 real fixes + 3 ghost fixes = 5 points total.
    let points = nav_file.nav_points();
    assert_eq!(points.len(), 5, "expected 2 real + 3 ghost nav points");

    // Ghost points are sandwiched between the two real fixes in time order.
    // Fix A (index 0) and Fix B (index 4) bracket ghost points at indices 1-3.
    let ghost_lons: Vec<f64> = points[1..=3]
        .iter()
        .map(|p| p.fix.lon.get::<degree>())
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
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::new::<degree>(0.0))
            .lon(Angle::new::<degree>(0.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    b.add_satellite_report(simple_report(10_000));

    let nav_file = b.finish()?;
    assert_eq!(nav_file.nav_points().len(), 2);

    let ghost = &nav_file.nav_points()[1];
    assert!(ghost.satellites.is_some());
    assert!(
        ghost.fix.heading.is_none(),
        "ghost after last fix must have no heading (circle indicator)"
    );

    // heading=0° means due north; lat increases by ~1/111_320 degrees per metre.
    // The ghost should be ~1 m north (not 2 m).
    let ghost_lat = ghost.fix.lat.get::<degree>();
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
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(10_000)); // fix at t=10 s
    b.add_satellite_report(simple_report(0)); // report at t=0, before the fix

    let nav_file = b.finish()?;

    // Only the real fix; the pre-fix report produces no ghost.
    assert_eq!(
        nav_file.nav_points().len(),
        1,
        "pre-fix orphan report must be dropped"
    );
    assert!(nav_file.nav_points()[0].satellites.is_none());

    Ok(())
}

// ─── Annotation interpolation ───────────────────────────────────────────────

#[test]
fn annotation_interpolation_mid_interval() -> Result<(), Box<dyn std::error::Error>> {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::new::<degree>(10.0))
            .lon(Angle::new::<degree>(20.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(1000))
            .lat(Angle::new::<degree>(12.0))
            .lon(Angle::new::<degree>(24.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    b.add_annotation(
        Annotation::builder()
            .time(t(500))
            .label("mid".to_owned())
            .build(),
    );
    let nav_file = b.finish()?;
    let m = &nav_file.markers()[0];
    assert!((m.lat.get::<degree>() - 11.0).abs() < 1e-10);
    assert!((m.lon.get::<degree>() - 22.0).abs() < 1e-10);
    Ok(())
}

#[test]
fn annotation_before_first_fix_strict() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(1000));
    b.add_annotation(Annotation::builder().time(t(0)).build());
    assert!(matches!(
        b.finish(),
        Err(BuildError::AnnotationsOutsideRange { count: 1 })
    ));
}

#[test]
fn annotation_before_first_fix_lenient() -> Result<(), Box<dyn std::error::Error>> {
    // Annotation before the first fix is clamped to the first fix position.
    let mut b = NavFileBuilder::new().with_continue_on_error(true);
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(1000))
            .lat(Angle::new::<degree>(55.0))
            .lon(Angle::new::<degree>(12.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    b.add_annotation(Annotation::builder().time(t(0)).build());
    let nav_file = b.finish()?;
    let m = &nav_file.markers()[0];
    assert!((m.lat.get::<degree>() - 55.0).abs() < 1e-10);
    assert!((m.lon.get::<degree>() - 12.0).abs() < 1e-10);
    Ok(())
}

#[test]
fn annotation_after_last_fix_lenient() -> Result<(), Box<dyn std::error::Error>> {
    // Annotation after the last fix is clamped to the last fix position.
    let mut b = NavFileBuilder::new().with_continue_on_error(true);
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::new::<degree>(55.0))
            .lon(Angle::new::<degree>(12.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    b.add_annotation(Annotation::builder().time(t(5000)).build());
    let nav_file = b.finish()?;
    let m = &nav_file.markers()[0];
    assert!((m.lat.get::<degree>() - 55.0).abs() < 1e-10);
    assert!((m.lon.get::<degree>() - 12.0).abs() < 1e-10);
    Ok(())
}

#[test]
fn annotation_out_of_range_strict_error() {
    // Only annotation errors are returned; satellite-association issues become ghost fixes.
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(0));
    // These two reports are well outside the window → ghost fixes, not errors.
    b.add_satellite_report(simple_report(2000));
    b.add_satellite_report(simple_report(3000));
    // This annotation is before the first fix → error in strict mode.
    b.add_annotation(Annotation::builder().time(t(-1000)).build());

    let err = b.finish().expect_err("should fail");
    assert!(matches!(
        err,
        BuildError::AnnotationsOutsideRange { count: 1 }
    ));
}

#[test]
fn no_nav_fixes_with_annotations_lenient() {
    // NoNavFixes is returned even in lenient mode — positions cannot be interpolated at all.
    let mut b = NavFileBuilder::new().with_continue_on_error(true);
    b.add_annotation(Annotation::builder().time(t(0)).build());
    assert!(matches!(b.finish(), Err(BuildError::NoNavFixes)));
}

// ─── General ────────────────────────────────────────────────────────────────

#[test]
fn unsorted_insertion() -> Result<(), BuildError> {
    let mut b = NavFileBuilder::new();
    // Insert in reverse chronological order; finish() must sort correctly.
    for i in (0..5).rev() {
        b.add_nav_fix(simple_fix(i * 1000));
        b.add_satellite_report(simple_report(i * 1000 + 100));
    }
    let nav_file = b.finish()?;
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
#[expect(clippy::float_cmp, reason = "exact round-trip")]
fn speed_none_propagates() -> Result<(), BuildError> {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::new::<degree>(0.0))
            .lon(Angle::new::<degree>(0.0))
            .heading(Angle::new::<degree>(0.0))
            .speed(Velocity::new::<meter_per_second>(15.0))
            .build(),
    );
    let nav_file = b.finish()?;
    assert_eq!(
        nav_file.nav_points()[0]
            .fix
            .speed
            .map(|v| v.get::<meter_per_second>()),
        Some(15.0)
    );
    Ok(())
}
