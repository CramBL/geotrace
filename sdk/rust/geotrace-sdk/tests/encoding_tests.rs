#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]
#![expect(
    clippy::unwrap_in_result,
    reason = "test code may use expect() for infallible test invariants"
)]

use geotrace_sdk::{Angle, DateTime, Duration, Utc};
use geotrace_sdk::{
    Annotation, Constellation, Error, MarkerIcon, NavFile, NavFileBuilder, NavFix, Satellite,
    SatelliteReport,
};
use hdf5_pure::{AttrValue, FileBuilder};

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    let dt = DateTime::from_timestamp(1_748_000_000, 0).expect("valid");
    dt
}

fn t(offset_ms: i64) -> DateTime<Utc> {
    base() + Duration::milliseconds(offset_ms)
}

fn to_bytes(nav_file: &NavFile) -> Vec<u8> {
    let mut bytes = Vec::new();
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    nav_file.write(&mut bytes).expect("write");
    bytes
}

#[test]
fn nan_for_absent_speed() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(base())
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let bytes = to_bytes(&nav_file);

    let file = hdf5_pure::File::from_bytes(bytes)?;
    let speeds = file.group("nav_points")?.dataset("speed_mps")?.read_f64()?;
    assert!(speeds[0].is_nan());

    let mut bytes2 = Vec::new();
    nav_file.write(&mut bytes2)?;
    let rt = NavFile::read(bytes2.as_slice())?;
    assert_eq!(rt.nav_points()[0].fix.speed, None);
    Ok(())
}

#[test]
fn nan_for_absent_satellite_fields() -> Result<(), Box<dyn std::error::Error>> {
    // elevation, azimuth, snr all None → NaN on disk, None on read-back
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(base())
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(base())
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .build(),
            ])
            .build(),
    );
    let nav_file = recorder.finish()?;
    let bytes = to_bytes(&nav_file);

    let file = hdf5_pure::File::from_bytes(bytes)?;
    let ts = file.group("tracked_sats")?;
    assert!(ts.dataset("elevation")?.read_f32()?[0].is_nan());
    assert!(ts.dataset("azimuth")?.read_f32()?[0].is_nan());
    assert!(ts.dataset("snr")?.read_f32()?[0].is_nan());

    let mut bytes2 = Vec::new();
    nav_file.write(&mut bytes2)?;
    let rt = NavFile::read(bytes2.as_slice())?;
    let sat = rt.nav_points()[0]
        .satellites
        .as_ref()
        .ok_or("no satellites")?;
    assert_eq!(sat.tracked[0].elevation, None);
    assert_eq!(sat.tracked[0].azimuth, None);
    assert_eq!(sat.tracked[0].snr, None);
    Ok(())
}

