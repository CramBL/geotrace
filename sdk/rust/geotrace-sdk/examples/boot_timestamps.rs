#![allow(
    clippy::restriction,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "SDK example"
)]

use std::env;
use std::error::Error;

use geotrace_sdk::{
    Angle, BootTime, DateTime, Duration, NavFile, NavFileBuilder, NavFix, NavFixTime,
};

fn main() -> Result<(), Box<dyn Error>> {
    let system_start = DateTime::from_timestamp(1_792_670_400, 0).ok_or("timestamp")?;
    let recorded_pairs = [(0, 60_000_000), (1_000_001, 61_000_000)];
    let mut recorder = NavFileBuilder::new()
        .with_notes("Simulated CLOCK_BOOTTIME readings, including time in suspend")?
        .open();
    for (system_elapsed_us, boot_time_us) in recorded_pairs {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Host(
                    system_start + Duration::microseconds(system_elapsed_us),
                ))
                .boot_time(BootTime::from_micros(boot_time_us)?)
                .lat(Angle::degrees(55.67))
                .lon(Angle::degrees(12.56))
                .build(),
        );
    }
    let path = env::temp_dir().join("boot_timestamps.gtd");
    recorder.finish()?.write_to_file(&path)?;
    for point in NavFile::open(path)?.nav_points() {
        println!(
            "System: {:?}, boot: {:?}µs",
            point.fix.sys_time(),
            point.fix.boot_time.map(BootTime::as_micros)
        );
    }
    Ok(())
}
