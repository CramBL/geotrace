//! Write and read back a `.nvd` file containing event markers.
//!
//! Event markers are timed, hierarchical events anchored to the GPS track.
//! Each marker has a slash-separated `variant_path` (e.g. `"power/boot"`)
//! that NaView uses to group and filter events in the Events panel.

use std::{env, error::Error, fs};

use naview_sdk::{
    Angle, DateTime, EventMarker, EventMarkerStyle, NavFileBuilder, NavFix, Utc, degree,
};

fn main() -> Result<(), Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let mut builder = NavFileBuilder::new();

    for (ts, lat, lon) in [
        ("2024-06-01T08:00:00Z", 51.5074, -0.1278),
        ("2024-06-01T08:01:00Z", 51.5088, -0.1248),
        ("2024-06-01T08:02:00Z", 51.5103, -0.1217),
    ] {
        builder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(ts))
                .lat(Angle::new::<degree>(lat))
                .lon(Angle::new::<degree>(lon))
                .heading(Angle::new::<degree>(90.0))
                .build(),
        );
    }

    builder.add_event_marker(EventMarker {
        variant_path: "power/boot".into(),
        sys_time: t("2024-06-01T08:00:02Z"),
        annotation: Some("cold start".into()),
    })?;
    builder.add_event_marker(EventMarker {
        variant_path: "sensor/gps/lock_acquired".into(),
        sys_time: t("2024-06-01T08:00:20Z"),
        annotation: None,
    })?;

    builder.add_event_marker_style(EventMarkerStyle {
        variant_path: "power/boot".into(),
        icon_name: "lightning".into(),
        color_hex: "#44BB44".into(),
    });

    let nav_file = builder.finish()?;

    let path = env::temp_dir().join("naview_event_markers_simple.nvd");
    nav_file.write_to_file(&path)?;

    let loaded = naview_sdk::NavFile::open(&path)?;
    println!(
        "{} fixes, {} event markers",
        loaded.nav_points().len(),
        loaded.event_markers().len()
    );
    for em in loaded.event_markers() {
        println!(
            "  {} @ {:.5}, {:.5}",
            em.variant_path,
            em.lat.get::<degree>(),
            em.lon.get::<degree>()
        );
    }

    fs::remove_file(&path)?;
    Ok(())
}
