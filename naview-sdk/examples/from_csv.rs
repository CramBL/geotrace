//! Convert GPS data from a CSV file into a `.nvd` naview data file.
//!
//! **Scenario**: your GPS logger exports fixes as CSV rows.
//! Parse each row, feed them to [`NavFileBuilder`], then call [`NavFileBuilder::finish`]
//! to produce a validated file ready for NaView to open.
//!
//! In a real workflow you would replace the inline `CSV_DATA` constant with a
//! `std::fs::read_to_string("track.csv")?` call.

use naview_sdk::{
    Angle, DateTime, NavFileBuilder, NavFix, Utc, Velocity, degree, meter_per_second,
};
use std::error::Error;

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
    let mut builder = NavFileBuilder::new();

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
        builder.add_nav_fix(
            NavFix::builder()
                .time(time)
                .lat(Angle::new::<degree>(lat.parse::<f64>()?))
                .lon(Angle::new::<degree>(lon.parse::<f64>()?))
                .heading(Angle::new::<degree>(heading.parse::<f64>()?))
                .speed(Velocity::new::<meter_per_second>(speed.parse::<f64>()?))
                .build(),
        );
    }

    let nav_file = builder.finish()?;

    let out = std::env::temp_dir().join("naview_from_csv.nvd");
    nav_file.write_to_file(&out)?;
    println!("Written {out:?}");
    std::fs::remove_file(&out)?;

    Ok(())
}
