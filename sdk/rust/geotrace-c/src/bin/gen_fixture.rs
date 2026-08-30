//! Generates `sdk/c/tests/fixtures/minimal.gtd` for the C SDK round-trip test.
//!
//! Run via: cargo run -p geotrace-c --bin gen_fixture

#![expect(
    clippy::expect_used,
    reason = "fixture generator binary - panicking on errors is intentional"
)]

use std::env;
use std::path::PathBuf;

use geotrace_sdk::{
    Angle, Constellation, NavFileBuilder, NavFix, Satellite, SatelliteReport, Velocity,
};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let out = PathBuf::from(manifest_dir).join("../../c/tests/fixtures/minimal.gtd");

    let t0 =
        chrono::DateTime::from_timestamp_micros(1_700_000_000_000_000).expect("valid timestamp");

    let t1 =
        chrono::DateTime::from_timestamp_micros(1_700_000_010_000_000).expect("valid timestamp");

    let mut recorder = NavFileBuilder::new()
        .with_title("minimal fixture")
        .with_device("gen_fixture")
        .open();

    recorder.add_nav_fix(NavFix {
        gps_time: Some(t0),
        sys_time: None,
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
        gps_time: Some(t1),
        sys_time: None,
        lat: Angle::degrees(51.5080),
        lon: Angle::degrees(-0.1265),
        heading: Some(Angle::degrees(85.0)),
        speed: Some(Velocity::meter_per_second(5.5)),
        eph_m: Some(2.5),
    });

    let nav_file = recorder.finish().expect("gen_fixture: build failed");

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create fixture dir");
    }

    nav_file.write_to_file(&out).expect("write fixture");
    println!("wrote {}", out.display());
}
