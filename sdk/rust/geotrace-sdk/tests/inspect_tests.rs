#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]
#![expect(
    clippy::unwrap_in_result,
    reason = "test code may use expect() for infallible test invariants"
)]

use geotrace_sdk::{Angle, DateTime, Duration, Unit, Utc, Velocity};
use geotrace_sdk::{
    Annotation, Channel, Constellation, MarkerIcon, NavFile, NavFileBuilder, NavFix, NavFixTime,
    Satellite, SatelliteReport,
};

#[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

/// A full inspect render of a file exercising every section: metadata, nav
/// points, satellites, a marker, and both a scalar and a vector channel. The
/// snapshot pins the exact layout, which the earlier substring assertions could
/// not. Timestamps derive from the fixed [`base`], so the output is stable.
#[test]
fn snapshot_inspect_populated_file() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let t1 = t0 + Duration::seconds(1);
    let tmid = t0 + Duration::milliseconds(500);

    let mut recorder = NavFileBuilder::new()
        .with_title("Inspect test")
        .with_device("test-device")
        .with_notes("a populated file")
        .open();

    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t0))
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .heading(Angle::degrees(270.0))
            .speed(Velocity::meter_per_second(10.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t1))
            .lat(Angle::degrees(51.6))
            .lon(Angle::degrees(-0.2))
            .heading(Angle::degrees(180.0))
            .speed(Velocity::meter_per_second(12.5))
            .build(),
    );

    recorder.add_satellite_report(
        SatelliteReport::builder()
            .time(NavFixTime::Receiver(t0))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .snr(35.0f32)
                    .in_fix(true)
                    .build(),
                Satellite::builder()
                    .constellation(Constellation::Galileo)
                    .prn(11u32)
                    .snr(30.0f32)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );

    recorder.add_annotation(
        Annotation::builder()
            .time(tmid)
            .label("midpoint")
            .icon(MarkerIcon::Warning)
            .build()?,
    );

    recorder.add_channel(
        Channel::builder()
            .name("incline")
            .unit(Unit::DEG)
            .times(vec![t0, t1])
            .values(vec![1.5, 2.0])
            .build()?,
    );
    recorder.add_channel(
        Channel::builder()
            .name("accel")
            .unit(Unit::G)
            .components(["x", "y", "z"].map(String::from).to_vec())
            .times(vec![t0, t1])
            .values(vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02])
            .build()?,
    );

    let nav_file = recorder.finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    insta::assert_snapshot!(output);
    Ok(())
}

#[test]
fn empty_file() -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = NavFileBuilder::new().open().finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(output.contains("version 1"), "missing version: {output}");

    Ok(())
}

#[test]
fn file_with_no_satellite_data() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..3i64 {
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(t0 + Duration::seconds(i)))
                .lat(Angle::degrees(55.0))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
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
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t0))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );

    let nav_file = recorder.finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(
        output.contains("Markers") && output.contains("0 records"),
        "expected markers section with '0 records': {output}"
    );

    Ok(())
}

#[test]
fn inspect_reports_no_channels_when_absent() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(base()))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;
    let output = NavFile::inspect(tmp.path())?;
    assert!(
        output.contains("0 channels"),
        "missing zero-channel line: {output}"
    );
    Ok(())
}