#[test]
fn constellation_encoding() -> Result<(), Box<dyn std::error::Error>> {
    // Each Constellation variant encodes to its documented u8 code and round-trips.
    let constellations = [
        (Constellation::Gps, 0u8),
        (Constellation::Glonass, 1),
        (Constellation::Galileo, 2),
        (Constellation::Beidou, 3),
    ];

    for (constellation, expected_code) in constellations {
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(base())
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .gps_time(base())
                .tracked(vec![
                    Satellite::builder()
                        .constellation(constellation)
                        .prn(1u32)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
        let nav_file = recorder.finish()?;
        let bytes = to_bytes(&nav_file);

        let file = hdf5_pure::File::from_bytes(bytes)?;
        let codes = file
            .group("tracked_sats")?
            .dataset("constellation")?
            .read_u8()?;
        assert_eq!(codes[0], expected_code);

        let mut bytes2 = Vec::new();
        nav_file.write(&mut bytes2)?;
        let rt = NavFile::read(bytes2.as_slice())?;
        assert_eq!(
            rt.nav_points()[0]
                .satellites
                .as_ref()
                .ok_or("missing")?
                .tracked[0]
                .constellation,
            constellation
        );
    }
    Ok(())
}

#[test]
fn unknown_constellation_on_read() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = make_file_with_invalid_constellation(99);
    let err = NavFile::read(bytes.as_slice()).expect_err("should fail");
    assert!(matches!(err, Error::UnknownConstellation { code: 99, .. }));
    Ok(())
}

fn make_file_with_invalid_constellation(code: u8) -> Vec<u8> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));

    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[0])
        .with_shape(&[1]);
    np.create_dataset("lat")
        .with_f64_data(&[0.0])
        .with_shape(&[1]);
    np.create_dataset("lon")
        .with_f64_data(&[0.0])
        .with_shape(&[1]);
    np.create_dataset("heading")
        .with_f64_data(&[0.0])
        .with_shape(&[1]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    fb.add_group(np.finish());

    let mut sr = fb.create_group("sat_reports");
    sr.create_dataset("nav_point_idx")
        .with_u64_data(&[0])
        .with_shape(&[1]);
    sr.create_dataset("time")
        .with_i64_data(&[0])
        .with_shape(&[1]);
    fb.add_group(sr.finish());

    let mut ts = fb.create_group("tracked_sats");
    ts.create_dataset("sat_report_idx")
        .with_u64_data(&[0])
        .with_shape(&[1]);
    ts.create_dataset("constellation")
        .with_u8_data(&[code])
        .with_shape(&[1]);
    ts.create_dataset("prn")
        .with_u32_data(&[1])
        .with_shape(&[1]);
    ts.create_dataset("in_fix")
        .with_u8_data(&[0])
        .with_shape(&[1]);
    ts.create_dataset("elevation")
        .with_f32_data(&[f32::NAN])
        .with_shape(&[1]);
    ts.create_dataset("azimuth")
        .with_f32_data(&[f32::NAN])
        .with_shape(&[1]);
    ts.create_dataset("snr")
        .with_f32_data(&[f32::NAN])
        .with_shape(&[1]);
    fb.add_group(ts.finish());

    #[expect(clippy::expect_used, reason = "test helper, panics are ok")]
    fb.finish().expect("build hdf5")
}

#[test]
fn marker_icon_encoding() -> Result<(), Box<dyn std::error::Error>> {
    // Each MarkerIcon variant encodes to its documented u8 code and round-trips.
    let icons = [
        (MarkerIcon::Pin, 0u8),
        (MarkerIcon::Cross, 1),
        (MarkerIcon::Circle, 2),
        (MarkerIcon::Lightning, 3),
        (MarkerIcon::Warning, 4),
        (MarkerIcon::Error, 5),
        (MarkerIcon::Check, 6),
    ];

    for (icon, expected_code) in icons {
        let mut recorder = NavFileBuilder::new().open();
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(0))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(1000))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_annotation(Annotation::builder().time(t(500)).icon(icon).build());
        let nav_file = recorder.finish()?;
        let bytes = to_bytes(&nav_file);

        let file = hdf5_pure::File::from_bytes(bytes)?;
        let codes = file.group("markers")?.dataset("icon")?.read_u8()?;
        assert_eq!(
            codes[0], expected_code,
            "icon {icon:?} should be code {expected_code}"
        );

        let mut bytes2 = Vec::new();
        nav_file.write(&mut bytes2)?;
        let rt = NavFile::read(bytes2.as_slice())?;
        assert_eq!(rt.markers()[0].annotation.icon, Some(icon));
    }
    Ok(())
}

#[test]
fn label_ascii() -> Result<(), Box<dyn std::error::Error>> {
    let rt = nav_file_with_label(Some("Hello, world!".into()))?;
    assert_eq!(
        rt.markers()[0].annotation.label.as_deref(),
        Some("Hello, world!")
    );
    Ok(())
}

#[test]
fn label_multibyte_utf8() -> Result<(), Box<dyn std::error::Error>> {
    let label = "日本語テスト 🌍";
    let rt = nav_file_with_label(Some(label.into()))?;
    assert_eq!(rt.markers()[0].annotation.label.as_deref(), Some(label));
    Ok(())
}

#[test]
fn label_none() -> Result<(), Box<dyn std::error::Error>> {
    // An all-zero label row decodes as None.
    let rt = nav_file_with_label(None)?;
    assert_eq!(rt.markers()[0].annotation.label, None);
    Ok(())
}

