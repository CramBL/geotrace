#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]
#![expect(clippy::cognitive_complexity, reason = "comprehensive round-trip test")]

use geotrace_sdk::{Angle, DateTime, Duration, Utc, Velocity};
use geotrace_sdk::{
    Annotation, Channel, Constellation, MarkerIcon, Meta, NavFile, NavFileBuilder, NavFix,
    Satellite, SatelliteReport,
};

#[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid timestamp")
}

fn round_trip(nav_file: &NavFile) -> Result<NavFile, geotrace_sdk::Error> {
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes)?;
    NavFile::read(bytes.as_slice())
}

#[test]
#[expect(clippy::float_cmp, reason = "round-trip exact bit preservation")]
fn all_fields_present() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let t1 = t0 + Duration::seconds(1);
    let tmid = t0 + Duration::milliseconds(500);

    let mut recorder = NavFileBuilder::new()
        .with_title("Test trace")
        .with_device("u-blox NEO-M9N")
        .with_notes("round-trip test")
        .open();

    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .heading(Angle::degrees(270.0))
            .speed(Velocity::meter_per_second(12.5))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t1)
            .lat(Angle::degrees(51.6))
            .lon(Angle::degrees(-0.2))
            .heading(Angle::degrees(180.0))
            .speed(Velocity::meter_per_second(0.0))
            .build(),
    );

    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t0)
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(5u32)
                    .elevation(45.0f32)
                    .azimuth(90.0f32)
                    .snr(32.5f32)
                    .in_fix(true)
                    .build(),
                Satellite::builder()
                    .constellation(Constellation::Galileo)
                    .prn(11u32)
                    .build(),
            ])
            .build(),
    );

    recorder.add_annotation(
        Annotation::builder()
            .time(tmid)
            .label("halfway")
            .icon(MarkerIcon::Warning)
            .build(),
    );

    let nav_file = recorder.finish()?;
    let rt = round_trip(&nav_file)?;

    assert_eq!(rt.meta().title.as_deref(), Some("Test trace"));
    assert_eq!(rt.meta().device.as_deref(), Some("u-blox NEO-M9N"));
    assert_eq!(rt.meta().notes.as_deref(), Some("round-trip test"));

    assert_eq!(rt.nav_points().len(), 2);
    let p0 = &rt.nav_points()[0];
    assert_eq!(p0.fix.gps_time, Some(t0));
    assert_eq!(p0.fix.lat.as_degrees(), 51.5);
    assert_eq!(p0.fix.lon.as_degrees(), -0.1);
    assert_eq!(p0.fix.heading.map(|h| h.as_degrees()), Some(270.0));
    assert_eq!(p0.fix.speed.map(|v| v.as_meters_per_second()), Some(12.5));

    let rep = p0.satellites.as_ref().ok_or("missing satellite report")?;
    assert_eq!(rep.gps_time, Some(t0));
    assert_eq!(rep.tracked.len(), 2);
    let s0 = &rep.tracked[0];
    assert_eq!(s0.constellation, Constellation::Gps);
    assert_eq!(s0.prn, 5);
    assert_eq!(s0.elevation, Some(45.0));
    assert_eq!(s0.azimuth, Some(90.0));
    assert_eq!(s0.snr, Some(32.5));
    let s1 = &rep.tracked[1];
    assert_eq!(s1.constellation, Constellation::Galileo);
    assert_eq!(s1.prn, 11);
    assert_eq!(s1.elevation, None);
    assert_eq!(s1.azimuth, None);
    assert_eq!(s1.snr, None);
    assert!(!s1.in_fix);
    assert_eq!(rep.tracked.iter().filter(|s| s.in_fix).count(), 1);
    assert!(s0.in_fix);

    assert_eq!(rt.markers().len(), 1);
    let m = &rt.markers()[0];
    assert_eq!(m.annotation.time, tmid);
    assert_eq!(m.annotation.label.as_deref(), Some("halfway"));
    assert_eq!(m.annotation.icon, Some(MarkerIcon::Warning));
    assert!((m.lat.as_degrees() - (51.5 + 51.6) / 2.0).abs() < 1e-10);
    assert!((m.lon.as_degrees() - (-0.1 + -0.2) / 2.0).abs() < 1e-10);

    Ok(())
}

