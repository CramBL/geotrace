use std::fmt;
use std::fmt::{Debug, Display, Formatter};

use geotrace_sdk::{BuildError, EventKind, EventMarkerError, NavFile};
use geotrace_sdk_test_util as test_util;
use rstest::rstest;

#[derive(EventKind)]
#[event_kind(note = none)]
enum WordBoundaryEvent {
    GPS3Lock,
    GPSLock,
    HTTPError,
    V2Event,
    _Reserved,
}

#[derive(EventKind)]
#[event_kind(note = none)]
#[expect(
    non_camel_case_types,
    reason = "a keyword is a variant name only as a lower-case raw identifier"
)]
enum RawIdentifierEvent {
    r#type,
}

#[rstest]
#[case::an_acronym_then_a_word(&WordBoundaryEvent::HTTPError, "http_error")]
#[case::an_acronym_then_a_short_word(&WordBoundaryEvent::GPSLock, "gps_lock")]
#[case::an_acronym_then_a_digit(&WordBoundaryEvent::GPS3Lock, "gps3_lock")]
#[case::a_digit_then_a_capital(&WordBoundaryEvent::V2Event, "v2_event")]
#[case::a_leading_underscore(&WordBoundaryEvent::_Reserved, "_reserved")]
#[case::a_raw_identifier(&RawIdentifierEvent::r#type, "type")]
fn a_variant_name_derives_its_snake_case_segment(
    #[case] event: &dyn EventKind,
    #[case] expected_segment: &str,
) {
    assert_eq!(event.variant_path().as_deref(), Some(expected_segment));
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum RenamedEvent {
    #[event_kind(rename = "groesse")]
    Größe,
    #[event_kind(rename = "radio-scan")]
    Scan(WordBoundaryEvent),
}

#[rstest]
#[case::a_leaf(RenamedEvent::Größe, "groesse")]
#[case::a_delegating_variant(RenamedEvent::Scan(WordBoundaryEvent::GPSLock), "radio-scan/gps_lock")]
fn a_rename_replaces_the_segment_of_its_variant(
    #[case] event: RenamedEvent,
    #[case] expected_path: &str,
) {
    assert_eq!(event.variant_path().as_deref(), Some(expected_path));
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum LongOuterEvent {
    Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa(
        LongInnerEvent,
    ),
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum LongInnerEvent {
    Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
}

#[test]
fn finish_rejects_a_nested_variant_path_past_255_bytes() {
    let event = LongOuterEvent::Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa(
        LongInnerEvent::Bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
    );
    let error = record_one_event(&event).expect_err("the variant path is 256 bytes");
    assert!(
        matches!(
            error,
            BuildError::InvalidEventMarkerVariantPath {
                source: EventMarkerError::TooLong { len: 256, .. }
            }
        ),
        "{error:?}"
    );
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum RadioEvent {
    Scan(ScanEvent),
}

#[derive(EventKind)]
#[event_kind(note = none)]
enum ScanEvent {
    #[event_kind(skip)]
    Calibration,
}

#[test]
fn the_variant_path_of_an_outer_variant_is_none_for_a_skipped_inner_variant() {
    assert_eq!(
        RadioEvent::Scan(ScanEvent::Calibration).variant_path(),
        None
    );
}

#[derive(Debug, EventKind)]
#[event_kind(lax)]
enum TaggedEvent<'a, T: Debug> {
    #[expect(dead_code, reason = "the derived Debug note reads the field")]
    Label(&'a str),
    Reading(T),
}

#[rstest]
#[case::a_borrowed_field(TaggedEvent::Label("north gate"), "label")]
#[case::a_generic_field(TaggedEvent::Reading(7_u8), "reading")]
fn a_generic_enum_with_a_lifetime_derives_the_segment_of_each_variant(
    #[case] event: TaggedEvent<'_, u8>,
    #[case] expected_segment: &str,
) {
    assert_eq!(event.variant_path().as_deref(), Some(expected_segment));
}

#[derive(Debug, EventKind)]
enum DebugNoteEvent {
    #[expect(dead_code, reason = "the derived Debug note reads the field")]
    Reading { value: u8 },
}

#[derive(EventKind)]
#[event_kind(note = display)]
enum DisplayNoteEvent {
    Reading,
}

impl Display for DisplayNoteEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("reading of the test sensor")
    }
}

#[derive(Debug, EventKind)]
#[event_kind(note = none)]
enum NoNoteEvent {
    Reading,
}

#[rstest]
#[case::debug_by_default(
    record_one_event(&DebugNoteEvent::Reading { value: 7 }),
    Some("Reading { value: 7 }")
)]
#[case::display(
    record_one_event(&DisplayNoteEvent::Reading),
    Some("reading of the test sensor")
)]
#[case::none(record_one_event(&NoNoteEvent::Reading), None)]
#[case::debug_on_a_generic_enum(
    record_one_event(&TaggedEvent::Reading(7_u8)),
    Some("Reading(7)")
)]
fn the_note_mode_of_an_enum_sets_the_annotation_of_its_events(
    #[case] recording: Result<NavFile, BuildError>,
    #[case] expected_annotation: Option<&str>,
) {
    let nav_file = recording.expect("the recording builds");
    let annotations: Vec<Option<&str>> = nav_file
        .event_markers()
        .iter()
        .map(|event_marker| event_marker.annotation.as_deref())
        .collect();
    assert_eq!(annotations, [expected_annotation]);
}

fn record_one_event(event: &impl EventKind) -> Result<NavFile, BuildError> {
    let mut recorder = test_util::recorder_with_one_fix();
    recorder.add_event(event, test_util::base());
    recorder.finish()
}
