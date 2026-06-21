//! Convert GPS data from a CSV file into a `.gtd` GeoTrace data file.
//!
//! **Scenario**: your GPS logger exports fixes as CSV rows.
//! Parse each row, feed them to [`NavRecorder`], then call [`NavRecorder::finish`]
//! to produce a validated file ready for GeoTrace to open.
//!
//! In a real workflow you would replace the inline `CSV_DATA` constant with a
//! `std::fs::read_to_string("track.csv")?` call.

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

use geotrace_sdk::{Angle, DateTime, NavFileBuilder, NavFix, Utc, Velocity};

const CSV_DATA: &str = "\
timestamp,lat,lon,heading,speed_mps
2024-01-15T09:00:00Z,51.5074,-0.1278,90.0,12.5
2024-01-15T09:00:01Z,51.5075,-0.1276,91.0,12.6
2024-01-15T09:00:02Z,51.5076,-0.1274,89.5,12.4
2024-01-15T09:00:03Z,51.5077,-0.1272,88.0,12.3
2024-01-15T09:00:04Z,51.5078,-0.1270,90.0,12.5
2024-01-15T09:00:05Z,51.5079,-0.1268,90.5,12.6
";

fn main() -> Result<(), Box<dyn Error>> {
    let mut recorder = NavFileBuilder::new().open();

    for line in CSV_DATA.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        let &[timestamp, lat, lon, heading, speed] = cols.as_slice() else {
            eprintln!("Skipping malformed row: {line}");
            continue;
        };
        let time = timestamp.parse::<DateTime<Utc>>()?;
        recorder.add(
            NavFix::builder()
                .gps_time(time)
                .lat(Angle::degrees(lat.parse::<f64>()?))
                .lon(Angle::degrees(lon.parse::<f64>()?))
                .heading(Angle::degrees(heading.parse::<f64>()?))
                .speed(Velocity::meter_per_second(speed.parse::<f64>()?))
                .build(),
        );
    }

    let nav_file = recorder.finish()?;

    let out = env::temp_dir().join("geotrace_from_csv.gtd");
    nav_file.write_to_file(&out)?;
    println!("Written {out:?}");
    fs::remove_file(&out)?;

    Ok(())
}
