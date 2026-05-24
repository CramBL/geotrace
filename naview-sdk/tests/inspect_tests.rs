use naview_sdk::{Angle, DateTime, Duration, Utc, Velocity, degree, meter_per_second};
use naview_sdk::{
    Annotation, Constellation, MarkerIcon, Meta, NavFile, NavFileBuilder, NavFix, Satellite,
    SatelliteReport,
};

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

#[test]
fn smoke_test_populated_file() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let t1 = t0 + Duration::seconds(1);
    let tmid = t0 + Duration::milliseconds(500);

    let mut b = NavFileBuilder::new().with_meta(Meta {
        title: Some("Inspect test".into()),
        device: Some("test-device".into()),
        notes: None,
    });

    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::new::<degree>(51.5))
            .lon(Angle::new::<degree>(-0.1))
            .heading(Angle::new::<degree>(270.0))
            .speed(Velocity::new::<meter_per_second>(10.0))
            .build(),
    );
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t1)
            .lat(Angle::new::<degree>(51.6))
            .lon(Angle::new::<degree>(-0.2))
            .heading(Angle::new::<degree>(180.0))
            .build(),
    );

    b.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t0)
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .snr(35.0f32)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );

    b.add_annotation(
        Annotation::builder()
            .time(tmid)
            .label("midpoint".to_owned())
            .icon(MarkerIcon::Warning)
            .build(),
    );

    let nav_file = b.finish()?;
    #[expect(clippy::expect_used, reason = "test setup")]
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;

    assert!(output.contains("version 1"), "missing version: {output}");
    assert!(
        output.contains("2 records"),
        "missing nav point count: {output}"
    );
    assert!(
        output.contains("1 records"),
        "missing marker count: {output}"
    );
    assert!(output.contains("Inspect test"), "missing title: {output}");

    Ok(())
}

#[test]
fn empty_file() -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = NavFileBuilder::new().finish()?;
    #[expect(clippy::expect_used, reason = "test setup")]
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(output.contains("version 1"), "missing version: {output}");

    Ok(())
}

#[test]
fn file_with_no_satellite_data() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut b = NavFileBuilder::new();
    for i in 0..3i64 {
        b.add_nav_fix(
            NavFix::builder()
                .gps_time(t0 + Duration::seconds(i))
                .lat(Angle::new::<degree>(55.0))
                .lon(Angle::new::<degree>(12.0))
                .heading(Angle::new::<degree>(0.0))
                .build(),
        );
    }

    let nav_file = b.finish()?;
    #[expect(clippy::expect_used, reason = "test setup")]
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(
        output.contains("Satellite Reports") && output.contains("0 records"),
        "expected satellite section with '0 records': {output}"
    );

    Ok(())
}

#[test]
fn file_with_no_markers() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::new::<degree>(55.0))
            .lon(Angle::new::<degree>(12.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );

    let nav_file = b.finish()?;
    #[expect(clippy::expect_used, reason = "test setup")]
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(
        output.contains("Markers") && output.contains("0 records"),
        "expected markers section with '0 records': {output}"
    );

    Ok(())
}
