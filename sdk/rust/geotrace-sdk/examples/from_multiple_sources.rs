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

use std::{env, error::Error, fs};

use geotrace_sdk::{
    Angle, Annotation, DateTime, Duration, MarkerIcon, NavFileBuilder, NavFix, NavFixTime, Utc,
};

/// Source 1, the GPS track, one fix every 10 s: second offset, latitude,
/// longitude, heading.
const GPS_FIXES: &[(i64, f64, f64, f64)] = &[
    (0, 51.5074, -0.1278, 90.0),
    (10, 51.5075, -0.1276, 91.0),
    (20, 51.5076, -0.1274, 89.5),
    (30, 51.5077, -0.1272, 88.0),
    (40, 51.5078, -0.1270, 90.0),
    (50, 51.5079, -0.1268, 90.5),
];

/// Source 2, annotations from a separate log: second offset, label, icon. Their
/// map positions are not supplied - `finish()` interpolates them from the GPS
/// fixes by timestamp.
const ANNOTATIONS: &[(i64, &str, MarkerIcon)] = &[
    (5, "Pothole", MarkerIcon::Warning),
    (15, "Speed camera", MarkerIcon::Circle),
    (25, "Junction", MarkerIcon::Pin),
];

fn main() -> Result<(), Box<dyn Error>> {
    let start = "2024-06-01T08:00:00Z".parse::<DateTime<Utc>>()?;

    let mut recorder = NavFileBuilder::new()
        .with_title("Merged GPS + annotations")
        .with_device("Aggregator v1.0")
        .open();

    for &(offset_secs, lat, lon, heading) in GPS_FIXES {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(start + Duration::seconds(offset_secs)))
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .heading(Angle::degrees(heading))
                .build(),
        );
    }

    for &(offset_secs, label, icon) in ANNOTATIONS {
        recorder.add(
            Annotation::builder()
                .time(start + Duration::seconds(offset_secs))
                .label(label)
                .icon(icon)
                .build()?,
        );
    }

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_from_multiple_sources.gtd");
    nav_file.write_to_file(&path)?;
    println!(
        "Merged {} GPS fixes + {} annotations -> {}",
        nav_file.nav_points().len(),
        nav_file.markers().len(),
        path.display()
    );
    println!("Annotations were interpolated onto the track by timestamp.");

    fs::remove_file(&path)?;
    Ok(())
}