#[test]
fn minimal() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(base())
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    let rt = round_trip(&recorder.finish()?)?;
    assert_eq!(rt.nav_points().len(), 1);
    assert_eq!(rt.nav_points()[0].fix.speed, None);
    assert!(rt.nav_points()[0].satellites.is_none());
    assert!(rt.markers().is_empty());
    Ok(())
}

#[test]
fn no_satellite_data() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..3 {
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t0 + Duration::seconds(i64::from(i)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
    }
    let rt = round_trip(&recorder.finish()?)?;
    assert_eq!(rt.nav_points().len(), 3);
    assert!(rt.nav_points().iter().all(|p| p.satellites.is_none()));

    // Re-round-trip to confirm the absent groups survive another write/read cycle.
    let mut bytes = Vec::new();
    round_trip(&rt)?.write(&mut bytes)?;
    Ok(())
}

#[test]
fn no_markers() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t0)
            .tracked(vec![])
            .build(),
    );
    let rt = round_trip(&recorder.finish()?)?;
    assert!(rt.markers().is_empty());
    assert!(rt.nav_points()[0].satellites.is_some());
    Ok(())
}

#[test]
fn empty_builder() -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = NavFileBuilder::new().open().finish()?;
    let rt = round_trip(&nav_file)?;
    assert!(rt.nav_points().is_empty());
    assert!(rt.markers().is_empty());
    Ok(())
}

#[test]
fn large_file() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();

    let tracked: Vec<_> = (0u32..12u32)
        .map(|i| {
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(i + 1)
                .snr(30.0f32)
                .in_fix(true)
                .build()
        })
        .collect();

    for i in 0u32..50_000 {
        let t = t0 + Duration::seconds(i64::from(i));
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t)
                .lat(Angle::degrees(55.0))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .gps_time(t)
                .tracked(tracked.clone())
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    assert_eq!(nav_file.nav_points().len(), 50_000);

    let rt = round_trip(&nav_file)?;
    assert_eq!(rt.nav_points().len(), 50_000);
    assert_eq!(rt.markers().len(), 0);

    assert_eq!(rt.nav_points()[0].fix.gps_time, Some(t0));
    assert_eq!(
        rt.nav_points()[49_999].fix.gps_time,
        Some(t0 + Duration::seconds(49_999))
    );
    let sat_rep = rt.nav_points()[0].satellites.as_ref().ok_or("missing")?;
    assert_eq!(sat_rep.tracked.len(), 12);
    assert_eq!(sat_rep.tracked.iter().filter(|s| s.in_fix).count(), 12);

    Ok(())
}

#[test]
fn identity_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new()
        .with_identity("device-serial-001")
        .open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );
    let nav_file = recorder.finish()?;
    assert_eq!(
        nav_file.meta().identity.as_deref(),
        Some("device-serial-001")
    );

    let rt = round_trip(&nav_file)?;
    assert_eq!(rt.meta().identity.as_deref(), Some("device-serial-001"));
    Ok(())
}

#[test]
fn no_identity_deserialises_as_none() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().with_title("No identity").open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );
    let nav_file = recorder.finish()?;

    let rt = round_trip(&nav_file)?;
    assert_eq!(rt.meta().identity, None);
    Ok(())
}

#[test]
fn identity_via_meta_builder() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let meta = Meta::builder().identity("route-a").build();
    let mut recorder = NavFileBuilder::new().with_meta(meta).open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(48.8))
            .lon(Angle::degrees(2.3))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let rt = round_trip(&nav_file)?;
    assert_eq!(rt.meta().identity.as_deref(), Some("route-a"));
    Ok(())
}

