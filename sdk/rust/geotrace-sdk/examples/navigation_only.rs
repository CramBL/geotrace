#![allow(
    clippy::restriction,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "SDK example"
)]

use std::env;
use std::error::Error;

use geotrace_sdk::{
    Angle, DateTime, Duration, NavFileBuilder, NavFix, NavFixTime, SolutionState, Velocity,
};

fn main() -> Result<(), Box<dyn Error>> {
    let start = DateTime::from_timestamp(1_792_670_400, 0).ok_or("timestamp")?;
    let mut recorder = NavFileBuilder::new()
        .with_title("Basic navigation recording")?
        .open();
    let first = NavFix::builder()
        .time(NavFixTime::Receiver(start))
        .lat(Angle::degrees(55.67))
        .lon(Angle::degrees(12.56))
        .state(SolutionState::Fix)
        .used_count(8)
        .hdop(0.9)
        .heading(Angle::degrees(75.0))
        .speed(Velocity::meter_per_second(4.2))
        .build();
    recorder.add(first);
    recorder.add(NavFix::status_only(
        NavFixTime::Receiver(start + Duration::seconds(2)),
        SolutionState::NoFix,
    ));
    recorder.add(
        NavFix::builder()
            .time(NavFixTime::Receiver(start + Duration::seconds(5)))
            .lat(Angle::degrees(55.671))
            .lon(Angle::degrees(12.562))
            .state(SolutionState::Fix)
            .used_count(9)
            .build(),
    );
    recorder
        .finish()?
        .write_to_file(env::temp_dir().join("navigation_only.gtd"))?;
    Ok(())
}
