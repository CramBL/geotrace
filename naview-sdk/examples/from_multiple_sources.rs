//! Aggregate data from multiple sources into a single `.nvd` naview data file.
//!
//! **Scenario**: your GPS unit logs fixes to one file, and a separate system
//! (a test harness, an annotation tool, a sensor log) records named events with
//! timestamps from a different file or database.
//!
//! Both sources are added independently to [`NavFileBuilder`]; `finish()` sorts
//! everything by time and interpolates each annotation's map position from the
//! surrounding GPS fixes.

use std::{env, error::Error, fs};

use naview_sdk::{Angle, Annotation, DateTime, MarkerIcon, NavFileBuilder, NavFix, Utc, degree};

// Source 1 — GPS track (lat/lon/heading in degrees, one fix per second).
const GPS_FIXES: &[(&str, f64, f64, f64)] = &[
    ("2024-01-15T09:00:00Z", 51.5074, -0.1278, 90.0),
    ("2024-01-15T09:00:01Z", 51.5075, -0.1276, 91.0),
    ("2024-01-15T09:00:02Z", 51.5076, -0.1274, 89.5),
    ("2024-01-15T09:00:03Z", 51.5077, -0.1272, 88.0),
    ("2024-01-15T09:00:04Z", 51.5078, -0.1270, 90.0),
    ("2024-01-15T09:00:05Z", 51.5079, -0.1268, 90.5),
];

// Source 2 — event annotations from a separate log / annotation system.
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
    let mut builder = NavFileBuilder::new();

    for &(timestamp, lat, lon, heading) in GPS_FIXES {
        let time = timestamp.parse::<DateTime<Utc>>()?;
        builder.add_nav_fix(
            NavFix::builder()
                .gps_time(time)
                .lat(Angle::new::<degree>(lat))
                .lon(Angle::new::<degree>(lon))
                .heading(Angle::new::<degree>(heading))
                .build(),
        );
    }

    for &(timestamp, label, icon) in EVENTS {
        let time = timestamp.parse::<DateTime<Utc>>()?;
        builder.add_annotation(
            Annotation::builder()
                .time(time)
                .label(label.to_owned())
                .icon(icon)
                .build(),
        );
    }

    let nav_file = builder.finish()?;

    let out = env::temp_dir().join("naview_from_multiple_sources.nvd");
    nav_file.write_to_file(&out)?;
    println!("Written {out:?}");
    fs::remove_file(&out)?;

    Ok(())
}
