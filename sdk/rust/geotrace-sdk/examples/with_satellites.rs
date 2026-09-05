//! Write a `.gtd` file that pairs each GPS fix with a satellite report.
//!
//! A [`SatelliteReport`] is a snapshot of every tracked satellite at one
//! instant: its constellation, PRN, whether it contributed to the fix, and
//! signal quality. Reports are matched to the nearest fix, so giving each
//! report the same timestamp as its fix keeps them aligned.
//!
//! The example writes the file, reads it back, and prints the per-fix satellite
//! counts - the data GeoTrace shows in its sky view.

// Examples favour brevity: the core's robustness restriction lints (no
// unwrap/expect/panic/indexing, no std::env::temp_dir) are not enforced on
// demonstration code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::cognitive_complexity,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "SDK example: demonstration code"
)]

use std::{env, error::Error, fs};

use geotrace_sdk::{
    Angle, Constellation, DateTime, NavFile, NavFileBuilder, NavFix, NavFixTime, Satellite,
    SatelliteReport, Utc, Velocity,
};

fn main() -> Result<(), Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let mut recorder = NavFileBuilder::new().open();

    let track = [
        ("2024-06-01T08:00:00Z", 51.5074, -0.1278),
        ("2024-06-01T08:00:01Z", 51.5080, -0.1265),
        ("2024-06-01T08:00:02Z", 51.5088, -0.1248),
        ("2024-06-01T08:00:03Z", 51.5095, -0.1233),
    ];

    for (i, (ts, lat, lon)) in track.iter().enumerate() {
        let time = t(ts);
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(time))
                .lat(Angle::degrees(*lat))
                .lon(Angle::degrees(*lon))
                .heading(Angle::degrees(90.0))
                .speed(Velocity::meter_per_second(5.5))
                .build(),
        );

        // SNR climbs slightly each second as the receiver settles.
        let snr = 36.0 + i as f32;
        recorder.add(
            SatelliteReport::builder()
                .gps_time(time)
                .tracked(vec![
                    Satellite::builder()
                        .constellation(Constellation::Gps)
                        .prn(1)
                        .in_fix(true)
                        .elevation(45.0)
                        .azimuth(90.0)
                        .snr(snr)
                        .build(),
                    Satellite::builder()
                        .constellation(Constellation::Gps)
                        .prn(5)
                        .in_fix(true)
                        .snr(snr - 2.0)
                        .build(),
                    Satellite::builder()
                        .constellation(Constellation::Galileo)
                        .prn(3)
                        .snr(21.0)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_with_satellites.gtd");
    nav_file.write_to_file(&path)?;

    let loaded = NavFile::open(&path)?;
    println!("Nav points: {}", loaded.nav_points().len());
    for (i, point) in loaded.nav_points().iter().enumerate() {
        let (tracked, in_fix) = match &point.satellites {
            Some(report) => (
                report.tracked.len(),
                report.tracked.iter().filter(|s| s.in_fix).count(),
            ),
            None => (0, 0),
        };
        println!("  [{i}] {tracked} tracked, {in_fix} in fix");
    }

    fs::remove_file(&path)?;
    Ok(())
}
