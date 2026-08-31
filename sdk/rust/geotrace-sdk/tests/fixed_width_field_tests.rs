//! The fixed-width string fields of the `.gtd` format: the writer refusing a
//! value past a field's capacity, the reader refusing a field row that is not
//! UTF-8, and the reader preserving a well-formed value it does not recognize.
//! The write tests go through the paths that reach those fields without passing
//! `EventMarker::builder().build()`.

use geotrace_sdk::{
    Angle, AnnotationField, ColorHexField, DateTime, Duration, EventKind, EventMarker,
    EventMarkerColor, EventMarkerIconChoice, EventMarkerStyle, IconNameField, MarkerIcon,
    MarkerLabelField, NavFile, NavFileBuilder, NavFix, NavRecorder, Utc, VariantPathField,
};
use hdf5_pure::{AttrValue, FileBuilder};
use rstest::rstest;

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

fn t(offset_secs: i64) -> DateTime<Utc> {
    base() + Duration::seconds(offset_secs)
}

fn recorder_with_one_fix() -> NavRecorder {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum PowerEvent {
    Boot,
}

#[test]
fn an_event_marker_at_the_field_capacities_round_trips() {
    let variant_path = "a".repeat(VariantPathField::CONTENT_CAPACITY);
    let annotation = "n".repeat(AnnotationField::CONTENT_CAPACITY);
    let mut recorder = recorder_with_one_fix();
    recorder.add_event_marker(
        EventMarker::builder()
            .variant_path(variant_path.clone())
            .sys_time(t(0))
            .annotation(annotation.clone())
            .build()
            .expect("a value at the field capacity is accepted"),
    );
    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("the recording builds")
        .write(&mut bytes)
        .expect("a value at the field capacity is written");

    let loaded = NavFile::read(bytes.as_slice()).expect("the written file reads back");
    let marker = loaded
        .event_markers()
        .first()
        .expect("the file holds the marker");
    assert_eq!(marker.variant_path, variant_path);
    assert_eq!(marker.annotation.as_deref(), Some(annotation.as_str()));
}

#[test]
fn a_note_one_byte_past_the_annotation_capacity_stops_the_write() {
    let note = "n".repeat(AnnotationField::CONTENT_CAPACITY + 1);
    let mut recorder = recorder_with_one_fix();
    recorder.add_event_with_note(&PowerEvent::Boot, t(0), note.clone());

    let refusal = recorder
        .finish()
        .expect("the recording builds")
        .write(Vec::new())
        .expect_err("a note past the field capacity stops the write")
        .to_string();
    assert_eq!(
        refusal,
        format!(
            "event_markers/annotation: {note:?} is 512 bytes, past the 511 bytes the field holds"
        )
    );
}

#[test]
fn a_style_variant_path_one_byte_past_the_capacity_stops_the_write() {
    let variant_path = "a".repeat(VariantPathField::CONTENT_CAPACITY + 1);
    let mut recorder = recorder_with_one_fix();
    recorder.add_event_marker_style(EventMarkerStyle {
        variant_path: variant_path.clone(),
        icon: EventMarkerIconChoice::Auto,
        color: EventMarkerColor::Auto,
    });

    let refusal = recorder
        .finish()
        .expect("the recording builds")
        .write(Vec::new())
        .expect_err("a variant path past the field capacity stops the write")
        .to_string();
    assert_eq!(
        refusal,
        format!(
            "event_marker_styles/variant_path: {variant_path:?} is 256 bytes, past the 255 bytes the field holds"
        )
    );
}

#[test]
fn a_style_color_one_byte_past_the_capacity_stops_the_write() {
    let mut recorder = recorder_with_one_fix();
    recorder.add_event_marker_style(EventMarkerStyle {
        variant_path: "power/boot".to_owned(),
        icon: EventMarkerIconChoice::Auto,
        color: EventMarkerColor::hex("#FFAA001"),
    });

    let refusal = recorder
        .finish()
        .expect("the recording builds")
        .write(Vec::new())
        .expect_err("a color past the field capacity stops the write")
        .to_string();
    assert_eq!(
        refusal,
        "event_marker_styles/color_hex: \"#FFAA001\" is 8 bytes, past the 7 bytes the field holds"
    );
}

const MARKER_LABEL_ROW_BYTES: usize = MarkerLabelField::CONTENT_CAPACITY + 1;
const VARIANT_PATH_ROW_BYTES: usize = VariantPathField::CONTENT_CAPACITY + 1;
const ANNOTATION_ROW_BYTES: usize = AnnotationField::CONTENT_CAPACITY + 1;
const ICON_NAME_ROW_BYTES: usize = IconNameField::CONTENT_CAPACITY + 1;
const COLOR_HEX_ROW_BYTES: usize = ColorHexField::CONTENT_CAPACITY + 1;

const FIX_TIME_US: i64 = 1_748_000_000_000_000;
const MARKER_ICON_WARNING_CODE: u8 = 4;

fn nul_padded_row(content: &[u8], row_bytes: usize) -> Vec<u8> {
    let mut row = content.to_vec();
    row.resize(row_bytes, 0);
    row
}

fn row_that_is_not_utf8(row_bytes: usize) -> Vec<u8> {
    nul_padded_row(&[0xff], row_bytes)
}

/// The six fixed-width field rows of the file [`gtd_bytes_with_field_rows`]
/// builds. [`Default`] fills each with a well-formed value.
struct FixedWidthFieldRows {
    marker_label: Vec<u8>,
    event_marker_variant_path: Vec<u8>,
    event_marker_annotation: Vec<u8>,
    style_variant_path: Vec<u8>,
    style_icon_name: Vec<u8>,
    style_color_hex: Vec<u8>,
}

impl Default for FixedWidthFieldRows {
    fn default() -> Self {
        Self {
            marker_label: nul_padded_row(b"start", MARKER_LABEL_ROW_BYTES),
            event_marker_variant_path: nul_padded_row(b"power/boot", VARIANT_PATH_ROW_BYTES),
            event_marker_annotation: nul_padded_row(b"battery replaced", ANNOTATION_ROW_BYTES),
            style_variant_path: nul_padded_row(b"power/boot", VARIANT_PATH_ROW_BYTES),
            style_icon_name: nul_padded_row(b"wrench", ICON_NAME_ROW_BYTES),
            style_color_hex: nul_padded_row(b"#FFAA00", COLOR_HEX_ROW_BYTES),
        }
    }
}

/// A `.gtd` file with one nav fix, one marker, one event marker and one event
/// marker style, whose fixed-width field rows are written as given. These tests
/// assemble the file themselves, since the writer cannot produce a malformed row.
#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn gtd_bytes_with_field_rows(rows: FixedWidthFieldRows) -> Vec<u8> {
    let FixedWidthFieldRows {
        marker_label,
        event_marker_variant_path,
        event_marker_annotation,
        style_variant_path,
        style_icon_name,
        style_color_hex,
    } = rows;

    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("2".into()));

    let mut nav_points = fb.create_group("nav_points");
    nav_points
        .create_dataset("time")
        .with_i64_data(&[FIX_TIME_US])
        .with_shape(&[1]);
    nav_points
        .create_dataset("lat")
        .with_f64_data(&[55.0])
        .with_shape(&[1]);
    nav_points
        .create_dataset("lon")
        .with_f64_data(&[12.0])
        .with_shape(&[1]);
    nav_points
        .create_dataset("heading")
        .with_f64_data(&[90.0])
        .with_shape(&[1]);
    nav_points
        .create_dataset("speed_mps")
        .with_f64_data(&[3.0])
        .with_shape(&[1]);
    fb.add_group(nav_points.finish());

    let mut markers = fb.create_group("markers");
    markers
        .create_dataset("time")
        .with_i64_data(&[FIX_TIME_US])
        .with_shape(&[1]);
    markers
        .create_dataset("lat")
        .with_f64_data(&[55.0])
        .with_shape(&[1]);
    markers
        .create_dataset("lon")
        .with_f64_data(&[12.0])
        .with_shape(&[1]);
    markers
        .create_dataset("icon")
        .with_u8_data(&[MARKER_ICON_WARNING_CODE])
        .with_shape(&[1]);
    markers
        .create_dataset("label")
        .with_u8_data(&marker_label)
        .with_shape(&[1, marker_label.len() as u64]);
    fb.add_group(markers.finish());

    let mut event_markers = fb.create_group("event_markers");
    event_markers
        .create_dataset("sys_time_us")
        .with_u64_data(&[FIX_TIME_US.cast_unsigned()])
        .with_shape(&[1]);
    event_markers
        .create_dataset("lat")
        .with_f64_data(&[55.0])
        .with_shape(&[1]);
    event_markers
        .create_dataset("lon")
        .with_f64_data(&[12.0])
        .with_shape(&[1]);
    event_markers
        .create_dataset("variant_path")
        .with_u8_data(&event_marker_variant_path)
        .with_shape(&[1, event_marker_variant_path.len() as u64]);
    event_markers
        .create_dataset("annotation")
        .with_u8_data(&event_marker_annotation)
        .with_shape(&[1, event_marker_annotation.len() as u64]);
    fb.add_group(event_markers.finish());

    let mut styles = fb.create_group("event_marker_styles");
    styles
        .create_dataset("variant_path")
        .with_u8_data(&style_variant_path)
        .with_shape(&[1, style_variant_path.len() as u64]);
    styles
        .create_dataset("icon_name")
        .with_u8_data(&style_icon_name)
        .with_shape(&[1, style_icon_name.len() as u64]);
    styles
        .create_dataset("color_hex")
        .with_u8_data(&style_color_hex)
        .with_shape(&[1, style_color_hex.len() as u64]);
    fb.add_group(styles.finish());

    fb.finish().expect("the assembled file builds")
}

