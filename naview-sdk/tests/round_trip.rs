use naview_sdk::{Angle, DateTime, Duration, Utc, Velocity, degree, meter_per_second};
use naview_sdk::{
    Annotation, Constellation, MarkerIcon, Meta, NavFile, NavFileBuilder, NavFix, Satellite,
    SatelliteReport,
};

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    let dt = DateTime::from_timestamp(1_748_000_000, 0).expect("valid timestamp");
    dt
}

fn round_trip(nav_file: NavFile) -> Result<NavFile, naview_sdk::Error> {
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

    let mut b = NavFileBuilder::new().with_meta(Meta {
        title: Some("Test trace".into()),
        device: Some("u-blox NEO-M9N".into()),
        notes: Some("round-trip test".into()),
    });

    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::new::<degree>(51.5))
            .lon(Angle::new::<degree>(-0.1))
            .heading(Angle::new::<degree>(270.0))
            .speed(Velocity::new::<meter_per_second>(12.5))
            .build(),
    );
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t1)
            .lat(Angle::new::<degree>(51.6))
            .lon(Angle::new::<degree>(-0.2))
            .heading(Angle::new::<degree>(180.0))
            .speed(Velocity::new::<meter_per_second>(0.0))
            .build(),
    );

    b.add_satellite_report(
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

    b.add_annotation(
        Annotation::builder()
            .time(tmid)
            .label("halfway".to_owned())
            .icon(MarkerIcon::Warning)
            .build(),
    );

    let nav_file = b.finish()?;
    let rt = round_trip(nav_file)?;

    assert_eq!(rt.meta.title.as_deref(), Some("Test trace"));
    assert_eq!(rt.meta.device.as_deref(), Some("u-blox NEO-M9N"));
    assert_eq!(rt.meta.notes.as_deref(), Some("round-trip test"));

    assert_eq!(rt.nav_points().len(), 2);
    let p0 = &rt.nav_points()[0];
    assert_eq!(p0.fix.gps_time, Some(t0));
    assert_eq!(p0.fix.lat.get::<degree>(), 51.5);
    assert_eq!(p0.fix.lon.get::<degree>(), -0.1);
    assert_eq!(p0.fix.heading.map(|h| h.get::<degree>()), Some(270.0));
    assert_eq!(
        p0.fix.speed.map(|v| v.get::<meter_per_second>()),
        Some(12.5)
    );

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
    assert!((m.lat.get::<degree>() - (51.5 + 51.6) / 2.0).abs() < 1e-10);
    assert!((m.lon.get::<degree>() - (-0.1 + -0.2) / 2.0).abs() < 1e-10);

    Ok(())
}

#[test]
fn minimal() -> Result<(), Box<dyn std::error::Error>> {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(base())
            .lat(Angle::new::<degree>(0.0))
            .lon(Angle::new::<degree>(0.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    let rt = round_trip(b.finish()?)?;
    assert_eq!(rt.nav_points().len(), 1);
    assert_eq!(rt.nav_points()[0].fix.speed, None);
    assert!(rt.nav_points()[0].satellites.is_none());
    assert!(rt.markers().is_empty());
    Ok(())
}

#[test]
fn no_satellite_data() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut b = NavFileBuilder::new();
    for i in 0..3 {
        b.add_nav_fix(
            NavFix::builder()
                .gps_time(t0 + Duration::seconds(i64::from(i)))
                .lat(Angle::new::<degree>(0.0))
                .lon(Angle::new::<degree>(0.0))
                .heading(Angle::new::<degree>(0.0))
                .build(),
        );
    }
    let rt = round_trip(b.finish()?)?;
    assert_eq!(rt.nav_points().len(), 3);
    assert!(rt.nav_points().iter().all(|p| p.satellites.is_none()));

    // Re-round-trip to confirm the absent groups survive another write/read cycle.
    let mut bytes = Vec::new();
    round_trip(rt)?.write(&mut bytes)?;
    Ok(())
}

#[test]
fn no_markers() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::new::<degree>(0.0))
            .lon(Angle::new::<degree>(0.0))
            .heading(Angle::new::<degree>(0.0))
            .build(),
    );
    b.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t0)
            .tracked(vec![])
            .build(),
    );
    let rt = round_trip(b.finish()?)?;
    assert!(rt.markers().is_empty());
    assert!(rt.nav_points()[0].satellites.is_some());
    Ok(())
}

#[test]
fn empty_builder() -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = NavFileBuilder::new().finish()?;
    let rt = round_trip(nav_file)?;
    assert!(rt.nav_points().is_empty());
    assert!(rt.markers().is_empty());
    Ok(())
}

#[test]
fn large_file() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut b = NavFileBuilder::new();

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
        b.add_nav_fix(
            NavFix::builder()
                .gps_time(t)
                .lat(Angle::new::<degree>(55.0))
                .lon(Angle::new::<degree>(12.0))
                .heading(Angle::new::<degree>(0.0))
                .build(),
        );
        b.add_satellite_report(
            SatelliteReport::builder()
                .gps_time(t)
                .tracked(tracked.clone())
                .build(),
        );
    }

    let nav_file = b.finish()?;
    assert_eq!(nav_file.nav_points().len(), 50_000);

    let rt = round_trip(nav_file)?;
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
