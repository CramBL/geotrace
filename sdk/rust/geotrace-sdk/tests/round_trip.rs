#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]
#![expect(clippy::cognitive_complexity, reason = "comprehensive round-trip test")]

use geotrace_sdk::{Angle, ChannelUnit, DateTime, Duration, Unit, Utc, Velocity};
use geotrace_sdk::{
    Annotation, Channel, Constellation, MarkerIcon, Meta, NavFile, NavFileBuilder, NavFix,
    Satellite, SatelliteReport, TravelMode,
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
            .build()?,
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
    assert_eq!(m.annotation.time(), tmid);
    assert_eq!(m.annotation.label(), Some("halfway"));
    assert_eq!(m.annotation.icon(), Some(MarkerIcon::Warning));
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
fn travel_mode_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new()
        .with_travel_mode(TravelMode::Bicycle)
        .open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );
    let nav_file = recorder.finish()?;
    assert_eq!(nav_file.meta().travel_mode, Some(TravelMode::Bicycle));

    let rt = round_trip(&nav_file)?;
    assert_eq!(rt.meta().travel_mode, Some(TravelMode::Bicycle));
    Ok(())
}

/// A wire value outside the known set must survive a read-write round trip
/// verbatim - readers warn about it but never drop it.
#[test]
fn unknown_travel_mode_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let meta = Meta::builder()
        .travel_mode(TravelMode::Unknown("hovercraft".into()))
        .build();
    let mut recorder = NavFileBuilder::new().with_meta(meta).open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );
    let nav_file = recorder.finish()?;

    let rt = round_trip(&nav_file)?;
    assert_eq!(
        rt.meta().travel_mode,
        Some(TravelMode::Unknown("hovercraft".into()))
    );

    let rt2 = round_trip(&rt)?;
    assert_eq!(
        rt2.meta().travel_mode,
        Some(TravelMode::Unknown("hovercraft".into()))
    );
    Ok(())
}

#[test]
fn no_travel_mode_deserialises_as_none() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().with_title("No travel mode").open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );
    let nav_file = recorder.finish()?;

    let rt = round_trip(&nav_file)?;
    assert_eq!(rt.meta().travel_mode, None);
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
            .unit(Unit::DEG)
            .period(Angle::degrees(360.0))
            .times(vec![t0, t1])
            .values(vec![10.0, 350.0])
            .build()?,
    );
    // ...and a linear channel with a unit and description.
    recorder.add_channel(
        Channel::builder()
            .name("accel_mag")
            .unit(Unit::G)
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
    assert_eq!(accel.unit(), Some(&ChannelUnit::from(Unit::G)));
    assert_eq!(accel.period(), None);
    assert_eq!(accel.description(), Some("accelerometer magnitude"));
    assert_eq!(accel.times(), &[t0, t1, t2]);
    assert_eq!(accel.values(), &[0.98, 1.02, 1.15]);

    let tilt = &rt.channels()[1];
    assert_eq!(tilt.name(), "tilt");
    assert_eq!(tilt.unit(), Some(&ChannelUnit::from(Unit::DEG)));
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
    // Uppercase, a space, an empty name, and a leading digit are all rejected;
    // a plain lowercase identifier is accepted.
    for bad in ["Accel Fwd", "accel-fwd", "", "1accel", "Accel"] {
        assert!(
            matches!(
                Channel::builder()
                    .name(bad)
                    .times(vec![base()])
                    .values(vec![1.0])
                    .build(),
                Err(geotrace_sdk::ChannelError::InvalidName { .. })
            ),
            "expected {bad:?} to be rejected"
        );
    }
    Channel::builder()
        .name("accel_fwd2")
        .times(vec![base()])
        .values(vec![1.0])
        .build()
        .expect("a lowercase identifier with digits and underscores is valid");
}

#[test]
fn a_bare_channel_round_trips_with_no_optional_fields() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("raw")
            .times(vec![t0, t0 + Duration::seconds(1)])
            .values(vec![1.0, 2.0])
            .build()?,
    );
    let rt = round_trip(&recorder.finish()?)?;
    let channel = &rt.channels()[0];
    assert_eq!(channel.name(), "raw");
    assert_eq!(channel.unit(), None);
    assert_eq!(channel.period(), None);
    assert_eq!(channel.description(), None);
    Ok(())
}

