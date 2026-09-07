#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]
#![expect(
    clippy::unwrap_in_result,
    reason = "test code may use expect() for infallible test invariants"
)]

use geotrace_sdk::{Angle, DateTime, Duration, Unit, Utc, Velocity};
use geotrace_sdk::{
    Annotation, AnnotationField, AnnotationIcon, Channel, ColorHexField, Constellation,
    EventMarker, EventMarkerColor, EventMarkerIconChoice, EventMarkerStyle, IconNameField,
    MarkerIcon, MarkerLabelField, NavFile, NavFileBuilder, NavFix, NavFixTime, Satellite,
    SatelliteReport, TravelMode, VariantPathField,
};
use hdf5_pure::{AttrValue, FileBuilder};

#[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

/// A full inspect render of a file exercising every section: the metadata a
/// recording declares, nav points, satellites, markers, event markers over
/// three variant paths, two styles, and both a scalar and a vector channel.
/// The output is the same in every build: timestamps derive from the fixed
/// [`base`] and the build stamp is scrubbed.
#[test]
fn snapshot_inspect_populated_file() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let t1 = t0 + Duration::seconds(10);
    let at = |offset_secs: i64| t0 + Duration::seconds(offset_secs);

    let mut recorder = NavFileBuilder::new()
        .with_title("Inspect test")
        .with_device("test-device")
        .with_notes("a populated file")
        .with_identity("test-fleet-7")
        .with_travel_mode(TravelMode::Bicycle)
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
            .time(at(5))
            .label("midpoint")
            .icon(MarkerIcon::Warning)
            .build()?,
    );
    recorder.add_annotation(
        Annotation::builder()
            .time(at(6))
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
                .sys_time(at(offset_secs))
                .build()?,
        );
    }
    recorder.add_event_marker(
        EventMarker::builder()
            .variant_path("gnss/fix_lost")
            .sys_time(at(8))
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
    recorder.add_event_marker_style(EventMarkerStyle {
        variant_path: "sensor/fault".to_owned(),
        icon: EventMarkerIconChoice::Unrecognized("hovercraft".to_owned()),
        color: EventMarkerColor::Unrecognized("FF9900".to_owned()),
    });

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

const MARKER_LABEL_ROW_BYTES: usize = MarkerLabelField::CONTENT_CAPACITY + 1;
const VARIANT_PATH_ROW_BYTES: usize = VariantPathField::CONTENT_CAPACITY + 1;
const ANNOTATION_ROW_BYTES: usize = AnnotationField::CONTENT_CAPACITY + 1;
const ICON_NAME_ROW_BYTES: usize = IconNameField::CONTENT_CAPACITY + 1;
const COLOR_HEX_ROW_BYTES: usize = ColorHexField::CONTENT_CAPACITY + 1;
const MARKER_ICON_WARNING_CODE: u8 = 4;

/// Field content no reader decodes as UTF-8.
const NOT_UTF8: &[u8] = &[0xff];

fn fix_time_us() -> i64 {
    base().timestamp_micros()
}

fn nul_padded_row(content: &[u8], row_bytes: usize) -> Vec<u8> {
    let mut row = content.to_vec();
    row.resize(row_bytes, 0);
    row
}

struct EventMarkerRow {
    variant_path: Vec<u8>,
    annotation: Vec<u8>,
}

struct StyleRow {
    variant_path: Vec<u8>,
    icon_name: Vec<u8>,
    color_hex: Vec<u8>,
}

/// What the file [`gtd_bytes`] builds holds beyond its one nav fix: root
/// attributes, one marker per label row, one event marker per row, one style
/// per row. A list left empty leaves its group out of the file.
#[derive(Default)]
struct GtdFileContents {
    attrs: Vec<(&'static str, &'static str)>,
    marker_labels: Vec<Vec<u8>>,
    event_markers: Vec<EventMarkerRow>,
    styles: Vec<StyleRow>,
}

fn nav_points_group_of_one_fix(fb: &mut FileBuilder) {
    let mut nav_points = fb.create_group("nav_points");
    nav_points
        .create_dataset("time")
        .with_i64_data(&[fix_time_us()])
        .with_shape(&[1]);
    for (name, value) in [
        ("lat", 55.0),
        ("lon", 12.0),
        ("heading", 90.0),
        ("speed_mps", 3.0),
    ] {
        nav_points
            .create_dataset(name)
            .with_f64_data(&[value])
            .with_shape(&[1]);
    }
    fb.add_group(nav_points.finish());
}

/// A `.gtd` file of one nav fix, with the attributes, markers, event markers
/// and styles of [`GtdFileContents`]. A row that is not UTF-8 has to be
/// assembled here, since the writer takes a `String` for every field that holds
/// one.
#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn gtd_bytes(
    GtdFileContents {
        attrs,
        marker_labels,
        event_markers,
        styles,
    }: GtdFileContents,
) -> Vec<u8> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));
    for (name, value) in attrs {
        fb.set_attr(name, AttrValue::String(value.to_owned()));
    }

    nav_points_group_of_one_fix(&mut fb);

    if !marker_labels.is_empty() {
        let count = marker_labels.len();
        let mut markers = fb.create_group("markers");
        markers
            .create_dataset("time")
            .with_i64_data(&vec![fix_time_us(); count])
            .with_shape(&[count as u64]);
        markers
            .create_dataset("lat")
            .with_f64_data(&vec![55.0; count])
            .with_shape(&[count as u64]);
        markers
            .create_dataset("lon")
            .with_f64_data(&vec![12.0; count])
            .with_shape(&[count as u64]);
        markers
            .create_dataset("icon")
            .with_u8_data(&vec![MARKER_ICON_WARNING_CODE; count])
            .with_shape(&[count as u64]);
        markers
            .create_dataset("label")
            .with_u8_data(&marker_labels.concat())
            .with_shape(&[count as u64, MARKER_LABEL_ROW_BYTES as u64]);
        fb.add_group(markers.finish());
    }

    if !event_markers.is_empty() {
        let count = event_markers.len();
        let mut grp = fb.create_group("event_markers");
        grp.create_dataset("sys_time_us")
            .with_u64_data(&vec![fix_time_us().cast_unsigned(); count])
            .with_shape(&[count as u64]);
        grp.create_dataset("lat")
            .with_f64_data(&vec![55.0; count])
            .with_shape(&[count as u64]);
        grp.create_dataset("lon")
            .with_f64_data(&vec![12.0; count])
            .with_shape(&[count as u64]);
        grp.create_dataset("variant_path")
            .with_u8_data(
                &event_markers
                    .iter()
                    .flat_map(|row| row.variant_path.clone())
                    .collect::<Vec<u8>>(),
            )
            .with_shape(&[count as u64, VARIANT_PATH_ROW_BYTES as u64]);
        grp.create_dataset("annotation")
            .with_u8_data(
                &event_markers
                    .iter()
                    .flat_map(|row| row.annotation.clone())
                    .collect::<Vec<u8>>(),
            )
            .with_shape(&[count as u64, ANNOTATION_ROW_BYTES as u64]);
        fb.add_group(grp.finish());
    }

    if !styles.is_empty() {
        let count = styles.len();
        let mut grp = fb.create_group("event_marker_styles");
        grp.create_dataset("variant_path")
            .with_u8_data(
                &styles
                    .iter()
                    .flat_map(|row| row.variant_path.clone())
                    .collect::<Vec<u8>>(),
            )
            .with_shape(&[count as u64, VARIANT_PATH_ROW_BYTES as u64]);
        grp.create_dataset("icon_name")
            .with_u8_data(
                &styles
                    .iter()
                    .flat_map(|row| row.icon_name.clone())
                    .collect::<Vec<u8>>(),
            )
            .with_shape(&[count as u64, ICON_NAME_ROW_BYTES as u64]);
        grp.create_dataset("color_hex")
            .with_u8_data(
                &styles
                    .iter()
                    .flat_map(|row| row.color_hex.clone())
                    .collect::<Vec<u8>>(),
            )
            .with_shape(&[count as u64, COLOR_HEX_ROW_BYTES as u64]);
        fb.add_group(grp.finish());
    }

    fb.finish().expect("the assembled file builds")
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
    let bytes = gtd_bytes(GtdFileContents {
        marker_labels: vec![
            nul_padded_row(b"start", MARKER_LABEL_ROW_BYTES),
            nul_padded_row(NOT_UTF8, MARKER_LABEL_ROW_BYTES),
            nul_padded_row(b"end", MARKER_LABEL_ROW_BYTES),
        ],
        ..GtdFileContents::default()
    });

    insta::assert_snapshot!(inspect_bytes(&bytes)?);
    Ok(())
}