#[test]
fn well_formed_fixed_width_field_rows_read_back() {
    let bytes = gtd_bytes_with_field_rows(FixedWidthFieldRows::default());
    let file = NavFile::read(bytes.as_slice()).expect("a file of well-formed field rows reads");

    let marker = file.markers().first().expect("the file holds the marker");
    assert_eq!(marker.annotation.label.as_deref(), Some("start"));

    let event_marker = file
        .event_markers()
        .first()
        .expect("the file holds the event marker");
    assert_eq!(event_marker.variant_path, "power/boot");
    assert_eq!(event_marker.annotation.as_deref(), Some("battery replaced"));

    let style = file
        .event_marker_styles()
        .first()
        .expect("the file holds the event marker style");
    assert_eq!(style.variant_path, "power/boot");
    assert_eq!(style.icon, EventMarkerIconChoice::Icon(MarkerIcon::Wrench));
    assert_eq!(style.color, EventMarkerColor::hex("#FFAA00"));
}

#[test]
fn an_icon_name_outside_the_known_set_reads_back_as_the_name_the_file_holds() {
    let bytes = gtd_bytes_with_field_rows(FixedWidthFieldRows {
        style_icon_name: nul_padded_row(b"hovercraft", ICON_NAME_ROW_BYTES),
        ..FixedWidthFieldRows::default()
    });

    let file = NavFile::read(bytes.as_slice()).expect("a well-formed icon name reads");

    let style = file
        .event_marker_styles()
        .first()
        .expect("the file holds the event marker style");
    assert_eq!(
        style.icon,
        EventMarkerIconChoice::Unrecognized("hovercraft".to_owned())
    );
}

