//! Write and read back a `.gtd` file containing event markers.
//!
//! Event markers are timed, hierarchical events anchored to the GPS track.
//! Each marker has a slash-separated `variant_path` (e.g. `"power/boot"`)
//! that GeoTrace uses to group and filter events in the Events panel.
//! Per-variant styles set an icon and a color. An unlisted variant gets a
//! deterministic fallback color derived from its path.

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
    Angle, DateTime, Duration, EventMarker, EventMarkerIconChoice, EventMarkerStyle, MarkerIcon,
    NavFile, NavFileBuilder, NavFix, NavFixTime, Utc,
};

/// A short London track, one fix every 30 s: second offset, latitude, longitude.
const TRACK: &[(i64, f64, f64)] = &[
    (0, 51.5074, -0.1278),
    (30, 51.5080, -0.1265),
    (60, 51.5088, -0.1248),
    (90, 51.5095, -0.1233),
    (120, 51.5103, -0.1217),
    (150, 51.5110, -0.1200),
];

/// Flat and nested variant paths: second offset, path, annotation.
const EVENTS: &[(i64, &str, Option<&str>)] = &[
    (2, "power/boot", Some("cold start")),
    (5, "connectivity/agps/request", Some("EPO fetch started")),
    (
        18,
        "connectivity/agps/success",
        Some("EPO applied, TTFF reduced"),
    ),
    (20, "sensor/gps/lock_acquired", None),
    (145, "power/sleep", None),
];

fn main() -> Result<(), Box<dyn Error>> {
    let start = "2024-06-01T08:00:00Z".parse::<DateTime<Utc>>()?;

    let mut recorder = NavFileBuilder::new()
        .with_title("Event marker tour")
        .with_device("Example GPS v1.0")
        .open();

    for &(offset_secs, lat, lon) in TRACK {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(start + Duration::seconds(offset_secs)))
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .heading(Angle::degrees(90.0))
                .build(),
        );
    }

    for &(offset_secs, variant_path, annotation) in EVENTS {
        recorder.add(
            EventMarker::builder()
                .variant_path(variant_path)
                .sys_time(start + Duration::seconds(offset_secs))
                .maybe_annotation(annotation)
                .build()?,
        );
    }

    recorder.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/boot")
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Lightning))
            .color("#44BB44")
            .build()?,
    );
    recorder.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/sleep")
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Pin))
            .color("#4488FF")
            .build()?,
    );

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_event_markers.gtd");
    nav_file.write_to_file(&path)?;

    let loaded = NavFile::open(&path)?;
    println!("Nav points: {}", loaded.nav_points().len());
    println!("Event markers: {}", loaded.event_markers().len());
    println!(
        "Event marker styles: {}",
        loaded.event_marker_styles().len()
    );
    for marker in loaded.event_markers() {
        print!(
            "  {}  {:.5}, {:.5}",
            marker.variant_path,
            marker.lat.as_degrees(),
            marker.lon.as_degrees()
        );
        if let Some(annotation) = &marker.annotation {
            print!(" - {annotation}");
        }
        println!();
    }

    fs::remove_file(&path)?;
    Ok(())
}
