#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]
#![expect(
    clippy::unwrap_in_result,
    reason = "test code may use expect() for infallible test invariants"
)]

use geotrace_sdk::{Angle, Unit, Velocity};
use geotrace_sdk::{
    Annotation, AnnotationIcon, Channel, Constellation, EventMarker, EventMarkerIconChoice,
    EventMarkerStyle, MarkerIcon, NavFile, NavFileBuilder, NavFix, NavFixTime, NavRecorder,
    Satellite, SatelliteReport, TravelMode,
};
use geotrace_sdk_test_util as test_util;
use geotrace_sdk_test_util::{
    ANNOTATION_ROW_BYTES, COLOR_HEX_ROW_BYTES, EventMarkerFieldRows, GtdFileContents,
    ICON_NAME_ROW_BYTES, MARKER_LABEL_ROW_BYTES, StyleFieldRows, VARIANT_PATH_ROW_BYTES,
};
use hdf5_pure::{AttrValue, FileBuilder};
use rstest::rstest;

/// A full inspect render of a file exercising every section: the metadata a
/// recording declares, nav points, satellites, markers, event markers over
/// three variant paths, two styles, and both a scalar and a vector channel.
/// The output is the same in every build: timestamps derive from the fixed
/// `test_util::base` and the build stamp is scrubbed.
#[test]
fn snapshot_inspect_populated_file() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = test_util::base();
    let t1 = test_util::t_s(10);

    let mut recorder = NavFileBuilder::new()
        .with_title("Inspect test")?
        .with_device("test-device")?
        .with_notes("a populated file")?
        .with_identity("test-fleet-7")?
        .with_travel_mode(TravelMode::Bicycle)?
        .with_scrubbed_provenance()
        .open();

    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t0))
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .heading(Angle::degrees(270.0))
            .speed(Velocity::meter_per_second(10.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t1))
            .lat(Angle::degrees(51.6))
            .lon(Angle::degrees(-0.2))
            .heading(Angle::degrees(180.0))
            .speed(Velocity::meter_per_second(12.5))
            .build(),
    );

    recorder.add_satellite_report(
        SatelliteReport::builder()
            .time(NavFixTime::Receiver(t0))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .elevation(72.0f32)
                    .azimuth(120.0f32)
                    .snr(35.0f32)
                    .in_fix(true)
                    .build(),
                Satellite::builder()
                    .constellation(Constellation::Galileo)
                    .prn(11u32)
                    .elevation(18.0f32)
                    .azimuth(310.0f32)
                    .snr(30.0f32)
                    .in_fix(true)
                    .build(),
                Satellite::builder()
                    .constellation(Constellation::Galileo)
                    .prn(24u32)
                    .elevation(5.0f32)
                    .azimuth(45.0f32)
                    .snr(99.0f32)
                    .build(),
            ])
            .build(),
    );

    recorder.add_annotation(
        Annotation::builder()
            .time(test_util::t_s(5))
            .label("midpoint")
            .icon(MarkerIcon::Warning)
            .build()?,
    );
    recorder.add_annotation(
        Annotation::builder()
            .time(test_util::t_s(6))
            .label("icon from a newer writer")
            .icon(AnnotationIcon::Unrecognized(200))
            .build()?,
    );

    for (variant_path, offset_secs) in [
        ("power/boot", 1),
        ("power/boot", 7),
        ("gnss/fix_lost", 3),
        ("app/start", 4),
    ] {
        recorder.add_event_marker(
            EventMarker::builder()
                .variant_path(variant_path)
                .sys_time(test_util::t_s(offset_secs))
                .build()?,
        );
    }
    recorder.add_event_marker(
        EventMarker::builder()
            .variant_path("gnss/fix_lost")
            .sys_time(test_util::t_s(8))
            .annotation("antenna unplugged")
            .build()?,
    );

    recorder.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/boot")
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Wrench))
            .color("#FF9900")
            .build()?,
    );
    recorder.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("gnss/fix_lost")
            .build()?,
    );
    // An icon and a color a newer writer set, which this build reads back
    // unchanged.
    let written_by_a_newer_build = NavFile::read(
        GtdFileContents {
            styles: vec![StyleFieldRows {
                variant_path: test_util::nul_padded_row(b"sensor/fault", VARIANT_PATH_ROW_BYTES),
                icon_name: test_util::nul_padded_row(b"hovercraft", ICON_NAME_ROW_BYTES),
                color_hex: test_util::nul_padded_row(b"FF9900", COLOR_HEX_ROW_BYTES),
            }],
            ..GtdFileContents::default()
        }
        .into_gtd_bytes()
        .as_slice(),
    )?;
    for style in written_by_a_newer_build.event_marker_styles() {
        recorder.add_event_marker_style(style.clone());
    }

    recorder.add_channel(
        Channel::builder()
            .name("incline")
            .unit(Unit::DEG)
            .period(Angle::degrees(360.0))
            .times(vec![t0, t1])
            .values(vec![1.5, 2.0])
            .build()?,
    );
    recorder.add_channel(
        Channel::builder()
            .name("accel")
            .unit(Unit::G)
            .description("body frame acceleration")
            .components(["x", "y", "z"].map(String::from).to_vec())
            .times(vec![t0, t1])
            .values(vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02])
            .build()?,
    );

    let nav_file = recorder.finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    insta::assert_snapshot!(output);
    Ok(())
}

