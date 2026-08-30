//! The fixed-width string fields of the event marker groups, written through
//! the paths that reach them without passing `EventMarker::builder().build()`.

use geotrace_sdk::{
    Angle, AnnotationField, DateTime, Duration, EventKind, EventMarker, EventMarkerColor,
    EventMarkerIconChoice, EventMarkerStyle, NavFile, NavFileBuilder, NavFix, NavRecorder, Utc,
    VariantPathField,
};

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