/// `NavFile::read` fails on an `event_markers` or `event_marker_styles` row
/// that is not UTF-8. `inspect` prints the summary of such a file and states
/// which rows they are.
#[test]
fn snapshot_inspect_file_with_event_marker_rows_that_are_not_utf8()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = gtd_bytes(GtdFileContents {
        event_markers: vec![
            EventMarkerRow {
                variant_path: nul_padded_row(b"power/boot", VARIANT_PATH_ROW_BYTES),
                annotation: nul_padded_row(b"battery replaced", ANNOTATION_ROW_BYTES),
            },
            EventMarkerRow {
                variant_path: nul_padded_row(NOT_UTF8, VARIANT_PATH_ROW_BYTES),
                annotation: nul_padded_row(NOT_UTF8, ANNOTATION_ROW_BYTES),
            },
        ],
        styles: vec![
            StyleRow {
                variant_path: nul_padded_row(b"power/boot", VARIANT_PATH_ROW_BYTES),
                icon_name: nul_padded_row(NOT_UTF8, ICON_NAME_ROW_BYTES),
                color_hex: nul_padded_row(NOT_UTF8, COLOR_HEX_ROW_BYTES),
            },
            StyleRow {
                variant_path: nul_padded_row(NOT_UTF8, VARIANT_PATH_ROW_BYTES),
                icon_name: nul_padded_row(b"wrench", ICON_NAME_ROW_BYTES),
                color_hex: nul_padded_row(b"#FF9900", COLOR_HEX_ROW_BYTES),
            },
        ],
        ..GtdFileContents::default()
    });

    insta::assert_snapshot!(inspect_bytes(&bytes)?);
    Ok(())
}

