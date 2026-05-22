use naview_sdk::{Angle, DateTime, Duration, Utc, Velocity, degree, meter_per_second};
use naview_sdk::{
    Annotation, BuildError, Constellation, FixEntry, NavFileBuilder, NavFix, Satellite,
    SatelliteReport,
};

fn t(offset_ms: i64) -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed base timestamp is always valid")]
    let base = DateTime::from_timestamp(1_748_000_000, 0).expect("valid");
    base + Duration::milliseconds(offset_ms)
}

fn simple_fix(offset_ms: i64) -> NavFix {
    NavFix::builder()
        .time(t(offset_ms))
        .lat(Angle::new::<degree>(55.0))
        .lon(Angle::new::<degree>(12.0))
        .heading(Angle::new::<degree>(0.0))
        .build()
}

fn simple_report(offset_ms: i64) -> SatelliteReport {
    SatelliteReport::builder()
        .time(t(offset_ms))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .build(),
        ])
        .fix(vec![
            FixEntry::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .build(),
        ])
        .build()
}

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
fn satellite_association_beyond_window_strict() {
    // Report 600 ms from fix exceeds the 500 ms default window.
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(600));
    let err = b.finish().expect_err("should fail");
    assert!(matches!(
        err,
        BuildError::UnassociatedSatelliteReports { count: 1, .. }
    ));
}

#[test]
fn satellite_association_beyond_window_lenient() -> Result<(), BuildError> {
    // Same setup in lenient mode: report is dropped, no error returned.
    let mut b = NavFileBuilder::new().with_continue_on_error(true);
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(600));
    let nav_file = b.finish()?;
    assert!(nav_file.nav_points()[0].satellites.is_none());
    Ok(())
}

#[test]
fn satellite_association_tie_breaking() -> Result<(), BuildError> {
    // Two reports at +250 ms and -250 ms from the fix at t=500 ms.
    // The earlier report (t=250) must win even when added last.
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(500));
    let later = SatelliteReport::builder()
        .time(t(750))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Glonass)
                .prn(99u32)
                .build(),
        ])
        .fix(vec![])
        .build();
    let earlier = SatelliteReport::builder()
        .time(t(250))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .build(),
        ])
        .fix(vec![])
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
fn satellite_association_narrowed_window() {
    // Report at 200 ms exceeds the 100 ms custom window.
    let mut b = NavFileBuilder::new().with_satellite_window(Duration::milliseconds(100));
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(200));
    assert!(b.finish().is_err());
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

#[test]
fn annotation_interpolation_mid_interval() -> Result<(), Box<dyn std::error::Error>> {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .time(t(0))
            .lat(Angle::new::<degree>(10.0))
            .lon(Angle::new::<degree>(20.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    b.add_nav_fix(
        NavFix::builder()
            .time(t(1000))
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
            .time(t(1000))
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
            .time(t(0))
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
fn no_nav_fixes_with_annotations_lenient() {
    // NoNavFixes is returned even in lenient mode — positions cannot be interpolated at all.
    let mut b = NavFileBuilder::new().with_continue_on_error(true);
    b.add_annotation(Annotation::builder().time(t(0)).build());
    assert!(matches!(b.finish(), Err(BuildError::NoNavFixes)));
}

#[test]
fn unsorted_insertion() -> Result<(), BuildError> {
    let mut b = NavFileBuilder::new();
    // Insert in reverse chronological order; finish() must sort correctly.
    for i in (0..5).rev() {
        b.add_nav_fix(simple_fix(i * 1000));
        b.add_satellite_report(simple_report(i * 1000 + 100));
    }
    let nav_file = b.finish()?;
    let times: Vec<_> = nav_file.nav_points().iter().map(|p| p.fix.time).collect();
    let mut sorted = times.clone();
    sorted.sort();
    assert_eq!(times, sorted);
    assert!(nav_file.nav_points().iter().all(|p| p.satellites.is_some()));
    Ok(())
}

#[test]
fn multiple_errors_accumulate() {
    // Two unassociated reports and one out-of-range annotation accumulate into one error.
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(simple_fix(0));
    b.add_satellite_report(simple_report(2000));
    b.add_satellite_report(simple_report(3000));
    b.add_annotation(Annotation::builder().time(t(-1000)).build());

    let err = b.finish().expect_err("should fail");
    assert!(matches!(
        err,
        BuildError::Multiple {
            unassociated_satellite_reports: 2,
            annotations_outside_range: 1,
            ..
        }
    ));
}

#[test]
#[expect(clippy::float_cmp, reason = "exact round-trip")]
fn speed_none_propagates() -> Result<(), BuildError> {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .time(t(0))
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
