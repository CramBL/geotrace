#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use geotrace_sdk::{Angle, DateTime, Utc};
use geotrace_sdk::{
    Annotation, AnnotationIcon, ChannelUnit, Constellation, Error, EventMarker, MarkerIcon,
    NavFile, NavFileBuilder, NavFix, NavFixTime, Satellite, SatelliteReport,
};
use geotrace_sdk_test_util as test_util;
use hdf5_pure::{AttrValue, FileBuilder};
use rstest::rstest;

#[test]
fn nan_for_absent_speed() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(test_util::base()))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let bytes = test_util::to_bytes(&nav_file)?;

    let file = hdf5_pure::File::from_bytes(bytes)?;
    let speeds = file.group("nav_points")?.dataset("speed_mps")?.read_f64()?;
    assert!(speeds[0].is_nan());

    let rt = test_util::round_trip(&nav_file)?;
    assert_eq!(rt.nav_points()[0].fix.speed, None);
    Ok(())
}

#[test]
fn nan_for_absent_satellite_fields() -> Result<(), Box<dyn std::error::Error>> {
    // `elevation`, `azimuth` and `snr` all `None` → NaN on disk, `None` on read-back
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(test_util::base()))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .time(NavFixTime::Receiver(test_util::base()))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .build(),
            ])
            .build(),
    );
    let nav_file = recorder.finish()?;
    let bytes = test_util::to_bytes(&nav_file)?;

    let file = hdf5_pure::File::from_bytes(bytes)?;
    let ts = file.group("tracked_sats")?;
    assert!(ts.dataset("elevation")?.read_f32()?[0].is_nan());
    assert!(ts.dataset("azimuth")?.read_f32()?[0].is_nan());
    assert!(ts.dataset("snr")?.read_f32()?[0].is_nan());

    let rt = test_util::round_trip(&nav_file)?;
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
                .time(NavFixTime::Receiver(test_util::base()))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .time(NavFixTime::Receiver(test_util::base()))
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
        let bytes = test_util::to_bytes(&nav_file)?;

        let file = hdf5_pure::File::from_bytes(bytes)?;
        let codes = file
            .group("tracked_sats")?
            .dataset("constellation")?
            .read_u8()?;
        assert_eq!(codes[0], expected_code);

        let rt = test_util::round_trip(&nav_file)?;
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
                .time(NavFixTime::Receiver(test_util::t_ms(0)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(test_util::t_ms(1000)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_annotation(
            Annotation::builder()
                .time(test_util::t_ms(500))
                .icon(icon)
                .build()?,
        );
        let nav_file = recorder.finish()?;
        let bytes = test_util::to_bytes(&nav_file)?;

        let file = hdf5_pure::File::from_bytes(bytes)?;
        let codes = file.group("markers")?.dataset("icon")?.read_u8()?;
        assert_eq!(
            codes[0], expected_code,
            "icon {icon:?} should be code {expected_code}"
        );

        let rt = test_util::round_trip(&nav_file)?;
        assert_eq!(
            rt.markers()[0].annotation.icon(),
            AnnotationIcon::Icon(icon)
        );
    }
    Ok(())
}

#[rstest]
#[case::a_number_the_reader_does_not_support("10")]
#[case::digits_followed_by_letters("1abc")]
fn a_geotrace_version_outside_the_supported_set_fails_the_read(#[case] version: &str) {
    let bytes = make_file_with_version(version);
    let err = NavFile::read(bytes.as_slice()).expect_err("should reject unknown version");
    assert!(
        matches!(&err, Error::UnsupportedVersion { version: read } if read == version),
        "expected UnsupportedVersion({version:?}), got: {err:?}"
    );
}

#[rstest]
#[case::the_layout_with_one_time_axis("1")]
#[case::the_version_the_writer_stamps("2")]
fn a_supported_geotrace_version_reads(#[case] version: &str) {
    let bytes = make_file_with_version(version);
    let nav_file = NavFile::read(bytes.as_slice()).expect("a supported version reads");
    assert_eq!(nav_file.nav_points().len(), 0);
}

fn make_file_with_version(version: &str) -> Vec<u8> {
    let mut fb = test_util::file_with_an_empty_nav_points_group();
    fb.set_attr("geotrace_version", AttrValue::String(version.into()));
    #[expect(clippy::expect_used, reason = "test helper")]
    fb.finish().expect("build")
}

/// h5py and the reference C library write a string attribute as `H5T_STRING`
/// with `STRSIZE = H5T_VARIABLE`.
#[test]
fn variable_length_string_attributes_are_read() -> Result<(), Box<dyn std::error::Error>> {
    let mut fb = test_util::file_with_an_empty_nav_points_group();
    fb.set_attr("geotrace_version", AttrValue::VarLenString("1".into()));
    fb.set_attr("meta_title", AttrValue::VarLenString("Ride home".into()));

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
    assert_eq!(nav_file.meta().title(), Some("Ride home"));
    let channel = nav_file.channels().first().ok_or("no channel")?;
    assert_eq!(channel.components(), ["x", "y"]);
    assert_eq!(channel.unit().map(ChannelUnit::label), Some("g"));
    Ok(())
}

#[test]
fn a_title_with_a_nul_byte_from_a_file_reads_and_writes_back_unchanged()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fb = test_util::file_with_an_empty_nav_points_group();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));
    fb.set_attr("meta_title", AttrValue::String("before\0after".into()));

    let nav_file = NavFile::read(fb.finish()?.as_slice())?;
    assert_eq!(nav_file.meta().title(), Some("before\0after"));
    assert_eq!(
        test_util::round_trip(&nav_file)?.meta().title(),
        Some("before\0after")
    );
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
fn a_fix_without_a_lock_writes_the_gps_time_sentinel_and_a_host_clock_time_axis()
-> Result<(), Box<dyn std::error::Error>> {
    let locked = test_util::t_ms(0);
    let host_only = test_util::t_ms(1000);

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
    let bytes = test_util::to_bytes(&recorder.finish()?)?;

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
    let fix_time = test_util::t_ms(0);
    let host_time = test_util::t_ms(500);
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
    let bytes = make_nav_points_file(test_util::t_ms(0).timestamp_micros(), u64::MAX, Some(&[]));

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

fn fix_stamped(time: NavFixTime) -> NavFix {
    NavFix::builder()
        .time(time)
        .lat(Angle::degrees(0.0))
        .lon(Angle::degrees(0.0))
        .build()
}

#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn file_with_a_fix_at(time: NavFixTime) -> NavFile {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_stamped(time));
    recorder.finish().expect("build")
}

#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn file_with_a_satellite_report_at(time: NavFixTime) -> NavFile {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_stamped(NavFixTime::Receiver(instant(
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
    // The marker is before the single fix. Lenient mode clamps it to that fix.
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    recorder.add_nav_fix(fix_stamped(NavFixTime::Receiver(instant(
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
    let read_back = test_util::round_trip(&nav_file)?;

    let fix = &read_back.nav_points()[0].fix;
    assert_eq!(fix.gps_time(), Some(time));
    assert_eq!(fix.sys_time(), Some(time));
    Ok(())
}