#[test]
fn a_color_that_is_not_rrggbb_reads_back_as_the_value_the_file_holds() {
    let bytes = gtd_bytes_with_field_rows(FixedWidthFieldRows {
        style_color_hex: nul_padded_row(b"FFAA00", COLOR_HEX_ROW_BYTES),
        ..FixedWidthFieldRows::default()
    });

    let file = NavFile::read(bytes.as_slice()).expect("a well-formed color reads");

    let style = file
        .event_marker_styles()
        .first()
        .expect("the file holds the event marker style");
    assert_eq!(
        style.color,
        EventMarkerColor::Unrecognized("FFAA00".to_owned())
    );
}

#[rstest]
#[case::marker_label(
    |rows: &mut FixedWidthFieldRows| rows.marker_label = row_that_is_not_utf8(MARKER_LABEL_ROW_BYTES),
    "markers/label"
)]
#[case::event_marker_variant_path(
    |rows: &mut FixedWidthFieldRows| rows.event_marker_variant_path = row_that_is_not_utf8(VARIANT_PATH_ROW_BYTES),
    "event_markers/variant_path"
)]
#[case::event_marker_annotation(
    |rows: &mut FixedWidthFieldRows| rows.event_marker_annotation = row_that_is_not_utf8(ANNOTATION_ROW_BYTES),
    "event_markers/annotation"
)]
#[case::style_variant_path(
    |rows: &mut FixedWidthFieldRows| rows.style_variant_path = row_that_is_not_utf8(VARIANT_PATH_ROW_BYTES),
    "event_marker_styles/variant_path"
)]
#[case::style_icon_name(
    |rows: &mut FixedWidthFieldRows| rows.style_icon_name = row_that_is_not_utf8(ICON_NAME_ROW_BYTES),
    "event_marker_styles/icon_name"
)]
#[case::style_color_hex(
    |rows: &mut FixedWidthFieldRows| rows.style_color_hex = row_that_is_not_utf8(COLOR_HEX_ROW_BYTES),
    "event_marker_styles/color_hex"
)]
fn a_field_row_that_is_not_utf8_stops_the_read(
    #[case] make_row_invalid: fn(&mut FixedWidthFieldRows),
    #[case] expected_field: &str,
) {
    let mut rows = FixedWidthFieldRows::default();
    make_row_invalid(&mut rows);
    let bytes = gtd_bytes_with_field_rows(rows);

    let refusal = NavFile::read(bytes.as_slice())
        .expect_err("a field row that is not UTF-8 stops the read")
        .to_string();
    assert_eq!(
        refusal,
        format!(
            "{expected_field}: the field row is not UTF-8: invalid utf-8 sequence of 1 bytes from index 0"
        )
    );
}
