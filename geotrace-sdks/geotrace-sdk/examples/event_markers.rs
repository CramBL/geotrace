//! Write and read back a `.nvd` file containing event markers.
//!
//! Event markers are timed, hierarchical events anchored to the GPS track.
//! Each marker has a slash-separated `variant_path` (e.g. `"power/boot"`)
//! that GeoTrace uses to group and filter events in the Events panel.

use std::{env, error::Error, fs};

use geotrace_sdk::{
    Angle, DateTime, EventMarker, EventMarkerColor, EventMarkerIconChoice, EventMarkerStyle,
    MarkerIcon, NavFileBuilder, NavFix, Utc,
};

fn main() -> Result<(), Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let mut sink = NavFileBuilder::new().open();

    for (ts, lat, lon) in [
        ("2024-06-01T08:00:00Z", 51.5074, -0.1278),
        ("2024-06-01T08:01:00Z", 51.5088, -0.1248),
        ("2024-06-01T08:02:00Z", 51.5103, -0.1217),
    ] {
        sink.add_nav_fix(
            NavFix::builder()
                .gps_time(t(ts))
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .heading(Angle::degrees(90.0))
                .build(),
        );
    }

    sink.add_event_marker(
        EventMarker::builder()
            .variant_path("power/boot")
            .sys_time(t("2024-06-01T08:00:02Z"))
            .annotation("cold start")
            .build()?,
    );
    sink.add_event_marker(
        EventMarker::builder()
            .variant_path("sensor/gps/lock_acquired")
            .sys_time(t("2024-06-01T08:00:20Z"))
            .build()?,
    );

    sink.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/boot")
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Lightning))
            .color(EventMarkerColor::hex("#44BB44"))
            .build(),
    );

    let nav_file = sink.finish()?;

    let path = env::temp_dir().join("geotrace_event_markers_simple.nvd");
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
