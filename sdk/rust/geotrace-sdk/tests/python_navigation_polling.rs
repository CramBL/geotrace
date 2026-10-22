use std::error::Error;
use std::path::Path;

use chrono::{Duration, TimeZone as _, Utc};
use geotrace_sdk::{
    Angle, Constellation, ConstellationSet, NavFile, NavFix, NavFixTime, PositionProvenance,
    SatelliteCoverage, SatelliteIdentity, SatelliteMembership, SatelliteObservation,
    SatelliteObservationReport, SatelliteReportPlacement, SatelliteScope, SatelliteUsage,
    SatelliteUsageReport, SolutionState, Velocity,
};

#[test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions verify decoded Python output"
)]
fn python_navigation_only_example_preserves_reported_facts() -> Result<(), Box<dyn Error>> {
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python/geotrace-py/build/examples");
    if !directory.join("navigation_only.gtd").exists() {
        eprintln!("skipping Python outputs: run just python-sdk");
        return Ok(());
    }
    let start = Utc
        .timestamp_opt(1_792_670_400, 0)
        .single()
        .ok_or("timestamp")?;
    let file = NavFile::open(directory.join("navigation_only.gtd"))?;
    let expected = NavFix::builder()
        .time(NavFixTime::Receiver(start))
        .lat(Angle::degrees(55.67))
        .lon(Angle::degrees(12.56))
        .state(SolutionState::Fix)
        .used_count(8u32)
        .hdop(0.9)
        .heading(Angle::degrees(75.0))
        .speed(Velocity::meter_per_second(4.2))
        .build();
    let points = file.nav_points();
    assert_eq!(points.len(), 3);
    assert_eq!(points[0].fix, expected);
    let status = points[1].fix;
    assert!(status.lat.as_degrees().is_nan());
    assert!(status.lon.as_degrees().is_nan());
    assert_eq!(
        status.time,
        NavFixTime::Receiver(start + Duration::seconds(2))
    );
    assert_eq!(status.state, Some(SolutionState::NoFix));
    assert_eq!(status.position_provenance, None);
    assert_eq!(status.used_count, None);
    assert_eq!(status.heading, None);
    assert_eq!(status.speed, None);
    assert_eq!(status.eph_m, None);
    assert_eq!((status.hdop, status.vdop, status.pdop), (None, None, None));
    let expected = NavFix::builder()
        .time(NavFixTime::Receiver(start + Duration::seconds(5)))
        .lat(Angle::degrees(55.671))
        .lon(Angle::degrees(12.562))
        .state(SolutionState::Fix)
        .used_count(9u32)
        .build();
    assert_eq!(points[2].fix, expected);
    assert!(
        points
            .iter()
            .all(|point| point.satellite_evidence().usage.is_empty()
                && point.satellite_evidence().observations.is_empty())
    );

    Ok(())
}

#[test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions verify decoded Python output"
)]
fn python_polling_examples_preserve_independent_reports_and_host_time() -> Result<(), Box<dyn Error>>
{
    let directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../python/geotrace-py/build/examples");
    if !directory.join("navigation_both.gtd").exists() {
        eprintln!("skipping Python outputs: run just python-sdk");
        return Ok(());
    }
    for (name, has_usage, has_observations) in [
        ("usage", true, false),
        ("observations", false, true),
        ("both", true, true),
    ] {
        let file = NavFile::open(directory.join(format!("navigation_{name}.gtd")))?;
        let points = file.nav_points();
        assert_eq!(points.len(), 3);
        for (point, seconds) in [(&points[0], 0), (&points[2], 10)] {
            let time = Utc
                .timestamp_opt(1_700_000_000 + seconds, 0)
                .single()
                .ok_or("timestamp")?;
            let expected = NavFix::builder()
                .time(NavFixTime::Host(time))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(seconds as f64))
                .state(SolutionState::Fix)
                .used_count(8u32)
                .hdop(0.9)
                .build();
            assert_eq!(point.fix, expected);
        }
        let carrier = &points[1];
        let time = Utc
            .timestamp_opt(1_700_000_002, 0)
            .single()
            .ok_or("timestamp")?;
        assert_eq!(carrier.fix.time, NavFixTime::Receiver(time));
        assert!((carrier.fix.lon.as_degrees() - 2.0).abs() < 1e-12);
        assert_eq!(
            carrier.fix.position_provenance,
            Some(PositionProvenance::SdkSynthesized)
        );
        assert_eq!(carrier.fix.state, None);
        assert!(carrier.satellites().is_none());
        let identity = SatelliteIdentity::new(Constellation::Gps, 1);
        let usage = SatelliteUsageReport::builder()
            .time(NavFixTime::Host(time))
            .scope(SatelliteScope::Constellations(ConstellationSet::new(
                Constellation::Gps,
                [],
            )))
            .coverage(SatelliteCoverage::Complete)
            .reported_count(8u32)
            .placement(SatelliteReportPlacement::new(time, 0))
            .satellites(vec![
                SatelliteUsage::new(identity, SatelliteMembership::Unknown),
                SatelliteUsage::new(identity, SatelliteMembership::Used),
            ])
            .build();
        let observations = SatelliteObservationReport::builder()
            .time(NavFixTime::Host(time))
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .reported_count(12u32)
            .placement(SatelliteReportPlacement::new(time, u64::from(has_usage)))
            .satellites(vec![
                SatelliteObservation::builder()
                    .identity(identity)
                    .snr(0.0)
                    .build(),
                SatelliteObservation::builder().identity(identity).build(),
            ])
            .build();
        let evidence = carrier.satellite_evidence();
        assert_eq!(
            evidence.usage,
            if has_usage { vec![usage] } else { Vec::new() }
        );
        assert_eq!(
            evidence.observations,
            if has_observations {
                vec![observations]
            } else {
                Vec::new()
            }
        );
    }
    Ok(())
}
