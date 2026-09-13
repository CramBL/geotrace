//! The fixed-width string fields of the `.gtd` format: the builder and the
//! writer rejecting a value past a field's capacity, the reader rejecting a field
//! row that is not UTF-8, and the reader preserving a well-formed value it does
//! not recognize. The write tests reach a field by a path that skips the checks
//! in `EventMarker::builder().build()`.

use geotrace_sdk::{
    Annotation, AnnotationField, EventKind, EventMarker, EventMarkerColor, EventMarkerIconChoice,
    EventMarkerStyle, MarkerIcon, MarkerLabelField, NavFile, NavRecorder, VariantPathField,
};
use geotrace_sdk_test_util as test_util;
use geotrace_sdk_test_util::{
    ANNOTATION_ROW_BYTES, COLOR_HEX_ROW_BYTES, EventMarkerFieldRows, GtdFileContents,
    ICON_NAME_ROW_BYTES, Lat, Lon, MARKER_LABEL_ROW_BYTES, StyleFieldRows, VARIANT_PATH_ROW_BYTES,
};
use rstest::rstest;

fn recorder_with_fixes_bracketing_an_annotation() -> NavRecorder {
    let mut recorder = test_util::recorder_with_one_fix();
    recorder.add_nav_fix(test_util::fix_at(10_000, Lat(55.1), Lon(12.1)));
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
    let mut recorder = test_util::recorder_with_one_fix();
    recorder.add_event_marker(
        EventMarker::builder()
            .variant_path(variant_path.clone())
            .sys_time(test_util::t_s(0))
            .annotation(annotation.clone())
            .build()
            .expect("a value at the field capacity is accepted"),
    );
    let loaded = test_util::round_trip(&recorder.finish().expect("the recording builds"))
        .expect("a value at the field capacity is written and read back");
    let marker = loaded
        .event_markers()
        .first()
        .expect("the file holds the marker");
    assert_eq!(marker.variant_path, variant_path);
    assert_eq!(marker.annotation.as_deref(), Some(annotation.as_str()));
}

#[rstest]
#[case::at_the_field_capacity(Some("l".repeat(MarkerLabelField::CONTENT_CAPACITY)))]
#[case::multibyte_utf8(Some("日本語テスト 🌍".to_owned()))]
#[case::no_label(None)]
fn a_marker_label_round_trips(#[case] label: Option<String>) {
    let mut recorder = recorder_with_fixes_bracketing_an_annotation();
    recorder.add_annotation(
        Annotation::builder()
            .time(test_util::t_s(5))
            .maybe_label(label.clone())
            .build()
            .expect("a label within the field capacity is accepted"),
    );
    let loaded = test_util::round_trip(&recorder.finish().expect("the recording builds"))
        .expect("a label within the field capacity is written and read back");
    let marker = loaded.markers().first().expect("the file holds the marker");
    assert_eq!(marker.annotation.label(), label.as_deref());
}

#[rstest]
#[case::one_ascii_byte_past_the_capacity("l".repeat(MarkerLabelField::CONTENT_CAPACITY + 1))]
#[case::a_two_byte_character_across_the_capacity(
    format!("{}é", "l".repeat(MarkerLabelField::CONTENT_CAPACITY - 1))
)]
fn a_label_past_the_field_capacity_is_rejected(#[case] label: String) {
    let error_message = Annotation::builder()
        .time(test_util::t_s(0))
        .label(label.clone())
        .build()
        .expect_err("a label past the field capacity is rejected")
        .to_string();
    assert_eq!(
        error_message,
        format!("markers/label: {label:?} is 256 bytes, past the 255 bytes the field holds")
    );
}

#[test]
fn a_note_one_byte_past_the_annotation_capacity_stops_the_write() {
    let note = "n".repeat(AnnotationField::CONTENT_CAPACITY + 1);
    let mut recorder = test_util::recorder_with_one_fix();
    recorder.add_event_with_note(&PowerEvent::Boot, test_util::t_s(0), note.clone());

    let error_message = recorder
        .finish()
        .expect("the recording builds")
        .write(Vec::new())
        .expect_err("a note past the field capacity stops the write")
        .to_string();
    assert_eq!(
        error_message,
        format!(
            "event_markers/annotation: {note:?} is 512 bytes, past the 511 bytes the field holds"
        )
    );
}

