//! Type-safe event markers using `#[derive(EventKind)]`.
//!
//! Define the event taxonomy as a Rust enum hierarchy and let the derive macro
//! produce the slash-separated path strings.  The compiler enforces that every
//! logged event is a known, valid variant, and exhaustive match coverage comes
//! for free.
//!
//! This example uses a three-level hierarchy:
//!
//!   Event::Connectivity(ConnectivityEvent::Agps(AgpsEvent::Request))
//!   → "connectivity/agps/request"
//!
//! ## Automatic notes
//!
//! By default, `#[derive(EventKind)]` uses the `Debug` representation of the
//! event value as the marker note.  Calling `add_event` is therefore enough to
//! get a human-readable note in the file - no separate string is needed.
//! `add_event_with_note` overrides the automatic note for that one instance.
//!
//! Use `#[event_kind(note = none)]` on an enum to opt out entirely, or
//! `#[event_kind(note = display)]` to use the `Display` implementation instead.

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
    Angle, DateTime, EventKind, EventMarkerIconChoice, EventMarkerStyle, MarkerIcon,
    NavFileBuilder, NavFix, Utc,
};

#[derive(Debug, EventKind)]
enum Event {
    Power(PowerEvent),
    Sensor(SensorEvent),
    Connectivity(ConnectivityEvent),
}

#[derive(Debug, EventKind)]
enum PowerEvent {
    Boot,
    Sleep,
    BatteryLow,
}

#[derive(Debug, EventKind)]
enum SensorEvent {
    Gps(GpsEvent),
}

// note = none: connectivity events always carry an explicit context string via
// add_event_with_note, so the Debug representation would be redundant noise.
#[derive(Debug, EventKind)]
#[event_kind(note = none)]
enum ConnectivityEvent {
    Agps(AgpsEvent),
}

#[derive(Debug, EventKind)]
enum GpsEvent {
    LockAcquired,
    #[expect(
        dead_code,
        reason = "full taxonomy; not every variant fires in this example run"
    )]
    LockLost,
}

#[derive(Debug, EventKind)]
enum AgpsEvent {
    Request,
    Success,
    #[expect(
        dead_code,
        reason = "full taxonomy; not every variant fires in this example run"
    )]
    Timeout,
}

fn main() -> Result<(), Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let mut sink = NavFileBuilder::new().open();

    for (ts, lat, lon) in [
        ("2024-06-01T08:00:00Z", 51.5074, -0.1278),
        ("2024-06-01T08:00:30Z", 51.5080, -0.1265),
        ("2024-06-01T08:01:00Z", 51.5088, -0.1248),
        ("2024-06-01T08:01:30Z", 51.5095, -0.1233),
        ("2024-06-01T08:02:00Z", 51.5103, -0.1217),
        ("2024-06-01T08:02:30Z", 51.5110, -0.1200),
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

    // Explicit note overrides the automatic Debug note for this instance.
    sink.add_event_with_note(
        &Event::Power(PowerEvent::Boot),
        t("2024-06-01T08:00:02Z"),
        "cold start",
    );
    // ConnectivityEvent has note = none, so add_event_with_note is required for a note.
    sink.add_event_with_note(
        &Event::Connectivity(ConnectivityEvent::Agps(AgpsEvent::Request)),
        t("2024-06-01T08:00:05Z"),
        "EPO fetch started",
    );
    sink.add_event_with_note(
        &Event::Connectivity(ConnectivityEvent::Agps(AgpsEvent::Success)),
        t("2024-06-01T08:00:18Z"),
        "EPO applied",
    );
    // No note argument - Debug representation is used automatically as the note.
    sink.add_event(
        &Event::Sensor(SensorEvent::Gps(GpsEvent::LockAcquired)),
        t("2024-06-01T08:00:20Z"),
    );
    sink.add_event_with_note(
        &Event::Power(PowerEvent::BatteryLow),
        t("2024-06-01T08:02:10Z"),
        "14%",
    );
    sink.add_event(&Event::Power(PowerEvent::Sleep), t("2024-06-01T08:02:25Z"));

    sink.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path(Event::Power(PowerEvent::Boot).variant_path().unwrap())
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Lightning))
            .color("#44BB44")
            .build()
            .expect("valid hex color"),
    );
    sink.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path(Event::Power(PowerEvent::Sleep).variant_path().unwrap())
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Pin))
            .color("#4488FF")
            .build()
            .expect("valid hex color"),
    );

    let nav_file = sink.finish()?;

    let path = env::temp_dir().join("geotrace_event_markers_typed.gtd");
    nav_file.write_to_file(&path)?;

    let loaded = geotrace_sdk::NavFile::open(&path)?;
    println!(
        "{} fixes, {} event markers",
        loaded.nav_points().len(),
        loaded.event_markers().len()
    );
    println!();
    for em in loaded.event_markers() {
        let note = em.annotation.as_deref().unwrap_or("—");
        println!(
            "  {:<40}  {:.5}, {:.5}  - {note}",
            em.variant_path,
            em.lat.as_degrees(),
            em.lon.as_degrees()
        );
    }

    fs::remove_file(&path)?;
    Ok(())
}