fn inspect_bytes(bytes: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::write(tmp.path(), bytes)?;
    Ok(NavFile::inspect(tmp.path())?)
}

/// `NavFile::read` fails on a `markers/label` row that is not UTF-8. `inspect`
/// prints the summary of such a file and states which row it is.
#[test]
fn snapshot_inspect_file_with_a_label_row_that_is_not_utf8()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = GtdFileContents {
        marker_labels: vec![
            test_util::nul_padded_row(b"start", MARKER_LABEL_ROW_BYTES),
            test_util::nul_padded_row(NOT_UTF8, MARKER_LABEL_ROW_BYTES),
            test_util::nul_padded_row(b"end", MARKER_LABEL_ROW_BYTES),
        ],
        ..GtdFileContents::default()
    }
    .into_gtd_bytes();

    insta::assert_snapshot!(inspect_bytes(&bytes)?);
    Ok(())
}

/// `NavFile::read` fails on an `event_markers` or `event_marker_styles` row
/// that is not UTF-8. `inspect` prints the summary of such a file and states
/// which rows they are.
#[test]
fn snapshot_inspect_file_with_event_marker_rows_that_are_not_utf8()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = GtdFileContents {
        event_markers: vec![
            EventMarkerFieldRows {
                variant_path: test_util::nul_padded_row(b"power/boot", VARIANT_PATH_ROW_BYTES),
                annotation: test_util::nul_padded_row(b"battery replaced", ANNOTATION_ROW_BYTES),
            },
            EventMarkerFieldRows {
                variant_path: test_util::nul_padded_row(NOT_UTF8, VARIANT_PATH_ROW_BYTES),
                annotation: test_util::nul_padded_row(NOT_UTF8, ANNOTATION_ROW_BYTES),
            },
        ],
        styles: vec![
            StyleFieldRows {
                variant_path: test_util::nul_padded_row(b"power/boot", VARIANT_PATH_ROW_BYTES),
                icon_name: test_util::nul_padded_row(NOT_UTF8, ICON_NAME_ROW_BYTES),
                color_hex: test_util::nul_padded_row(NOT_UTF8, COLOR_HEX_ROW_BYTES),
            },
            StyleFieldRows {
                variant_path: test_util::nul_padded_row(NOT_UTF8, VARIANT_PATH_ROW_BYTES),
                icon_name: test_util::nul_padded_row(b"wrench", ICON_NAME_ROW_BYTES),
                color_hex: test_util::nul_padded_row(b"#FF9900", COLOR_HEX_ROW_BYTES),
            },
        ],
        ..GtdFileContents::default()
    }
    .into_gtd_bytes();

    insta::assert_snapshot!(inspect_bytes(&bytes)?);
    Ok(())
}

