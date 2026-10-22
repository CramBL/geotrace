#![allow(
    clippy::restriction,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "SDK example"
)]

use std::env;
use std::error::Error;

use geotrace_sdk::{
    Angle, Constellation, DateTime, Duration, NavFileBuilder, NavFix, NavFixTime,
    SatelliteCoverage, SatelliteIdentity, SatelliteObservation, SatelliteObservationReport,
    SatelliteScope, SolutionState,
};

fn main() -> Result<(), Box<dyn Error>> {
    let start = DateTime::from_timestamp(1_700_000_000, 0).ok_or("timestamp")?;
    let mut recorder = NavFileBuilder::new().open();
    for second in [0, 10] {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Host(start + Duration::seconds(second)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(second as f64))
                .state(SolutionState::Fix)
                .used_count(8)
                .hdop(0.9)
                .build(),
        );
    }
    recorder.add(
        SatelliteObservationReport::builder()
            .time(NavFixTime::Host(start + Duration::seconds(2)))
            .scope(SatelliteScope::Global)
            .coverage(SatelliteCoverage::Partial)
            .reported_count(12)
            .satellites(vec![
                SatelliteObservation::builder()
                    .identity(SatelliteIdentity::new(Constellation::Gps, 1))
                    .snr(40.0)
                    .build(),
            ])
            .build(),
    );
    recorder
        .finish()?
        .write_to_file(env::temp_dir().join("navigation_polling.gtd"))?;
    Ok(())
}
