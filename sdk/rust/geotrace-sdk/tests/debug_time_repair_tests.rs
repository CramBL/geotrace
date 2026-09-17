#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use geotrace_sdk::{
    Angle, Channel, DateTime, DebugTimeRepair, Duration, EventMarker, MarkerIcon, NavFile,
    NavFileBuilder, NavFileOpenMode, NavFix, NavFixTime, Satellite, SatelliteReport, Unit, Utc,
};
use geotrace_sdk_test_util as test_util;
use hdf5_pure::{AttrValue, FileBuilder};

#[test]
fn untagged_regular_open_keeps_repeated_time() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = test_util::base();
    let file = NavFile::read(repeated_time_gtd_bytes(t0, None).as_slice())?;

    assert_eq!(
        nav_fix_times(&file),
        vec![
            t0,
            t0 + Duration::seconds(3),
            t0 + Duration::seconds(6),
            t0,
            t0 + Duration::seconds(3),
            t0 + Duration::seconds(6)
        ]
    );
    Ok(())
}

#[test]
fn explicit_debug_open_mode_repairs_repeated_time() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = test_util::base();
    let mode = NavFileOpenMode::DebugTimeRepair(DebugTimeRepair::new(Duration::seconds(5))?);
    let file = NavFile::read_with_mode(repeated_time_gtd_bytes(t0, None).as_slice(), mode)?;

    assert_repaired_timeline(&file, t0);
    assert_eq!(file.debug_time_repair_tag(), None);
    Ok(())
}

#[test]
fn tagged_regular_open_repairs_repeated_time() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = test_util::base();
    let tag = DebugTimeRepair::new(Duration::seconds(5))?;
    let file = NavFile::read(repeated_time_gtd_bytes(t0, Some(tag)).as_slice())?;

    assert_repaired_timeline(&file, t0);
    assert_eq!(file.debug_time_repair_tag(), Some(tag));
    Ok(())
}

#[test]
fn debug_time_repair_tag_round_trips_through_write_and_read()
-> Result<(), Box<dyn std::error::Error>> {
    let tag = DebugTimeRepair::new(Duration::seconds(7))?;
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(test_util::base()))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .build(),
    );
    let mut file = recorder.finish()?;
    file.set_debug_time_repair_tag(tag);

    let round_tripped = test_util::round_trip(&file)?;

    assert_eq!(round_tripped.debug_time_repair_tag(), Some(tag));
    assert_eq!(
        round_tripped.nav_points()[0].fix.gps_time(),
        Some(test_util::base())
    );
    Ok(())
}

fn assert_repaired_timeline(file: &NavFile, t0: DateTime<Utc>) {
    assert_eq!(
        nav_fix_times(file),
        vec![
            t0,
            t0 + Duration::seconds(3),
            t0 + Duration::seconds(6),
            t0 + Duration::seconds(11),
            t0 + Duration::seconds(14),
            t0 + Duration::seconds(17)
        ]
    );
    let second_span_first_fix = file
        .nav_points()
        .get(3)
        .and_then(|point| point.satellites.as_ref());
    assert_eq!(
        second_span_first_fix.and_then(SatelliteReport::gps_time),
        Some(t0 + Duration::seconds(11))
    );
    assert_eq!(
        second_span_first_fix.and_then(SatelliteReport::sys_time),
        Some(t0 + Duration::seconds(11))
    );
    assert_eq!(
        file.markers()
            .iter()
            .map(|marker| marker.annotation.time())
            .collect::<Vec<_>>(),
        vec![t0 + Duration::seconds(6), t0 + Duration::seconds(14)]
    );
    assert_eq!(
        file.event_markers()
            .iter()
            .map(|marker| marker.sys_time)
            .collect::<Vec<_>>(),
        vec![t0 + Duration::seconds(6), t0 + Duration::seconds(14)]
    );
    assert_eq!(
        file.channels()
            .iter()
            .find(|channel| channel.name() == "clock")
            .map(|channel| channel.times().to_vec()),
        Some(vec![
            t0,
            t0 + Duration::seconds(3),
            t0 + Duration::seconds(6),
            t0 + Duration::seconds(11),
            t0 + Duration::seconds(14),
            t0 + Duration::seconds(17)
        ])
    );
    assert_eq!(
        file.channels()
            .iter()
            .find(|channel| channel.name() == "sparse")
            .map(|channel| channel.times().to_vec()),
        Some(vec![t0 + Duration::seconds(6), t0 + Duration::seconds(14)])
    );
}