/// The summary lists the first rows of a group and closes the list with `…`.
#[test]
fn snapshot_inspect_file_with_more_rows_than_the_summary_lists()
-> Result<(), Box<dyn std::error::Error>> {
    let mut event_markers: Vec<EventMarkerFieldRows> = (0..22)
        .map(|row| EventMarkerFieldRows {
            variant_path: test_util::nul_padded_row(
                format!("event/{row:02}").as_bytes(),
                VARIANT_PATH_ROW_BYTES,
            ),
            annotation: test_util::nul_padded_row(
                format!("note {row:02}").as_bytes(),
                ANNOTATION_ROW_BYTES,
            ),
        })
        .collect();
    event_markers.extend((0..4).map(|_| EventMarkerFieldRows {
        variant_path: test_util::nul_padded_row(NOT_UTF8, VARIANT_PATH_ROW_BYTES),
        annotation: test_util::nul_padded_row(NOT_UTF8, ANNOTATION_ROW_BYTES),
    }));

    let styles: Vec<StyleFieldRows> = (0..21)
        .map(|row| StyleFieldRows {
            variant_path: test_util::nul_padded_row(
                format!("style/{row:02}").as_bytes(),
                VARIANT_PATH_ROW_BYTES,
            ),
            icon_name: test_util::nul_padded_row(b"", ICON_NAME_ROW_BYTES),
            color_hex: test_util::nul_padded_row(b"", COLOR_HEX_ROW_BYTES),
        })
        .collect();

    let bytes = GtdFileContents {
        event_markers,
        styles,
        ..GtdFileContents::default()
    }
    .into_gtd_bytes();

    insta::assert_snapshot!(inspect_bytes(&bytes)?);
    Ok(())
}

/// The metadata section states the version, commit and commit time of the SDK
/// build that wrote the file.
#[test]
fn inspect_states_the_build_stamp_a_file_holds() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = GtdFileContents {
        attrs: vec![
            ("sdk_version", "0.4.2"),
            ("sdk_git_commit", "0123456789abcdef0123456789abcdef01234567"),
            ("sdk_commit_time", "2026-02-01T15:00:00Z"),
        ],
        ..GtdFileContents::default()
    }
    .into_gtd_bytes();
    let output = inspect_bytes(&bytes)?;

    let metadata: Vec<&str> = output
        .lines()
        .skip_while(|line| *line != "Metadata")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .collect();
    assert_eq!(
        metadata,
        [
            "  sdk version    : 0.4.2",
            "  sdk commit     : 0123456789abcdef0123456789abcdef01234567",
            "  sdk commit time: 2026-02-01T15:00:00Z",
        ]
    );
    Ok(())
}

/// A group whose `variant_path` dataset `inspect` cannot read at all has no
/// record count, and the rest of the summary follows.
#[test]
fn inspect_states_an_event_marker_styles_group_without_a_variant_path_dataset()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));
    test_util::add_nav_points_group_of_one_fix(&mut fb);
    let mut styles = fb.create_group("event_marker_styles");
    styles
        .create_dataset("icon_name")
        .with_u8_data(&test_util::nul_padded_row(b"wrench", ICON_NAME_ROW_BYTES))
        .with_shape(&[1, ICON_NAME_ROW_BYTES as u64]);
    fb.add_group(styles.finish());

    let output = inspect_bytes(&fb.finish()?)?;

    let section = output
        .lines()
        .find(|line| line.starts_with("Event Marker Styles"));
    assert_eq!(
        section,
        Some("Event Marker Styles     unreadable variant_path")
    );
    Ok(())
}

#[rstest]
#[case::nav_points_of_a_file_without_fixes(
    NavFileBuilder::new().open(),
    "Nav Points              0 records"
)]
#[case::satellite_reports(
    test_util::recorder_with_one_fix(),
    "Satellite Reports       0 records"
)]
#[case::markers(
    test_util::recorder_with_one_fix(),
    "Markers                 0 records"
)]
#[case::channels(
    test_util::recorder_with_one_fix(),
    "Channels                0 channels"
)]
fn inspect_states_a_count_of_zero_for_an_empty_section(
    #[case] recorder: NavRecorder,
    #[case] expected_line: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = test_util::to_bytes(&recorder.finish()?)?;

    let output = inspect_bytes(&bytes)?;
    assert!(
        output.lines().any(|line| line == expected_line),
        "no line {expected_line:?} in:\n{output}"
    );
    Ok(())
}

/// Field content no reader decodes as UTF-8.
const NOT_UTF8: &[u8] = &[0xff];
