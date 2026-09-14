//! Generates the `.gtd` fixtures under `sdk/c/tests/fixtures/` that the C, C++,
//! Python and Rust SDK tests open.
//!
//! Run via: `cargo run -p geotrace-c --bin gen_fixture`
//!
//! The committed bytes are the same from any build: every builder here writes
//! `<scrubbed>` as the SDK version and no build commit.

#![expect(
    clippy::expect_used,
    reason = "fixture generator binary - panicking on errors is intentional"
)]

use std::env;
use std::path::{Path, PathBuf};

use geotrace_sdk::{
    Angle, Annotation, AnnotationIcon, Constellation, DateTime, Duration, EventMarkerColor,
    EventMarkerIconChoice, EventMarkerStyle, NavFile, NavFileBuilder, NavFix, NavFixTime,
    NavRecorder, SCRUBBED_SDK_VERSION, SDK_VERSION_ATTR, Satellite, SatelliteReport, TravelMode,
    Velocity,
};
use hdf5_pure::{AttrValue, FileBuilder};

/// The `markers/icon` code of the unrecognized marker icon fixture, outside
/// the 0 to 13 the `MarkerIcon` set covers.
const UNRECOGNIZED_MARKER_ICON_CODE: u8 = 200;

/// The root attribute and the value of each metadata string in `metadata_with_a_nul_byte.gtd`.
const METADATA_WITH_A_NUL_BYTE: [(&str, &str); 5] = [
    ("meta_title", "title\0after"),
    ("meta_device", "device\0after"),
    ("meta_notes", "notes\0after"),
    ("meta_identity", "identity\0after"),
    ("meta_travel_mode", "car\0after"),
];

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
    write_fixture(
        &unrecognized_marker_icon(),
        &fixtures.join("unrecognized_marker_icon.gtd"),
    );
    write_fixture(
        &channel_description_with_a_nul_byte(),
        &fixtures.join("channel_description_with_a_nul_byte.gtd"),
    );
    write_fixture(
        &metadata_with_a_nul_byte(),
        &fixtures.join("metadata_with_a_nul_byte.gtd"),
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

fn fixture_recorder(title: &str) -> NavRecorder {
    NavFileBuilder::new()
        .with_scrubbed_provenance()
        .with_title(title)
        .and_then(|builder| builder.with_device("gen_fixture"))
        .expect("gen_fixture: the title and the device have no nul byte")
        .open()
}

fn minimal() -> NavFile {
    let t0 = DateTime::from_timestamp_micros(1_700_000_000_000_000).expect("valid timestamp");
    let t1 = t0 + Duration::seconds(10);

    let mut recorder = fixture_recorder("minimal fixture");

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

    let mut recorder = fixture_recorder("out of range values fixture");

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

    let mut recorder = fixture_recorder("unrecognized style values fixture");

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

/// A file as a newer build would write it: a map marker with a `markers/icon`
/// code outside the [`MarkerIcon`](geotrace_sdk::MarkerIcon) set.
fn unrecognized_marker_icon() -> NavFile {
    let t0 = DateTime::from_timestamp_micros(1_700_000_000_000_000).expect("valid timestamp");

    let mut recorder = fixture_recorder("unrecognized marker icon fixture");

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0),
        lat: Angle::degrees(51.5074),
        lon: Angle::degrees(-0.1278),
        heading: Some(Angle::degrees(90.0)),
        speed: Some(Velocity::meter_per_second(5.0)),
        eph_m: None,
    });

    recorder.add_nav_fix(NavFix {
        time: NavFixTime::Receiver(t0 + Duration::seconds(10)),
        lat: Angle::degrees(51.5080),
        lon: Angle::degrees(-0.1265),
        heading: Some(Angle::degrees(85.0)),
        speed: Some(Velocity::meter_per_second(5.5)),
        eph_m: None,
    });

    recorder.add_annotation(
        Annotation::builder()
            .time(t0 + Duration::seconds(5))
            .label("hovercraft")
            .icon(AnnotationIcon::Unrecognized(UNRECOGNIZED_MARKER_ICON_CODE))
            .build()
            .expect("gen_fixture: annotation label fits the field"),
    );

    recorder.finish().expect("gen_fixture: build failed")
}

/// A channel whose description has a nul byte at offset 6. The channel builder rejects that
/// description: `hdf5_pure` writes the file, and the SDK reads it back. [`write_fixture`] writes
/// the result in the layout of the SDK writer.
fn channel_description_with_a_nul_byte() -> NavFile {
    let t0_micros: i64 = 1_700_000_000_000_000;

    let mut fb = file_builder_with_one_nav_point(t0_micros);
    fb.set_attr(
        "meta_title",
        AttrValue::String("channel description with a nul byte fixture".into()),
    );
    fb.set_attr("meta_device", AttrValue::String("gen_fixture".into()));

    let mut channels = fb.create_group("channels");
    let mut speed = channels.create_group("speed");
    speed.set_attr("description", AttrValue::String("before\0after".into()));
    speed
        .create_dataset("time")
        .with_i64_data(&[t0_micros])
        .with_shape(&[1]);
    speed
        .create_dataset("value")
        .with_f64_data(&[1.0])
        .with_shape(&[1]);
    channels.add_group(speed.finish());
    fb.add_group(channels.finish());

    let bytes = fb.finish().expect("gen_fixture: build failed");
    NavFile::read(bytes.as_slice()).expect("gen_fixture: the SDK reads the file")
}

/// A file with the [`METADATA_WITH_A_NUL_BYTE`] strings. The metadata setters reject each of them:
/// `hdf5_pure` writes the file, and the SDK reads it back. [`write_fixture`] writes the result in
/// the layout of the SDK writer.
fn metadata_with_a_nul_byte() -> NavFile {
    let mut fb = file_builder_with_one_nav_point(1_700_000_000_000_000);
    for (attribute, value) in METADATA_WITH_A_NUL_BYTE {
        fb.set_attr(attribute, AttrValue::String(value.into()));
    }

    let bytes = fb.finish().expect("gen_fixture: build failed");
    let nav_file = NavFile::read(bytes.as_slice()).expect("gen_fixture: the SDK reads the file");
    let meta = nav_file.meta();
    let read_back = [
        meta.title(),
        meta.device(),
        meta.notes(),
        meta.identity(),
        meta.travel_mode().map(TravelMode::name),
    ];
    for ((attribute, written), read) in METADATA_WITH_A_NUL_BYTE.into_iter().zip(read_back) {
        assert_eq!(
            read,
            Some(written),
            "gen_fixture: the SDK reads {attribute} back whole"
        );
    }
    nav_file
}

/// A file with one nav point at `time_micros`, stamped with the scrubbed SDK version.
fn file_builder_with_one_nav_point(time_micros: i64) -> FileBuilder {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));
    fb.set_attr(
        SDK_VERSION_ATTR,
        AttrValue::String(SCRUBBED_SDK_VERSION.into()),
    );

    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[time_micros])
        .with_shape(&[1]);
    np.create_dataset("gps_time_us")
        .with_u64_data(&[time_micros.unsigned_abs()])
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
    fb
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