fn nav_fix_times(file: &NavFile) -> Vec<DateTime<Utc>> {
    file.nav_points()
        .iter()
        .map(|point| point.fix.effective_gps_time())
        .collect()
}

#[expect(clippy::expect_used, reason = "test fixture assembly must succeed")]
fn repeated_time_gtd_bytes(t0: DateTime<Utc>, tag: Option<DebugTimeRepair>) -> Vec<u8> {
    let offsets = [0, 3, 6, 0, 3, 6].map(Duration::seconds);
    let times_us: Vec<i64> = offsets
        .iter()
        .map(|offset| (t0 + *offset).timestamp_micros())
        .collect();
    let times_us_as_u64: Vec<u64> = times_us
        .iter()
        .map(|time| u64::try_from(*time).expect("positive timestamp"))
        .collect();
    let count = times_us.len();
    let shape = [count as u64];
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".to_owned()));
    if let Some(tag) = tag {
        fb.set_attr(
            "debug_time_repair_backward_jump_threshold_us",
            AttrValue::I64(
                tag.backward_jump_threshold()
                    .num_microseconds()
                    .expect("threshold fits the tag"),
            ),
        );
    }

    let mut nav_points = fb.create_group("nav_points");
    nav_points
        .create_dataset("time")
        .with_i64_data(&times_us)
        .with_shape(&shape);
    nav_points
        .create_dataset("gps_time_us")
        .with_u64_data(&times_us_as_u64)
        .with_shape(&shape);
    nav_points
        .create_dataset("sys_time_us")
        .with_u64_data(&times_us_as_u64)
        .with_shape(&shape);
    for (name, value) in [
        ("lat", 55.0),
        ("lon", 12.0),
        ("heading", 0.0),
        ("speed_mps", f64::NAN),
        ("eph_m", f64::NAN),
    ] {
        nav_points
            .create_dataset(name)
            .with_f64_data(&vec![value; count])
            .with_shape(&shape);
    }
    fb.add_group(nav_points.finish());

    let nav_point_indices: Vec<u64> = (0..count as u64).collect();
    let mut sat_reports = fb.create_group("sat_reports");
    sat_reports
        .create_dataset("nav_point_idx")
        .with_u64_data(&nav_point_indices)
        .with_shape(&shape);
    sat_reports
        .create_dataset("gps_time_us")
        .with_u64_data(&times_us_as_u64)
        .with_shape(&shape);
    sat_reports
        .create_dataset("sys_time_us")
        .with_u64_data(&times_us_as_u64)
        .with_shape(&shape);
    fb.add_group(sat_reports.finish());

    let mut tracked_sats = fb.create_group("tracked_sats");
    tracked_sats
        .create_dataset("sat_report_idx")
        .with_u64_data(&nav_point_indices)
        .with_shape(&shape);
    tracked_sats
        .create_dataset("constellation")
        .with_u8_data(&vec![0; count])
        .with_shape(&shape);
    tracked_sats
        .create_dataset("prn")
        .with_u32_data(&vec![3; count])
        .with_shape(&shape);
    tracked_sats
        .create_dataset("in_fix")
        .with_u8_data(&vec![1; count])
        .with_shape(&shape);
    for name in ["elevation", "azimuth", "snr"] {
        tracked_sats
            .create_dataset(name)
            .with_f32_data(&vec![f32::NAN; count])
            .with_shape(&shape);
    }
    fb.add_group(tracked_sats.finish());

    let marker_times = [
        (t0 + Duration::seconds(6)).timestamp_micros(),
        (t0 + Duration::seconds(3)).timestamp_micros(),
    ];
    let mut markers = fb.create_group("markers");
    markers
        .create_dataset("time")
        .with_i64_data(&marker_times)
        .with_shape(&[2]);
    markers
        .create_dataset("lat")
        .with_f64_data(&[55.0, 55.0])
        .with_shape(&[2]);
    markers
        .create_dataset("lon")
        .with_f64_data(&[12.0, 12.0])
        .with_shape(&[2]);
    markers
        .create_dataset("icon")
        .with_u8_data(&[MarkerIcon::Pin.wire_code(), MarkerIcon::Pin.wire_code()])
        .with_shape(&[2]);
    markers
        .create_dataset("label")
        .with_u8_data(
            &[
                test_util::nul_padded_row(b"first", test_util::MARKER_LABEL_ROW_BYTES),
                test_util::nul_padded_row(b"second", test_util::MARKER_LABEL_ROW_BYTES),
            ]
            .concat(),
        )
        .with_shape(&[2, test_util::MARKER_LABEL_ROW_BYTES as u64]);
    fb.add_group(markers.finish());

    let event_times = [
        u64::try_from((t0 + Duration::seconds(6)).timestamp_micros()).expect("positive timestamp"),
        u64::try_from((t0 + Duration::seconds(3)).timestamp_micros()).expect("positive timestamp"),
    ];
    let mut event_markers = fb.create_group("event_markers");
    event_markers
        .create_dataset("sys_time_us")
        .with_u64_data(&event_times)
        .with_shape(&[2]);
    event_markers
        .create_dataset("lat")
        .with_f64_data(&[55.0, 55.0])
        .with_shape(&[2]);
    event_markers
        .create_dataset("lon")
        .with_f64_data(&[12.0, 12.0])
        .with_shape(&[2]);
    event_markers
        .create_dataset("variant_path")
        .with_u8_data(
            &[
                test_util::nul_padded_row(b"debug/first", test_util::VARIANT_PATH_ROW_BYTES),
                test_util::nul_padded_row(b"debug/second", test_util::VARIANT_PATH_ROW_BYTES),
            ]
            .concat(),
        )
        .with_shape(&[2, test_util::VARIANT_PATH_ROW_BYTES as u64]);
    event_markers
        .create_dataset("annotation")
        .with_u8_data(
            &[
                test_util::nul_padded_row(b"", test_util::ANNOTATION_ROW_BYTES),
                test_util::nul_padded_row(b"", test_util::ANNOTATION_ROW_BYTES),
            ]
            .concat(),
        )
        .with_shape(&[2, test_util::ANNOTATION_ROW_BYTES as u64]);
    fb.add_group(event_markers.finish());

    let mut channels = fb.create_group("channels");
    let mut clock = channels.create_group("clock");
    clock
        .create_dataset("time")
        .with_i64_data(&times_us)
        .with_shape(&shape);
    clock
        .create_dataset("value")
        .with_f64_data(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0])
        .with_shape(&shape);
    channels.add_group(clock.finish());
    let mut sparse = channels.create_group("sparse");
    sparse
        .create_dataset("time")
        .with_i64_data(&[
            (t0 + Duration::seconds(6)).timestamp_micros(),
            (t0 + Duration::seconds(3)).timestamp_micros(),
        ])
        .with_shape(&[2]);
    sparse
        .create_dataset("value")
        .with_f64_data(&[10.0, 11.0])
        .with_shape(&[2]);
    channels.add_group(sparse.finish());
    fb.add_group(channels.finish());

    fb.finish().expect("valid debug recording")
}

#[test]
fn debug_time_repair_can_tag_a_nav_file_with_builder_output()
-> Result<(), Box<dyn std::error::Error>> {
    let tag = DebugTimeRepair::new(Duration::seconds(9))?;
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(test_util::base()))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .build(),
    );
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .time(NavFixTime::Receiver(test_util::base()))
            .tracked(vec![
                Satellite::builder()
                    .constellation(geotrace_sdk::Constellation::Gps)
                    .prn(3u32)
                    .build(),
            ])
            .build(),
    );
    recorder.add_event_marker(
        EventMarker::builder()
            .variant_path("debug/open")
            .sys_time(test_util::base())
            .build()?,
    );
    recorder.add_channel(
        Channel::builder()
            .name("speed")
            .unit(Unit::M_PER_S)
            .times(vec![test_util::base()])
            .values(vec![1.0])
            .build()?,
    );

    let tagged = recorder.finish()?.with_debug_time_repair_tag(tag);

    assert_eq!(tagged.debug_time_repair_tag(), Some(tag));
    Ok(())
}