#[test]
fn an_empty_channel_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("empty")
            .unit(Unit::G)
            .times(vec![])
            .values(vec![])
            .build()?,
    );
    let rt = round_trip(&recorder.finish()?)?;
    let channel = &rt.channels()[0];
    assert_eq!(channel.name(), "empty");
    assert!(channel.times().is_empty());
    assert!(channel.values().is_empty());
    Ok(())
}

#[test]
fn an_invalid_custom_unit_is_rejected() {
    assert!(matches!(
        ChannelUnit::custom("   "),
        Err(geotrace_sdk::UnitParseError::EmptyCustom)
    ));
}

#[test]
fn legacy_invalid_unit_metadata_cannot_be_new_writer_input() {
    let result = Channel::builder()
        .name("legacy")
        .unit(ChannelUnit::from_file_label("bad\nunit"))
        .times(vec![base()])
        .values(vec![1.0])
        .build();

    assert!(matches!(
        result,
        Err(geotrace_sdk::ChannelError::UnwritableUnit { .. })
    ));
}

#[test]
fn channel_period_requires_a_positive_angular_unit() {
    let build = |unit: Option<ChannelUnit>, period: Option<Angle>| {
        Channel::builder()
            .name("bearing")
            .maybe_unit(unit)
            .maybe_period(period)
            .times(vec![base()])
            .values(vec![10.0])
            .build()
    };

    assert!(matches!(
        build(Some(Unit::G.into()), Some(Angle::degrees(360.0))),
        Err(geotrace_sdk::ChannelError::PeriodNeedsAngularUnit { .. })
    ));
    assert!(matches!(
        build(Some(Unit::DEG.into()), Some(Angle::degrees(0.0))),
        Err(geotrace_sdk::ChannelError::InvalidPeriod { .. })
    ));
    build(Some(Unit::DEG.into()), Some(Angle::degrees(360.0)))
        .expect("positive angular period is valid");
}

#[test]
fn a_custom_unit_round_trips_without_scaling_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("shaft_speed")
            .unit(ChannelUnit::custom("rpm")?)
            .times(vec![base()])
            .values(vec![1200.0])
            .build()?,
    );
    let round_tripped = round_trip(&recorder.finish()?)?;
    let unit = round_tripped.channels()[0].unit();
    assert_eq!(
        unit.map(ChannelUnit::kind),
        Some(geotrace_sdk::ChannelUnitKind::Custom)
    );
    assert_eq!(unit.map(ToString::to_string).as_deref(), Some("rpm"));
    assert_eq!(round_tripped.channels()[0].values(), [1200.0]);
    Ok(())
}

#[test]
fn duplicate_channel_names_are_rejected() {
    let mut recorder = NavFileBuilder::new().open();
    for value in [1.0, 2.0] {
        recorder.add_channel(
            Channel::builder()
                .name("accel")
                .times(vec![base()])
                .values(vec![value])
                .build()
                .expect("valid channel"),
        );
    }
    assert!(matches!(
        recorder.finish(),
        Err(geotrace_sdk::BuildError::DuplicateChannelName { name }) if name == "accel"
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
            expected: 1,
            actual: 2,
            ..
        }
    ));
}

#[test]
fn a_vector_channel_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let t1 = t0 + Duration::milliseconds(80);

    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("accel")
            .unit(Unit::G)
            .description("device-frame acceleration")
            .components(["x", "y", "z"])
            // Two samples, row-major: [x0, y0, z0, x1, y1, z1].
            .times(vec![t0, t1])
            .values(vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02])
            .build()?,
    );

    let nav_file = recorder.finish()?;
    let rt = round_trip(&nav_file)?;
    let accel = &rt.channels()[0];

    assert!(accel.is_vector());
    assert_eq!(accel.component_count(), 3);
    assert_eq!(accel.components(), &["x", "y", "z"]);
    assert_eq!(accel.unit(), Some(&ChannelUnit::from(Unit::G)));
    assert_eq!(accel.times(), &[t0, t1]);
    let rows: Vec<Vec<f64>> = accel.rows().map(<[f64]>::to_vec).collect();
    assert_eq!(rows, vec![vec![0.1, 0.2, 0.98], vec![-0.1, 0.3, 1.02]]);

    // The whole file round-trips by value, components and all.
    assert_eq!(rt, nav_file);
    Ok(())
}

