//! Generates the `.gtd` fixtures under `sdk/c/tests/fixtures/` that the C, C++,
//! Python and Rust SDK tests open.
//!
//! Run via: `cargo run -p geotrace-c --bin gen_fixture`

#![expect(
    clippy::expect_used,
    reason = "fixture generator binary - panicking on errors is intentional"
)]

use std::env;
use std::path::{Path, PathBuf};

use geotrace_sdk::{
    Angle, Constellation, DateTime, Duration, EventMarkerColor, EventMarkerIconChoice,
    EventMarkerStyle, NavFile, NavFileBuilder, NavFix, NavFixTime, Satellite, SatelliteReport,
    Velocity,
};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let fixtures = PathBuf::from(manifest_dir).join("../../c/tests/fixtures");
    write_fixture(&minimal(), &fixtures.join("minimal.gtd"));
    write_fixture(
        &out_of_range_values(),
        &fixtures.join("out_of_range_values.gtd"),
    );
    write_fixture(
        &unrecognized_style_values(),
        &fixtures.join("unrecognized_style_values.gtd"),
    );
}

fn write_fixture(nav_file: &NavFile, path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture dir");
    }
    nav_file.write_to_file(path).expect("write fixture");
    println!("wrote {}", path.display());
}

fn minimal() -> NavFile {
    let t0 = DateTime::from_timestamp_micros(1_700_000_000_000_000).expect("valid timestamp");
    let t1 = t0 + Duration::seconds(10);

    let mut recorder = NavFileBuilder::new()
        .with_title("minimal fixture")
        .with_device("gen_fixture")
        .open();

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0),
        lat: Angle::degrees(51.5074),
        lon: Angle::degrees(-0.1278),
        heading: Some(Angle::degrees(90.0)),
        speed: Some(Velocity::meter_per_second(5.0)),
        eph_m: Some(3.0),
    });

    recorder.add_satellite_report(SatelliteReport {
        gps_time: Some(t0),
        sys_time: None,
        tracked: vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(1u32)
                .in_fix(true)
                .maybe_elevation(Some(45.0f32))
                .maybe_azimuth(Some(90.0f32))
                .maybe_snr(Some(38.0f32))
                .build(),
            Satellite::builder()
                .constellation(Constellation::Galileo)
                .prn(3u32)
                .in_fix(false)
                .maybe_elevation(None)
                .maybe_azimuth(None)
                .maybe_snr(Some(22.0f32))
                .build(),
        ],
    });

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t1),
        lat: Angle::degrees(51.5080),
        lon: Angle::degrees(-0.1265),
        heading: Some(Angle::degrees(85.0)),
        speed: Some(Velocity::meter_per_second(5.5)),
        eph_m: Some(2.5),
    });

    recorder.finish().expect("gen_fixture: build failed")
}

fn out_of_range_values() -> NavFile {
    let t0 = DateTime::from_timestamp_micros(1_700_000_000_000_000).expect("valid timestamp");

    let mut recorder = NavFileBuilder::new()
        .with_title("out of range values fixture")
        .with_device("gen_fixture")
        .open();

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0),
        lat: Angle::degrees(f64::NAN),
        lon: Angle::degrees(-0.1278),
        heading: None,
        speed: None,
        eph_m: None,
    });

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0 + Duration::seconds(1)),
        lat: Angle::degrees(91.0),
        lon: Angle::degrees(-0.1278),
        heading: None,
        speed: None,
        eph_m: None,
    });

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0 + Duration::seconds(2)),
        lat: Angle::degrees(51.5074),
        lon: Angle::degrees(-181.0),
        heading: None,
        speed: None,
        eph_m: None,
    });

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0 + Duration::seconds(3)),
        lat: Angle::degrees(51.5074),
        lon: Angle::degrees(-0.1278),
        heading: Some(Angle::degrees(675.0)),
        speed: None,
        eph_m: None,
    });

    recorder.finish().expect("gen_fixture: build failed")
}

/// A file as a newer build would write it: an event marker style with an icon
/// outside the [`MarkerIcon`](geotrace_sdk::MarkerIcon) set, and a color that is
/// not `#RRGGBB`.
fn unrecognized_style_values() -> NavFile {
    let t0 = DateTime::from_timestamp_micros(1_700_000_000_000_000).expect("valid timestamp");

    let mut recorder = NavFileBuilder::new()
        .with_title("unrecognized style values fixture")
        .with_device("gen_fixture")
        .open();

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0),
        lat: Angle::degrees(51.5074),
        lon: Angle::degrees(-0.1278),
        heading: Some(Angle::degrees(90.0)),
        speed: Some(Velocity::meter_per_second(5.0)),
        eph_m: None,
    });

    recorder.add_event_marker_style(EventMarkerStyle {
        variant_path: "power/boot".to_owned(),
        icon: EventMarkerIconChoice::Unrecognized("hovercraft".to_owned()),
        color: EventMarkerColor::Unrecognized("FFAA00".to_owned()),
    });

    recorder.finish().expect("gen_fixture: build failed")
}