#[test]
fn large_file_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    const N: u32 = 100_000;
    let t0 = base();

    let mut recorder = NavFileBuilder::new().with_title("large file test").open();
    for i in 0..N {
        let t = t0 + Duration::seconds(i as i64);
        let lat = -89.0 + (i as f64 % 178.0);
        let lon = -179.0 + (i as f64 % 358.0);
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t)
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .build(),
        );
    }
    let nav_file = recorder.finish()?;

    assert_eq!(nav_file.nav_points().len(), N as usize);

    let rt = round_trip(&nav_file)?;

    assert_eq!(rt.nav_points().len(), N as usize);

    let first = &rt.nav_points()[0];
    let last = &rt.nav_points()[(N - 1) as usize];

    assert!((first.fix.lat.as_degrees() - (-89.0)).abs() < 1e-9);
    assert!((first.fix.lon.as_degrees() - (-179.0)).abs() < 1e-9);

    let last_lat = -89.0 + ((N - 1) as f64 % 178.0);
    let last_lon = -179.0 + ((N - 1) as f64 % 358.0);
    assert!((last.fix.lat.as_degrees() - last_lat).abs() < 1e-9);
    assert!((last.fix.lon.as_degrees() - last_lon).abs() < 1e-9);

    Ok(())
}

#[test]
fn channels_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let t1 = t0 + Duration::milliseconds(100);
    let t2 = t0 + Duration::milliseconds(200);

    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );

    // Added out of name order to exercise the canonical sort on finish. An
    // angular channel carrying a wrap period and no description...
    recorder.add_channel(
        Channel::builder()
            .name("tilt")
            .unit("deg")
            .period(Angle::degrees(360.0))
            .times(vec![t0, t1])
            .values(vec![10.0, 350.0])
            .build()?,
    );
    // ...and a linear channel with a unit and description.
    recorder.add_channel(
        Channel::builder()
            .name("accel_mag")
            .unit("g")
            .description("accelerometer magnitude")
            .times(vec![t0, t1, t2])
            .values(vec![0.98, 1.02, 1.15])
            .build()?,
    );

    let nav_file = recorder.finish()?;
    let rt = round_trip(&nav_file)?;

    // Restored channels are name-sorted, so accel_mag precedes tilt.
    assert_eq!(rt.channels().len(), 2);

    let accel = &rt.channels()[0];
    assert_eq!(accel.name(), "accel_mag");
    assert_eq!(accel.unit(), Some("g"));
    assert_eq!(accel.period(), None);
    assert_eq!(accel.description(), Some("accelerometer magnitude"));
    assert_eq!(accel.times(), &[t0, t1, t2]);
    assert_eq!(accel.values(), &[0.98, 1.02, 1.15]);

    let tilt = &rt.channels()[1];
    assert_eq!(tilt.name(), "tilt");
    assert_eq!(tilt.unit(), Some("deg"));
    assert_eq!(tilt.period(), Some(Angle::degrees(360.0)));
    assert_eq!(tilt.description(), None);
    assert_eq!(tilt.times(), &[t0, t1]);
    assert_eq!(tilt.values(), &[10.0, 350.0]);

    // The whole file round-trips by value (finish already sorted the channels).
    assert_eq!(rt, nav_file);

    Ok(())
}

#[test]
fn a_file_without_channels_still_reads() -> Result<(), Box<dyn std::error::Error>> {
    // The channels group is absent, so there are simply none.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(base())
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .build(),
    );
    let rt = round_trip(&recorder.finish()?)?;
    assert!(rt.channels().is_empty());
    Ok(())
}

#[test]
fn a_channel_name_must_be_a_lowercase_identifier() {
    let err = Channel::builder()
        .name("Accel Fwd")
        .times(vec![base()])
        .values(vec![1.0])
        .build()
        .expect_err("uppercase and space are invalid");
    assert!(matches!(
        err,
        geotrace_sdk::ChannelError::InvalidName { .. }
    ));
}

#[test]
fn a_channel_rejects_mismatched_lengths() {
    let err = Channel::builder()
        .name("accel")
        .times(vec![base()])
        .values(vec![1.0, 2.0])
        .build()
        .expect_err("two values but one timestamp");
    assert!(matches!(
        err,
        geotrace_sdk::ChannelError::LengthMismatch {
            times: 1,
            values: 2,
            ..
        }
    ));
}
