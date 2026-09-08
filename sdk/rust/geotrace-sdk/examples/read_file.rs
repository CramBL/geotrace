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

use std::path::PathBuf;
use std::{env, error::Error, fs};

use geotrace_sdk::{
    Angle, Annotation, Channel, Constellation, DateTime, Duration, EventMarker, EventMarkerColor,
    EventMarkerIconChoice, EventMarkerStyle, MarkerIcon, NavFile, NavFileBuilder, NavFix,
    NavFixTime, Satellite, SatelliteReport, Unit, Utc, Velocity,
};

fn main() -> Result<(), Box<dyn Error>> {
    // Use the path argument if given, otherwise generate a throwaway file.
    let (path, temp) = match env::args().nth(1) {
        Some(arg) => (PathBuf::from(arg), false),
        None => (write_sample_file()?, true),
    };

    let file = NavFile::open(&path)?;

    let meta = file.meta();
    if let Some(title) = &meta.title {
        println!("Title:  {title}");
    }
    if let Some(device) = &meta.device {
        println!("Device: {device}");
    }

    print_nav_points(&file);
    print_markers(&file);
    print_event_markers(&file);
    print_event_marker_styles(&file);
    print_channels(&file);

    if temp {
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn print_nav_points(file: &NavFile) {
    if file.nav_points().is_empty() {
        return;
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
}

fn print_markers(file: &NavFile) {
    if file.markers().is_empty() {
        return;
    }

    println!("Markers: {}", file.markers().len());
    for (i, marker) in file.markers().iter().enumerate() {
        print!(
            "  [{i}] {:.5}, {:.5}  icon={}",
            marker.lat.as_degrees(),
            marker.lon.as_degrees(),
            marker.annotation.icon().wire_code()
        );
        if let Some(label) = marker.annotation.label() {
            print!(" - {label}");
        }
        println!();
    }
}

fn print_event_markers(file: &NavFile) {
    if file.event_markers().is_empty() {
        return;
    }

    println!("Event markers: {}", file.event_markers().len());
    for (i, marker) in file.event_markers().iter().enumerate() {
        print!("  [{i}] {}", marker.variant_path);
        if let Some(annotation) = &marker.annotation {
            print!(" - {annotation}");
        }
        println!();
    }
}

fn print_event_marker_styles(file: &NavFile) {
    if file.event_marker_styles().is_empty() {
        return;
    }

    println!("Event marker styles: {}", file.event_marker_styles().len());
    for (i, style) in file.event_marker_styles().iter().enumerate() {
        let icon = match style.icon.wire_name() {
            "" => "auto",
            name => name,
        };
        let color = match &style.color {
            EventMarkerColor::Auto => "auto",
            EventMarkerColor::Hex(hex) | EventMarkerColor::Unrecognized(hex) => hex,
        };
        println!("  [{i}] {}  icon={icon}  color={color}", style.variant_path);
    }
}

fn print_channels(file: &NavFile) {
    if file.channels().is_empty() {
        return;
    }

    println!("Channels: {}", file.channels().len());
    for (i, channel) in file.channels().iter().enumerate() {
        print!(
            "  [{i}] {} {} samples",
            channel.name(),
            channel.times().len()
        );
        if let Some(unit) = channel.unit() {
            print!(" [{unit}]");
        }
        if !channel.components().is_empty() {
            print!(" components: {}", channel.components().join(" "));
        }
        println!();
    }
}

/// Write a sample file holding one of every section this example prints, and
/// return its path.
fn write_sample_file() -> Result<PathBuf, Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");
    let start = t("2024-06-01T08:00:00Z");

    let mut recorder = NavFileBuilder::new()
        .with_title("Sample track")
        .with_device("Example GPS v1.0")
        .open();

    for (offset_secs, lat, lon) in [
        (0, 51.5074, -0.1278),
        (30, 51.5088, -0.1248),
        (60, 51.5103, -0.1217),
    ] {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(start + Duration::seconds(offset_secs)))
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .heading(Angle::degrees(90.0))
                .speed(Velocity::meter_per_second(5.5))
                .build(),
        );
    }

    recorder.add(
        SatelliteReport::builder()
            .time(NavFixTime::Receiver(start))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1)
                    .in_fix(true)
                    .elevation(45.0)
                    .azimuth(90.0)
                    .snr(38.0)
                    .build(),
                Satellite::builder()
                    .constellation(Constellation::Galileo)
                    .prn(3)
                    .snr(22.0)
                    .build(),
            ])
            .build(),
    );

    recorder.add(
        Annotation::builder()
            .time(start + Duration::seconds(10))
            .label("Start point")
            .icon(MarkerIcon::Pin)
            .build()?,
    );

    recorder.add(
        EventMarker::builder()
            .variant_path("power/boot")
            .sys_time(start + Duration::seconds(2))
            .annotation("cold start")
            .build()?,
    );

    recorder.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/boot")
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Lightning))
            .color("#44BB44")
            .build()?,
    );

    recorder.add(
        Channel::builder()
            .name("incline")
            .unit(Unit::DEG)
            .times(vec![
                start,
                start + Duration::seconds(30),
                start + Duration::seconds(60),
            ])
            .values(vec![1.0, 1.5, 2.0])
            .build()?,
    );

    let nav_file = recorder.finish()?;
    let path = env::temp_dir().join("geotrace_read_file_sample.gtd");
    nav_file.write_to_file(&path)?;
    Ok(path)
}
