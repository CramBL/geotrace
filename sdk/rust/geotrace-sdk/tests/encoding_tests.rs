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
    Annotation, ChannelUnit, Constellation, Error, EventMarker, MarkerIcon, NavFile,
    NavFileBuilder, NavFix, NavFixTime, Satellite, SatelliteReport,
};
use hdf5_pure::{AttrValue, FileBuilder};
use rstest::rstest;

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
            .time(NavFixTime::Receiver(base()))
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
    // `elevation`, `azimuth` and `snr` all `None` → NaN on disk, `None` on read-back
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(base()))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .time(NavFixTime::Receiver(base()))
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
                .time(NavFixTime::Receiver(base()))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .time(NavFixTime::Receiver(base()))
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
                .time(NavFixTime::Receiver(t(0)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(t(1000)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_annotation(Annotation::builder().time(t(500)).icon(icon).build()?);
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
        assert_eq!(rt.markers()[0].annotation.icon(), icon);
    }
    Ok(())
}

#[test]
fn label_ascii() -> Result<(), Box<dyn std::error::Error>> {
    let rt = nav_file_with_label(Some("Hello, world!".into()))?;
    assert_eq!(rt.markers()[0].annotation.label(), Some("Hello, world!"));
    Ok(())
}

#[test]
fn label_multibyte_utf8() -> Result<(), Box<dyn std::error::Error>> {
    let label = "日本語テスト 🌍";
    let rt = nav_file_with_label(Some(label.into()))?;
    assert_eq!(rt.markers()[0].annotation.label(), Some(label));
    Ok(())
}

#[test]
fn label_none() -> Result<(), Box<dyn std::error::Error>> {
    // An all-zero label row decodes as None.
    let rt = nav_file_with_label(None)?;
    assert_eq!(rt.markers()[0].annotation.label(), None);
    Ok(())
}

#[test]
fn timestamp_precision() -> Result<(), Box<dyn std::error::Error>> {
    // Timestamps with sub-millisecond precision survive the round-trip.
    let t = DateTime::from_timestamp_micros(1_748_000_000_000_500).expect("valid");

    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes)?;
    let rt = NavFile::read(bytes.as_slice())?;
    assert_eq!(rt.nav_points()[0].fix.gps_time(), Some(t));
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

/// h5py and the reference C library write a string attribute as `H5T_STRING`
/// with `STRSIZE = H5T_VARIABLE`.
#[test]
fn variable_length_string_attributes_are_read() -> Result<(), Box<dyn std::error::Error>> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::VarLenString("1".into()));
    fb.set_attr("meta_title", AttrValue::VarLenString("Ride home".into()));

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

    let mut channels = fb.create_group("channels");
    let mut accel = channels.create_group("accel");
    accel.set_attr("unit", AttrValue::VarLenString("g".into()));
    accel.set_attr(
        "components",
        AttrValue::VarLenStringArray(vec!["x".into(), "y".into()]),
    );
    accel
        .create_dataset("time")
        .with_i64_data(&[0])
        .with_shape(&[1]);
    accel
        .create_dataset("value")
        .with_f64_data(&[1.0, 2.0])
        .with_shape(&[1, 2]);
    channels.add_group(accel.finish());
    fb.add_group(channels.finish());

    let nav_file = NavFile::read(fb.finish()?.as_slice())?;
    assert_eq!(nav_file.meta().title.as_deref(), Some("Ride home"));
    let channel = nav_file.channels().first().ok_or("no channel")?;
    assert_eq!(channel.components(), ["x", "y"]);
    assert_eq!(channel.unit().map(ChannelUnit::label), Some("g"));
    Ok(())
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
    // used the "paged" variant, triggered when `num_chunks` > 1024. Using
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
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t(0)))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t(1000)))
            .lat(Angle::degrees(1.0))
            .lon(Angle::degrees(1.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_annotation(
        Annotation::builder()
            .time(t(500))
            .maybe_label(label)
            .build()?,
    );
    let mut bytes = Vec::new();
    recorder.finish()?.write(&mut bytes)?;
    Ok(NavFile::read(bytes.as_slice())?)
}

#[test]
fn a_fix_without_a_lock_writes_the_gps_time_sentinel_and_a_host_clock_time_axis()
-> Result<(), Box<dyn std::error::Error>> {
    let locked = t(0);
    let host_only = t(1000);

    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Both {
                gps: locked,
                sys: locked,
            })
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Host(host_only))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .build(),
    );
    let bytes = to_bytes(&recorder.finish()?);

    let grp = hdf5_pure::File::from_bytes(bytes)?.group("nav_points")?;
    assert_eq!(
        grp.dataset("gps_time_us")?.read_u64()?,
        vec![locked.timestamp_micros().cast_unsigned(), u64::MAX]
    );
    assert_eq!(
        grp.dataset("time")?.read_i64()?,
        vec![locked.timestamp_micros(), host_only.timestamp_micros()]
    );
    Ok(())
}

#[test]
fn a_file_without_gps_time_us_reads_its_time_axis_as_the_receiver_timestamp()
-> Result<(), Box<dyn std::error::Error>> {
    let fix_time = t(0);
    let host_time = t(500);
    let bytes = make_nav_points_file(
        fix_time.timestamp_micros(),
        host_time.timestamp_micros().cast_unsigned(),
        None,
    );

    let nav_file = NavFile::read(bytes.as_slice())?;
    let fix = &nav_file.nav_points().first().ok_or("no nav point")?.fix;

    assert_eq!(fix.gps_time(), Some(fix_time));
    assert_eq!(fix.sys_time(), Some(host_time));
    Ok(())
}

