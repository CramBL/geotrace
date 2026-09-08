//! Convert GPS data from a CSV file into a `.gtd` GeoTrace data file.
//!
//! **Scenario**: your GPS logger exports fixes as CSV rows.
//! Parse each row, feed them to [`NavRecorder`], then call [`NavRecorder::finish`]
//! to produce a validated file ready for GeoTrace to open.
//!
//! In a real workflow you would replace the inline `CSV_DATA` constant with a
//! `std::fs::read_to_string("track.csv")?` call.
//!
//! Timestamps here are whole Unix epoch seconds to keep the parser tiny.

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

use geotrace_sdk::{Angle, DateTime, NavFileBuilder, NavFix, NavFixTime, Velocity};

const CSV_DATA: &str = "\
timestamp_s,lat,lon,heading_deg,speed_mps
1705309200,51.5074,-0.1278,90.0,12.5
1705309201,51.5075,-0.1276,91.0,12.6
1705309202,51.5076,-0.1274,89.5,12.4
1705309203,51.5077,-0.1272,88.0,12.3
1705309204,51.5078,-0.1270,90.0,12.5
1705309205,51.5079,-0.1268,90.5,12.6
";

fn main() -> Result<(), Box<dyn Error>> {
    let mut recorder = NavFileBuilder::new()
        .with_title("Imported from CSV")
        .with_device("CSV importer v1.0")
        .open();

    let mut rows = 0;
    for line in CSV_DATA.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        let &[timestamp, lat, lon, heading, speed] = cols.as_slice() else {
            eprintln!("Skipping malformed row: {line}");
            continue;
        };
        let time = DateTime::from_timestamp(timestamp.parse::<i64>()?, 0)
            .ok_or("timestamp outside the representable range")?;
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(time))
                .lat(Angle::degrees(lat.parse::<f64>()?))
                .lon(Angle::degrees(lon.parse::<f64>()?))
                .heading(Angle::degrees(heading.parse::<f64>()?))
                .speed(Velocity::meter_per_second(speed.parse::<f64>()?))
                .build(),
        );
        rows += 1;
    }

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_from_csv.gtd");
    nav_file.write_to_file(&path)?;
    println!(
        "Parsed {rows} CSV rows into {} nav points -> {}",
        nav_file.nav_points().len(),
        path.display()
    );

    fs::remove_file(&path)?;
    Ok(())
}
