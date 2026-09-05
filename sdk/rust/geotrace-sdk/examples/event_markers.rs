//! Write and read back a `.gtd` file containing event markers.
//!
//! Event markers are timed, hierarchical events anchored to the GPS track.
//! Each marker has a slash-separated `variant_path` (e.g. `"power/boot"`)
//! that GeoTrace uses to group and filter events in the Events panel.

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
    Angle, DateTime, EventMarker, EventMarkerIconChoice, EventMarkerStyle, MarkerIcon,
    NavFileBuilder, NavFix, NavFixTime, Utc,
};

fn main() -> Result<(), Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let mut recorder = NavFileBuilder::new().open();

    for (ts, lat, lon) in [
        ("2024-06-01T08:00:00Z", 51.5074, -0.1278),
        ("2024-06-01T08:01:00Z", 51.5088, -0.1248),
        ("2024-06-01T08:02:00Z", 51.5103, -0.1217),
    ] {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(t(ts)))
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .heading(Angle::degrees(90.0))
                .build(),
        );
    }

    recorder.add(
        EventMarker::builder()
            .variant_path("power/boot")
            .sys_time(t("2024-06-01T08:00:02Z"))
            .annotation("cold start")
            .build()?,
    );
    recorder.add(
        EventMarker::builder()
            .variant_path("sensor/gps/lock_acquired")
            .sys_time(t("2024-06-01T08:00:20Z"))
            .build()?,
    );

    recorder.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/boot")
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Lightning))
            .color("#44BB44")
            .build()
            .expect("valid hex color"),
    );

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_event_markers_simple.gtd");
    nav_file.write_to_file(&path)?;

    let loaded = geotrace_sdk::NavFile::open(&path)?;
    println!(
        "{} fixes, {} event markers",
        loaded.nav_points().len(),
        loaded.event_markers().len()
    );
    for em in loaded.event_markers() {
        println!(
            "  {} @ {:.5}, {:.5}",
            em.variant_path,
            em.lat.as_degrees(),
            em.lon.as_degrees()
        );
    }

    fs::remove_file(&path)?;
    Ok(())
}
