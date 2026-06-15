//! Open a `.gtd` file and print a summary of its contents.
//!
//! Pass a path on the command line to inspect an existing file:
//!
//! ```text
//! cargo run -p geotrace-sdk --example read_file -- path/to/file.gtd
//! ```
//!
//! With no argument the example first writes a small file to a temp directory
//! and then reads that back, so it is runnable on its own.

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

use geotrace_sdk::{Angle, DateTime, NavFile, NavFileBuilder, NavFix, Utc};

fn main() -> Result<(), Box<dyn Error>> {
    // Use the path argument if given, otherwise generate a throwaway file.
    let (path, temp) = match env::args().nth(1) {
        Some(arg) => (std::path::PathBuf::from(arg), false),
        None => (generate_sample()?, true),
    };

    let file = NavFile::open(&path)?;

    let meta = file.meta();
    if let Some(title) = &meta.title {
        println!("Title:  {title}");
    }
    if let Some(device) = &meta.device {
        println!("Device: {device}");
    }

    println!("Nav points: {}", file.nav_points().len());
    for (i, point) in file.nav_points().iter().enumerate() {
        let fix = &point.fix;
        print!(
            "  [{i}] {:.5}, {:.5}",
            fix.lat.as_degrees(),
            fix.lon.as_degrees()
        );
        if let Some(speed) = fix.speed {
            print!("  {:.1} m/s", speed.as_meters_per_second());
        }
        if let Some(report) = &point.satellites {
            print!("  sats={}", report.tracked.len());
        }
        println!();
    }

    if !file.event_markers().is_empty() {
        println!("Event markers: {}", file.event_markers().len());
        for em in file.event_markers() {
            println!("  {}", em.variant_path);
        }
    }

    if temp {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Write a small sample file to a temp path and return it.
fn generate_sample() -> Result<std::path::PathBuf, Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let mut sink = NavFileBuilder::new().open();
    for (ts, lat, lon) in [
        ("2024-06-01T08:00:00Z", 51.5074, -0.1278),
        ("2024-06-01T08:00:30Z", 51.5088, -0.1248),
        ("2024-06-01T08:01:00Z", 51.5103, -0.1217),
    ] {
        sink.add_nav_fix(
            NavFix::builder()
                .gps_time(t(ts))
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .build(),
        );
    }

    let nav_file = sink.finish()?;
    let path = env::temp_dir().join("geotrace_read_file_sample.gtd");
    nav_file.write_to_file(&path)?;
    Ok(path)
}