/// The summary lists the first rows of a group and closes the list with `…`.
#[test]
fn snapshot_inspect_file_with_more_rows_than_the_summary_lists()
-> Result<(), Box<dyn std::error::Error>> {
    let mut event_markers: Vec<EventMarkerRow> = (0..22)
        .map(|row| EventMarkerRow {
            variant_path: nul_padded_row(
                format!("event/{row:02}").as_bytes(),
                VARIANT_PATH_ROW_BYTES,
            ),
            annotation: nul_padded_row(format!("note {row:02}").as_bytes(), ANNOTATION_ROW_BYTES),
        })
        .collect();
    event_markers.extend((0..4).map(|_| EventMarkerRow {
        variant_path: nul_padded_row(NOT_UTF8, VARIANT_PATH_ROW_BYTES),
        annotation: nul_padded_row(NOT_UTF8, ANNOTATION_ROW_BYTES),
    }));

    let styles: Vec<StyleRow> = (0..21)
        .map(|row| StyleRow {
            variant_path: nul_padded_row(
                format!("style/{row:02}").as_bytes(),
                VARIANT_PATH_ROW_BYTES,
            ),
            icon_name: nul_padded_row(b"", ICON_NAME_ROW_BYTES),
            color_hex: nul_padded_row(b"", COLOR_HEX_ROW_BYTES),
        })
        .collect();

    let bytes = gtd_bytes(GtdFileContents {
        event_markers,
        styles,
        ..GtdFileContents::default()
    });

    insta::assert_snapshot!(inspect_bytes(&bytes)?);
    Ok(())
}

/// The metadata section states the version, commit and commit time of the SDK
/// build that wrote the file.
#[test]
fn inspect_states_the_build_stamp_a_file_holds() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = gtd_bytes(GtdFileContents {
        attrs: vec![
            ("sdk_version", "0.4.2"),
            ("sdk_git_commit", "0123456789abcdef0123456789abcdef01234567"),
            ("sdk_commit_time", "2026-02-01T15:00:00Z"),
        ],
        ..GtdFileContents::default()
    });
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
    nav_points_group_of_one_fix(&mut fb);
    let mut styles = fb.create_group("event_marker_styles");
    styles
        .create_dataset("icon_name")
        .with_u8_data(&nul_padded_row(b"wrench", ICON_NAME_ROW_BYTES))
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

#[test]
fn empty_file() -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = NavFileBuilder::new().open().finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(output.contains("version 2"), "missing version: {output}");

    Ok(())
}

#[test]
fn file_with_no_satellite_data() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..3i64 {
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(t0 + Duration::seconds(i)))
                .lat(Angle::degrees(55.0))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(
        output.contains("Satellite Reports") && output.contains("0 records"),
        "expected satellite section with '0 records': {output}"
    );

    Ok(())
}

#[test]
fn file_with_no_markers() -> Result<(), Box<dyn std::error::Error>> {
    let t0 = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(t0))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );

    let nav_file = recorder.finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;

    let output = NavFile::inspect(tmp.path())?;
    assert!(
        output.contains("Markers") && output.contains("0 records"),
        "expected markers section with '0 records': {output}"
    );

    Ok(())
}

#[test]
fn inspect_reports_no_channels_when_absent() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(base()))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    nav_file.write(tmp.as_file())?;
    let output = NavFile::inspect(tmp.path())?;
    assert!(
        output.contains("0 channels"),
        "missing zero-channel line: {output}"
    );
    Ok(())
}