#[test]
fn a_style_variant_path_one_byte_past_the_capacity_stops_the_write() {
    let variant_path = "a".repeat(VariantPathField::CONTENT_CAPACITY + 1);
    let mut recorder = test_util::recorder_with_one_fix();
    recorder.add_event_marker_style(EventMarkerStyle {
        variant_path: variant_path.clone(),
        icon: EventMarkerIconChoice::Auto,
        color: EventMarkerColor::Auto,
    });

    let error_message = recorder
        .finish()
        .expect("the recording builds")
        .write(Vec::new())
        .expect_err("a variant path past the field capacity stops the write")
        .to_string();
    assert_eq!(
        error_message,
        format!(
            "event_marker_styles/variant_path: {variant_path:?} is 256 bytes, past the 255 bytes the field holds"
        )
    );
}

#[test]
fn a_style_color_one_byte_past_the_capacity_stops_the_write() {
    let mut recorder = test_util::recorder_with_one_fix();
    recorder.add_event_marker_style(EventMarkerStyle {
        variant_path: "power/boot".to_owned(),
        icon: EventMarkerIconChoice::Auto,
        color: EventMarkerColor::hex("#FFAA001"),
    });

    let error_message = recorder
        .finish()
        .expect("the recording builds")
        .write(Vec::new())
        .expect_err("a color past the field capacity stops the write")
        .to_string();
    assert_eq!(
        error_message,
        "event_marker_styles/color_hex: \"#FFAA001\" is 8 bytes, past the 7 bytes the field holds"
    );
}

fn row_that_is_not_utf8(row_bytes: usize) -> Vec<u8> {
    test_util::nul_padded_row(&[0xff], row_bytes)
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
            marker_label: test_util::nul_padded_row(b"start", MARKER_LABEL_ROW_BYTES),
            event_marker_variant_path: test_util::nul_padded_row(
                b"power/boot",
                VARIANT_PATH_ROW_BYTES,
            ),
            event_marker_annotation: test_util::nul_padded_row(
                b"battery replaced",
                ANNOTATION_ROW_BYTES,
            ),
            style_variant_path: test_util::nul_padded_row(b"power/boot", VARIANT_PATH_ROW_BYTES),
            style_icon_name: test_util::nul_padded_row(b"wrench", ICON_NAME_ROW_BYTES),
            style_color_hex: test_util::nul_padded_row(b"#FFAA00", COLOR_HEX_ROW_BYTES),
        }
    }
}

/// A `.gtd` file with one nav fix, one marker, one event marker and one event
/// marker style, whose fixed-width field rows are written as given.
fn gtd_bytes_with_field_rows(
    FixedWidthFieldRows {
        marker_label,
        event_marker_variant_path,
        event_marker_annotation,
        style_variant_path,
        style_icon_name,
        style_color_hex,
    }: FixedWidthFieldRows,
) -> Vec<u8> {
    GtdFileContents {
        attrs: Vec::new(),
        marker_labels: vec![marker_label],
        event_markers: vec![EventMarkerFieldRows {
            variant_path: event_marker_variant_path,
            annotation: event_marker_annotation,
        }],
        styles: vec![StyleFieldRows {
            variant_path: style_variant_path,
            icon_name: style_icon_name,
            color_hex: style_color_hex,
        }],
    }
    .into_gtd_bytes()
}

#[test]
fn well_formed_fixed_width_field_rows_read_back() {
    let bytes = gtd_bytes_with_field_rows(FixedWidthFieldRows::default());
    let file = NavFile::read(bytes.as_slice()).expect("a file of well-formed field rows reads");

    let marker = file.markers().first().expect("the file holds the marker");
    assert_eq!(marker.annotation.label(), Some("start"));

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
fn an_icon_name_outside_the_known_set_survives_the_read() {
    let bytes = gtd_bytes_with_field_rows(FixedWidthFieldRows {
        style_icon_name: test_util::nul_padded_row(b"hovercraft", ICON_NAME_ROW_BYTES),
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
fn a_color_that_is_not_rrggbb_survives_the_read() {
    let bytes = gtd_bytes_with_field_rows(FixedWidthFieldRows {
        style_color_hex: test_util::nul_padded_row(b"FFAA00", COLOR_HEX_ROW_BYTES),
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

    let error_message = NavFile::read(bytes.as_slice())
        .expect_err("a field row that is not UTF-8 stops the read")
        .to_string();
    assert_eq!(
        error_message,
        format!(
            "{expected_field}: the field row is not UTF-8: invalid utf-8 sequence of 1 bytes from index 0"
        )
    );
}
