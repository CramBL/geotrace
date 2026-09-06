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
use hdf5_pure::{AttrValue, FileBuilder};

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
    write_bytes(
        &nav_point_idx_past_the_nav_points(),
        &fixtures.join("nav_point_idx_past_the_nav_points.gtd"),
    );
}

fn write_fixture(nav_file: &NavFile, path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture dir");
    }
    nav_file.write_to_file(path).expect("write fixture");
    println!("wrote {}", path.display());
}

fn write_bytes(bytes: &[u8], path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture dir");
    }
    std::fs::write(path, bytes).expect("write fixture");
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
        time: NavFixTime::Receiver(t0),
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

/// One nav point and one satellite report whose `nav_point_idx` is 5, written
/// through `hdf5_pure`.
fn nav_point_idx_past_the_nav_points() -> Vec<u8> {
    let t0 = 1_700_000_000_000_000u64;

    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));

    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[0])
        .with_shape(&[1]);
    np.create_dataset("gps_time_us")
        .with_u64_data(&[t0])
        .with_shape(&[1]);
    np.create_dataset("lat")
        .with_f64_data(&[51.5074])
        .with_shape(&[1]);
    np.create_dataset("lon")
        .with_f64_data(&[-0.1278])
        .with_shape(&[1]);
    np.create_dataset("heading")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    fb.add_group(np.finish());

    let mut sr = fb.create_group("sat_reports");
    sr.create_dataset("nav_point_idx")
        .with_u64_data(&[5])
        .with_shape(&[1]);
    sr.create_dataset("gps_time_us")
        .with_u64_data(&[t0])
        .with_shape(&[1]);
    fb.add_group(sr.finish());

    let mut ts = fb.create_group("tracked_sats");
    ts.create_dataset("sat_report_idx")
        .with_u64_data(&[0])
        .with_shape(&[1]);
    ts.create_dataset("constellation")
        .with_u8_data(&[0])
        .with_shape(&[1]);
    ts.create_dataset("prn")
        .with_u32_data(&[1])
        .with_shape(&[1]);
    ts.create_dataset("in_fix")
        .with_u8_data(&[1])
        .with_shape(&[1]);
    ts.create_dataset("elevation")
        .with_f32_data(&[45.0])
        .with_shape(&[1]);
    ts.create_dataset("azimuth")
        .with_f32_data(&[90.0])
        .with_shape(&[1]);
    ts.create_dataset("snr")
        .with_f32_data(&[38.0])
        .with_shape(&[1]);
    fb.add_group(ts.finish());

    fb.finish().expect("gen_fixture: build failed")
}