#[test]
fn label_truncation() -> Result<(), Box<dyn std::error::Error>> {
    // Labels longer than 255 bytes are truncated. The truncated attribute is set.
    let long_label: String = "A".repeat(300);
    let nav_file = build_nav_file_with_label(Some(long_label))?;
    let bytes = to_bytes(&nav_file);

    let file = hdf5_pure::File::from_bytes(bytes)?;
    let attrs = file.group("markers")?.dataset("label")?.attrs()?;
    // hdf5-pure reads all signed integer attrs as I64 regardless of write type.
    let truncated = match attrs.get("truncated") {
        Some(AttrValue::I32(v)) => i64::from(*v),
        Some(AttrValue::I64(v)) => *v,
        _ => 0,
    };
    assert_eq!(truncated, 1);

    let mut bytes2 = Vec::new();
    nav_file.write(&mut bytes2)?;
    let rt = NavFile::read(bytes2.as_slice())?;
    let label = rt.markers()[0].annotation.label.as_deref().unwrap_or("");
    assert_eq!(label.len(), 255);
    assert!(label.chars().all(|c| c == 'A'));
    Ok(())
}

#[test]
fn timestamp_precision() -> Result<(), Box<dyn std::error::Error>> {
    // Timestamps with sub-millisecond precision survive the round-trip.
    let t = DateTime::from_timestamp_micros(1_748_000_000_000_500).expect("valid");

    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t)
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes)?;
    let rt = NavFile::read(bytes.as_slice())?;
    assert_eq!(rt.nav_points()[0].fix.gps_time, Some(t));
    Ok(())
}

#[test]
fn version_rejection() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = make_file_with_version("99");
    let err = NavFile::read(bytes.as_slice()).expect_err("should reject unknown version");
    assert!(matches!(err, Error::UnsupportedVersion { version } if version == "99"));
    Ok(())
}

fn make_file_with_version(version: &str) -> Vec<u8> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String(version.into()));
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("lat").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("lon").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("heading")
        .with_f64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[])
        .with_shape(&[0]);
    fb.add_group(np.finish());
    #[expect(clippy::expect_used, reason = "test helper")]
    fb.finish().expect("build")
}

#[test]
fn shape_mismatch_rejection() -> Result<(), Box<dyn std::error::Error>> {
    // nav_points/lat has an extra element. The reader should detect the mismatch.
    let bytes = make_file_with_shape_mismatch();
    let err = NavFile::read(bytes.as_slice()).expect_err("should detect shape mismatch");
    assert!(matches!(
        err,
        Error::ShapeMismatch {
            group: "nav_points",
            ..
        }
    ));
    Ok(())
}

fn make_file_with_shape_mismatch() -> Vec<u8> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[0])
        .with_shape(&[1]);
    np.create_dataset("lat")
        .with_f64_data(&[0.0, 1.0])
        .with_shape(&[2]); // extra element
    np.create_dataset("lon")
        .with_f64_data(&[0.0])
        .with_shape(&[1]);
    np.create_dataset("heading")
        .with_f64_data(&[0.0])
        .with_shape(&[1]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    fb.add_group(np.finish());
    #[expect(clippy::expect_used, reason = "test helper")]
    fb.finish().expect("build")
}

#[test]
fn chunked_fixed_array_large_dataset_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    // hdf5-pure ≤ 0.5.0 could not read datasets whose Fixed Array chunk index
    // used the "paged" variant, triggered when num_chunks > 1024. Using
    // chunk_size=1 with 1025 elements forces this path.
    const N: usize = 1025;
    let data: Vec<f64> = (0..N).map(|i| i as f64).collect();

    let mut fb = FileBuilder::new();
    let mut grp = fb.create_group("data");
    grp.create_dataset("values")
        .with_f64_data(&data)
        .with_shape(&[N as u64])
        .with_chunks(&[1])
        .with_deflate(6);
    fb.add_group(grp.finish());
    let bytes = fb.finish().expect("build");

    let file = hdf5_pure::File::from_bytes(bytes)?;
    let read_back = file.group("data")?.dataset("values")?.read_f64()?;
    assert_eq!(read_back, data);
    Ok(())
}

fn nav_file_with_label(label: Option<String>) -> Result<NavFile, Box<dyn std::error::Error>> {
    let nav_file = build_nav_file_with_label(label)?;
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes)?;
    Ok(NavFile::read(bytes.as_slice())?)
}

fn build_nav_file_with_label(label: Option<String>) -> Result<NavFile, Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(1000))
            .lat(Angle::degrees(1.0))
            .lon(Angle::degrees(1.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    let ann = if let Some(l) = label {
        Annotation::builder().time(t(500)).label(l).build()
    } else {
        Annotation::builder().time(t(500)).build()
    };
    recorder.add_annotation(ann);
    Ok(recorder.finish()?)
}