#[test]
fn vector_channel_validation() {
    let t = vec![base()];
    // Empty component list.
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(Vec::<String>::new())
            .times(t.clone())
            .values(vec![1.0])
            .build(),
        Err(geotrace_sdk::ChannelError::EmptyComponents { .. })
    ));
    // A component label that is not an identifier.
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(["x", "Y"])
            .times(t.clone())
            .values(vec![1.0, 2.0])
            .build(),
        Err(geotrace_sdk::ChannelError::InvalidComponent { .. })
    ));
    // A repeated component label.
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(["x", "x"])
            .times(t.clone())
            .values(vec![1.0, 2.0])
            .build(),
        Err(geotrace_sdk::ChannelError::DuplicateComponent { .. })
    ));
    // Values are not times × components long (1 sample × 3 components = 3).
    assert!(matches!(
        Channel::builder()
            .name("v")
            .components(["x", "y", "z"])
            .times(t)
            .values(vec![1.0, 2.0])
            .build(),
        Err(geotrace_sdk::ChannelError::LengthMismatch {
            expected: 3,
            actual: 2,
            ..
        })
    ));
}

#[test]
fn a_single_component_vector_channel_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    // A 1-component vector's `components` attribute stores as a single string;
    // it must still read back as a vector, not collapse to a scalar.
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("tilt")
            .unit(Unit::DEG)
            .components(["angle"])
            .times(vec![t0, t0 + Duration::seconds(1)])
            .values(vec![10.0, 20.0])
            .build()?,
    );
    let nav_file = recorder.finish()?;
    let rt = round_trip(&nav_file)?;

    let tilt = &rt.channels()[0];
    assert!(tilt.is_vector());
    assert_eq!(tilt.component_count(), 1);
    assert_eq!(tilt.components(), &["angle"]);
    assert_eq!(rt, nav_file);
    Ok(())
}

#[test]
fn scalar_and_vector_channels_coexist() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("accel")
            .components(["x", "y", "z"])
            .times(vec![t0])
            .values(vec![0.1, 0.2, 0.98])
            .build()?,
    );
    recorder.add_channel(
        Channel::builder()
            .name("temp")
            .unit(ChannelUnit::custom("degc")?)
            .times(vec![t0])
            .values(vec![21.5])
            .build()?,
    );
    let nav_file = recorder.finish()?;
    let rt = round_trip(&nav_file)?;

    // Sorted by name: accel (vector) then temp (scalar).
    assert!(rt.channels()[0].is_vector());
    assert_eq!(rt.channels()[0].component_count(), 3);
    assert!(!rt.channels()[1].is_vector());
    assert_eq!(rt.channels()[1].name(), "temp");
    assert_eq!(rt, nav_file);
    Ok(())
}

#[test]
fn a_vector_channel_preserves_its_period() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("bearing")
            .unit(Unit::DEG)
            .period(Angle::degrees(360.0))
            .components(["fwd", "aft"].map(String::from).to_vec())
            .times(vec![t0])
            .values(vec![10.0, 190.0])
            .build()?,
    );
    let nav_file = recorder.finish()?;
    let rt = round_trip(&nav_file)?;
    let bearing = &rt.channels()[0];
    assert_eq!(bearing.period(), Some(Angle::degrees(360.0)));
    assert_eq!(bearing.components(), &["fwd", "aft"]);
    assert_eq!(rt, nav_file);
    Ok(())
}

#[test]
#[expect(clippy::float_cmp, reason = "round-trip exact bit preservation")]
fn a_vector_channel_preserves_nan_holes() -> Result<(), Box<dyn std::error::Error>> {
    // NaN marks an absent sample per column, and must survive the round-trip.
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_channel(
        Channel::builder()
            .name("accel")
            .components(["x", "y"].map(String::from).to_vec())
            .times(vec![t0, t0 + Duration::seconds(1)])
            .values(vec![1.0, f64::NAN, f64::NAN, 4.0])
            .build()?,
    );
    let rt = round_trip(&recorder.finish()?)?;
    let values = rt.channels()[0].values();
    // NaN != NaN, so compare finiteness and the finite values explicitly.
    assert!(values[0] == 1.0 && values[3] == 4.0);
    assert!(values[1].is_nan() && values[2].is_nan());
    Ok(())
}