#[test]
fn a_gps_time_us_shorter_than_the_time_axis_is_rejected() -> Result<(), Box<dyn std::error::Error>>
{
    let bytes = make_nav_points_file(t(0).timestamp_micros(), u64::MAX, Some(&[]));

    let err = NavFile::read(bytes.as_slice()).expect_err("should detect shape mismatch");

    assert!(matches!(
        err,
        Error::ShapeMismatch {
            group: "nav_points",
            dataset: "gps_time_us",
            expected: 1,
            actual: 0,
        }
    ));
    Ok(())
}

fn make_nav_points_file(time_us: i64, sys_time_us: u64, gps_time_us: Option<&[u64]>) -> Vec<u8> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[time_us])
        .with_shape(&[1]);
    if let Some(gps_time_us) = gps_time_us {
        np.create_dataset("gps_time_us")
            .with_u64_data(gps_time_us)
            .with_shape(&[gps_time_us.len() as u64]);
    }
    np.create_dataset("sys_time_us")
        .with_u64_data(&[sys_time_us])
        .with_shape(&[1]);
    np.create_dataset("lat")
        .with_f64_data(&[0.0])
        .with_shape(&[1]);
    np.create_dataset("lon")
        .with_f64_data(&[0.0])
        .with_shape(&[1]);
    np.create_dataset("heading")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[f64::NAN])
        .with_shape(&[1]);
    fb.add_group(np.finish());
    #[expect(clippy::expect_used, reason = "test helper")]
    fb.finish().expect("build")
}

#[expect(
    clippy::expect_used,
    reason = "test helper only called with valid input"
)]
fn instant(text: &str) -> DateTime<Utc> {
    text.parse().expect("valid RFC 3339 timestamp")
}

/// The instant whose microsecond count is -1. Its two's complement bits are the
/// value that marks an absent timestamp.
fn absent_count_instant() -> DateTime<Utc> {
    instant("1969-12-31T23:59:59.999999Z")
}

fn fix_at(time: NavFixTime) -> NavFix {
    NavFix::builder()
        .time(time)
        .lat(Angle::degrees(0.0))
        .lon(Angle::degrees(0.0))
        .build()
}

#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn file_with_a_fix_at(time: NavFixTime) -> NavFile {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(time));
    recorder.finish().expect("build")
}

#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn file_with_a_satellite_report_at(time: NavFixTime) -> NavFile {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(NavFixTime::Receiver(instant(
        "1970-01-01T00:00:00Z",
    ))));
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .time(time)
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .build(),
            ])
            .build(),
    );
    recorder.finish().expect("build")
}

#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn file_with_an_event_marker_at(sys_time: DateTime<Utc>) -> NavFile {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(NavFixTime::Receiver(instant(
        "1970-01-01T00:00:00Z",
    ))));
    recorder.add_event_marker(
        EventMarker::builder()
            .variant_path("power/boot")
            .sys_time(sys_time)
            .build()
            .expect("valid variant path"),
    );
    recorder.finish().expect("build")
}

#[rstest]
#[case::nav_point_gps_time(
    file_with_a_fix_at(NavFixTime::Receiver(absent_count_instant())),
    "nav_points",
    "gps_time_us"
)]
#[case::nav_point_sys_time(
    file_with_a_fix_at(NavFixTime::Host(absent_count_instant())),
    "nav_points",
    "sys_time_us"
)]
#[case::satellite_report_gps_time(
    file_with_a_satellite_report_at(NavFixTime::Receiver(absent_count_instant())),
    "sat_reports",
    "gps_time_us"
)]
#[case::satellite_report_sys_time(
    file_with_a_satellite_report_at(NavFixTime::Host(absent_count_instant())),
    "sat_reports",
    "sys_time_us"
)]
#[case::event_marker_sys_time(
    file_with_an_event_marker_at(absent_count_instant()),
    "event_markers",
    "sys_time_us"
)]
fn a_timestamp_at_the_absent_count_fails_to_write(
    #[case] nav_file: NavFile,
    #[case] expected_group: &str,
    #[case] expected_dataset: &str,
) {
    let mut bytes = Vec::new();
    let err = nav_file
        .write(&mut bytes)
        .expect_err("the writer must reject the absent count");

    match err {
        Error::TimestampIsTheAbsentValue {
            group,
            dataset,
            record,
        } => assert_eq!(
            (group, dataset, record),
            (expected_group, expected_dataset, 0)
        ),
        other => panic!("expected a rejected timestamp, got: {other:?}"),
    }
}

#[rstest]
#[case::one_microsecond_before_the_absent_count(instant("1969-12-31T23:59:59.999998Z"))]
#[case::ten_years_before_the_epoch(instant("1960-01-01T00:00:00Z"))]
#[case::the_epoch(instant("1970-01-01T00:00:00Z"))]
fn a_fix_reads_back_the_time_it_was_written_with(
    #[case] time: DateTime<Utc>,
) -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = file_with_a_fix_at(NavFixTime::Both {
        gps: time,
        sys: time,
    });
    let read_back = NavFile::read(to_bytes(&nav_file).as_slice())?;

    let fix = &read_back.nav_points()[0].fix;
    assert_eq!(fix.gps_time(), Some(time));
    assert_eq!(fix.sys_time(), Some(time));
    Ok(())
}
