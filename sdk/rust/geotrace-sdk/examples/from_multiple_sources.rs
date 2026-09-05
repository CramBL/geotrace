//! Aggregate data from multiple sources into a single `.gtd` GeoTrace data file.
//!
//! **Scenario**: your GPS unit logs fixes to one file, and a separate system
//! (a test harness, an annotation tool, a sensor log) records named events with
//! timestamps from a different file or database.
//!
//! Both sources are added independently to [`NavRecorder`]. `finish()` sorts
//! everything by time and interpolates each annotation's map position from the
//! surrounding GPS fixes.

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

use std::{error::Error, fs};

use geotrace_sdk::{
    Angle, Annotation, DateTime, MarkerIcon, NavFileBuilder, NavFix, NavFixTime, Utc,
};

// Source 1 - GPS track (lat/lon/heading in degrees, one fix per second).
const GPS_FIXES: &[(&str, f64, f64, f64)] = &[
    ("2024-01-15T09:00:00Z", 51.5074, -0.1278, 90.0),
    ("2024-01-15T09:00:01Z", 51.5075, -0.1276, 91.0),
    ("2024-01-15T09:00:02Z", 51.5076, -0.1274, 89.5),
    ("2024-01-15T09:00:03Z", 51.5077, -0.1272, 88.0),
    ("2024-01-15T09:00:04Z", 51.5078, -0.1270, 90.0),
    ("2024-01-15T09:00:05Z", 51.5079, -0.1268, 90.5),
];

// Source 2 - event annotations from a separate log / annotation system.
const EVENTS: &[(&str, &str, MarkerIcon)] = &[
    ("2024-01-15T09:00:01Z", "Checkpoint A", MarkerIcon::Pin),
    (
        "2024-01-15T09:00:03Z",
        "Speed bump ahead",
        MarkerIcon::Warning,
    ),
    ("2024-01-15T09:00:04Z", "Checkpoint B", MarkerIcon::Check),
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut recorder = NavFileBuilder::new().open();

    for &(timestamp, lat, lon, heading) in GPS_FIXES {
        let time = timestamp.parse::<DateTime<Utc>>()?;
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(time))
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .heading(Angle::degrees(heading))
                .build(),
        );
    }

    for &(timestamp, label, icon) in EVENTS {
        let time = timestamp.parse::<DateTime<Utc>>()?;
        recorder.add(
            Annotation::builder()
                .time(time)
                .label(label)
                .icon(icon)
                .build()?,
        );
    }

    let nav_file = recorder.finish()?;

    let temp_dir = tempfile::tempdir()?;
    let out = temp_dir.path().join("geotrace_from_multiple_sources.gtd");
    nav_file.write_to_file(&out)?;
    println!("Written {out:?}");
    fs::remove_file(&out)?;

    Ok(())
}
