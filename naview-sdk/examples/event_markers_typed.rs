//! Event markers using nested Rust enums for type-safe variant paths.
//!
//! The recommended approach for producers: define the event taxonomy as a
//! Rust enum hierarchy and derive the variant path string from it.  The
//! compiler then enforces that every logged event is a known, valid variant,
//! and you get exhaustive match coverage for free.
//!
//! This example uses a three-level hierarchy:
//!
//!   Event::Connectivity(ConnectivityEvent::Agps(AgpsEvent::Request))
//!   → "connectivity/agps/request"

use std::{env, error::Error, fs};

use naview_sdk::{
    Angle, DateTime, EventMarker, EventMarkerStyle, NavFileBuilder, NavFix, Utc, degree,
};

// Level 1 — top-level event categories.
enum Event {
    Power(PowerEvent),
    Sensor(SensorEvent),
    Connectivity(ConnectivityEvent),
}

// Level 2.
enum PowerEvent {
    Boot,
    Sleep,
    BatteryLow,
}

enum SensorEvent {
    Gps(GpsEvent),
}

enum ConnectivityEvent {
    Agps(AgpsEvent),
}

// Level 3.
enum GpsEvent {
    LockAcquired,
    #[expect(
        dead_code,
        reason = "full taxonomy; not every variant fires in this example run"
    )]
    LockLost,
}

enum AgpsEvent {
    Request,
    Success,
    #[expect(
        dead_code,
        reason = "full taxonomy; not every variant fires in this example run"
    )]
    Timeout,
}

impl Event {
    fn variant_path(&self) -> String {
        match self {
            Self::Power(e) => format!("power/{}", e.segment()),
            Self::Sensor(e) => format!("sensor/{}", e.path()),
            Self::Connectivity(e) => format!("connectivity/{}", e.path()),
        }
    }
}

impl PowerEvent {
    fn segment(&self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::Sleep => "sleep",
            Self::BatteryLow => "battery_low",
        }
    }
}

impl SensorEvent {
    fn path(&self) -> String {
        match self {
            Self::Gps(e) => format!("gps/{}", e.segment()),
        }
    }
}

impl GpsEvent {
    fn segment(&self) -> &'static str {
        match self {
            Self::LockAcquired => "lock_acquired",
            Self::LockLost => "lock_lost",
        }
    }
}

impl ConnectivityEvent {
    fn path(&self) -> String {
        match self {
            Self::Agps(e) => format!("agps/{}", e.segment()),
        }
    }
}

impl AgpsEvent {
    fn segment(&self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Success => "success",
            Self::Timeout => "timeout",
        }
    }
}

fn log_event(
    builder: &mut NavFileBuilder,
    event: Event,
    ts: &str,
    note: Option<&str>,
) -> Result<(), naview_sdk::EventMarkerError> {
    builder.add_event_marker(EventMarker {
        variant_path: event.variant_path(),
        sys_time: ts.parse::<DateTime<Utc>>().expect("valid timestamp"),
        annotation: note.map(str::to_owned),
    })?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let mut builder = NavFileBuilder::new();

    for (ts, lat, lon) in [
        ("2024-06-01T08:00:00Z", 51.5074, -0.1278),
        ("2024-06-01T08:00:30Z", 51.5080, -0.1265),
        ("2024-06-01T08:01:00Z", 51.5088, -0.1248),
        ("2024-06-01T08:01:30Z", 51.5095, -0.1233),
        ("2024-06-01T08:02:00Z", 51.5103, -0.1217),
        ("2024-06-01T08:02:30Z", 51.5110, -0.1200),
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

    log_event(
        &mut builder,
        Event::Power(PowerEvent::Boot),
        "2024-06-01T08:00:02Z",
        Some("cold start"),
    )?;
    log_event(
        &mut builder,
        Event::Connectivity(ConnectivityEvent::Agps(AgpsEvent::Request)),
        "2024-06-01T08:00:05Z",
        Some("EPO fetch started"),
    )?;
    log_event(
        &mut builder,
        Event::Connectivity(ConnectivityEvent::Agps(AgpsEvent::Success)),
        "2024-06-01T08:00:18Z",
        Some("EPO applied"),
    )?;
    log_event(
        &mut builder,
        Event::Sensor(SensorEvent::Gps(GpsEvent::LockAcquired)),
        "2024-06-01T08:00:20Z",
        None,
    )?;
    log_event(
        &mut builder,
        Event::Power(PowerEvent::BatteryLow),
        "2024-06-01T08:02:10Z",
        Some("14%"),
    )?;
    log_event(
        &mut builder,
        Event::Power(PowerEvent::Sleep),
        "2024-06-01T08:02:25Z",
        None,
    )?;

    // Style the two power events so they stand out visually.
    builder.add_event_marker_style(EventMarkerStyle {
        variant_path: Event::Power(PowerEvent::Boot).variant_path(),
        icon_name: "lightning".into(),
        color_hex: "#44BB44".into(),
    });
    builder.add_event_marker_style(EventMarkerStyle {
        variant_path: Event::Power(PowerEvent::Sleep).variant_path(),
        icon_name: "pin".into(),
        color_hex: "#4488FF".into(),
    });

    let nav_file = builder.finish()?;

    let path = env::temp_dir().join("naview_event_markers_typed.nvd");
    nav_file.write_to_file(&path)?;

    let loaded = naview_sdk::NavFile::open(&path)?;
    println!(
        "{} fixes, {} event markers",
        loaded.nav_points().len(),
        loaded.event_markers().len()
    );
    println!();
    for em in loaded.event_markers() {
        let note = em.annotation.as_deref().unwrap_or("—");
        println!(
            "  {:<40}  {:.5}, {:.5}  — {note}",
            em.variant_path,
            em.lat.get::<degree>(),
            em.lon.get::<degree>()
        );
    }

    fs::remove_file(&path)?;
    Ok(())
}
