use std::error::Error;
use std::path::Path;

use chrono::{TimeZone as _, Utc};
use geotrace_sdk::{
    Angle, Constellation, NavFile, NavFix, NavFixTime, PositionProvenance, SatelliteCoverage,
    SatelliteIdentity, SatelliteObservation, SatelliteObservationReport, SatelliteReportPlacement,
    SatelliteScope, SolutionState,
};

#[test]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions verify decoded C++ output"
)]
fn cpp_polling_example_preserves_navigation_facts_and_host_orphan_time()
-> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../cpp/build/examples/examples/navigation_polling.gtd");
    if !path.exists() {
        eprintln!("skipping C++ polling output: build and run the navigation_polling example");
        return Ok(());
    }
    let file = NavFile::open(path)?;
    let points = file.nav_points();
    assert_eq!(points.len(), 3);
    for (point, seconds) in [(&points[0], 1_700_000_000), (&points[2], 1_700_000_010)] {
        let time = Utc.timestamp_opt(seconds, 0).single().ok_or("timestamp")?;
        let expected = NavFix::builder()
            .time(NavFixTime::Host(time))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees((seconds - 1_700_000_000) as f64))
            .state(SolutionState::Fix)
            .used_count(8u32)
            .hdop(0.9)
            .build();
        assert_eq!(point.fix, expected);
        assert!(point.satellite_evidence().observations.is_empty());
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
    assert!(carrier.satellites().is_none());
    let expected = SatelliteObservationReport::builder()
        .time(NavFixTime::Host(time))
        .scope(SatelliteScope::Global)
        .coverage(SatelliteCoverage::Partial)
        .reported_count(12u32)
        .placement(SatelliteReportPlacement::new(time, 0))
        .satellites(vec![
            SatelliteObservation::builder()
                .identity(SatelliteIdentity::new(Constellation::Gps, 1))
                .snr(40.0)
                .build(),
        ])
        .build();
    assert_eq!(carrier.satellite_evidence().observations, vec![expected]);
    Ok(())
}
