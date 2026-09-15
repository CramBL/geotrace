use geotrace_sdk::__private::Sealed;
use geotrace_sdk::{
    Angle, AnnotationField, BuildError, EventKind, EventMarker, EventMarkerColor, EventMarkerError,
    EventMarkerIconChoice, EventMarkerStyle, IconNameField, MarkerIcon, NavFileBuilder, NavFix,
    NavFixTime, NavRecorder, UnplacedRecordCounts, VariantPathError, VariantPathField,
};
use geotrace_sdk_test_util as test_util;
use rstest::rstest;

fn fix(offset_secs: i64, lat: f64, lon: f64) -> NavFix {
    NavFix::builder()
        .time(NavFixTime::Receiver(test_util::t_s(offset_secs)))
        .lat(Angle::degrees(lat))
        .lon(Angle::degrees(lon))
        .heading(Angle::degrees(0.0))
        .build()
}

fn marker(variant_path: &str, offset_secs: i64) -> EventMarker {
    #[expect(
        clippy::expect_used,
        reason = "test helper only called with valid paths"
    )]
    EventMarker::builder()
        .variant_path(variant_path)
        .sys_time(test_util::t_s(offset_secs))
        .build()
        .expect("test marker path should be valid")
}

#[rstest]
#[case::two_segments("power/turn_on")]
#[case::kebab_case_segments("agps/request-epo/gps")]
#[case::a_single_segment("boot")]
#[case::mixed_case_and_digits("sensor/GPS3/lock")]
#[case::at_the_field_capacity(&"a".repeat(VariantPathField::CONTENT_CAPACITY))]
fn a_well_formed_variant_path_is_accepted(#[case] path: &str) {
    let event_marker = EventMarker::builder()
        .variant_path(path)
        .sys_time(test_util::t_s(0))
        .build()
        .expect("the variant path is well formed");
    assert_eq!(event_marker.variant_path(), path);
}

#[rstest]
#[case::empty(
    "",
    |error: &VariantPathError| matches!(error, VariantPathError::Empty { .. }),
    r#"invalid event marker variant path "": path is empty"#
)]
#[case::a_leading_slash(
    "/power/on",
    |error: &VariantPathError| matches!(error, VariantPathError::LeadingSlash { .. }),
    r#"invalid event marker variant path "/power/on": starts with '/'"#
)]
#[case::a_trailing_slash(
    "power/on/",
    |error: &VariantPathError| matches!(error, VariantPathError::TrailingSlash { .. }),
    r#"invalid event marker variant path "power/on/": ends with '/'"#
)]
#[case::a_double_slash(
    "power//on",
    |error: &VariantPathError| matches!(error, VariantPathError::EmptySegment { .. }),
    r#"invalid event marker variant path "power//on": contains '//'"#
)]
#[case::a_space(
    "power/turn on",
    |error: &VariantPathError| matches!(error, VariantPathError::InvalidChars { .. }),
    r#"invalid event marker variant path "power/turn on": contains characters outside ASCII alphanumeric, hyphen, underscore, and slash"#
)]
#[case::a_dot(
    "power/v1.2",
    |error: &VariantPathError| matches!(error, VariantPathError::InvalidChars { .. }),
    r#"invalid event marker variant path "power/v1.2": contains characters outside ASCII alphanumeric, hyphen, underscore, and slash"#
)]
fn a_malformed_variant_path_is_rejected(
    #[case] path: &str,
    #[case] is_expected_error: fn(&VariantPathError) -> bool,
    #[case] expected_message: &str,
) {
    let error = EventMarker::builder()
        .variant_path(path)
        .sys_time(test_util::t_s(0))
        .build()
        .expect_err("the variant path is malformed");
    assert!(
        matches!(&error, EventMarkerError::InvalidVariantPath { source } if is_expected_error(source)),
        "got {error:?}"
    );
    assert_eq!(error.to_string(), expected_message);
}

#[test]
fn a_variant_path_one_byte_past_the_capacity_is_rejected() {
    let path = "a".repeat(VariantPathField::CONTENT_CAPACITY + 1);
    let err = EventMarker::builder()
        .variant_path(path.clone())
        .sys_time(test_util::t_s(0))
        .build()
        .expect_err("should fail");
    assert_eq!(
        err.to_string(),
        format!(
            "invalid event marker variant path {path:?}: 256 bytes, past the 255 bytes the field holds"
        )
    );
}

#[rstest]
#[case::one_byte_past_the_capacity("a".repeat(AnnotationField::CONTENT_CAPACITY + 1))]
#[case::a_multi_byte_character_straddling_the_capacity(
    format!("{}é", "a".repeat(AnnotationField::CONTENT_CAPACITY - 1))
)]
fn an_annotation_the_field_cannot_hold_is_rejected(#[case] annotation: String) {
    let err = EventMarker::builder()
        .variant_path("power/boot")
        .sys_time(test_util::t_s(0))
        .annotation(annotation.clone())
        .build()
        .expect_err("should fail");
    assert_eq!(
        err.to_string(),
        format!(
            "invalid event marker annotation: {annotation:?} is 512 bytes, past the 511 bytes the field holds"
        )
    );
}

// Builder - counts and round-trip
#[test]
fn markers_are_stored_in_nav_file() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 10.0, 20.0));
    recorder.add_nav_fix(fix(100, 12.0, 22.0));
    recorder.add_event_marker(marker("power/on", 0));
    recorder.add_event_marker(marker("power/off", 100));

    let nav_file = recorder.finish().unwrap();
    assert_eq!(nav_file.event_markers().len(), 2);
    assert_eq!(nav_file.event_markers()[0].variant_path, "power/on");
    assert_eq!(nav_file.event_markers()[1].variant_path, "power/off");
}

// Position interpolation
#[rstest]
#[case::at_the_first_fix_time(0, 10.0, 20.0)]
#[case::halfway_between_two_fixes(50, 11.0, 22.0)]
#[case::at_the_last_fix_time(100, 12.0, 24.0)]
fn an_event_marker_within_the_fix_time_span_takes_the_position_at_its_time(
    #[case] marker_offset_secs: i64,
    #[case] expected_lat_deg: f64,
    #[case] expected_lon_deg: f64,
) {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 10.0, 20.0));
    recorder.add_nav_fix(fix(100, 12.0, 24.0));
    recorder.add_event_marker(marker("sensor/sample", marker_offset_secs));

    let nav_file = recorder.finish().unwrap();
    let event_marker = &nav_file.event_markers()[0];
    assert!(
        (event_marker.lat.as_degrees() - expected_lat_deg).abs() < 1e-9,
        "lat is {}, expected {expected_lat_deg}",
        event_marker.lat.as_degrees()
    );
    assert!(
        (event_marker.lon.as_degrees() - expected_lon_deg).abs() < 1e-9,
        "lon is {}, expected {expected_lon_deg}",
        event_marker.lon.as_degrees()
    );
}

#[test]
fn an_event_marker_between_two_fixes_in_host_clock_order_is_placed_between_them() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(10, 12.0, 24.0));
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Both {
                gps: test_util::t_s(12),
                sys: test_util::t_s(8),
            })
            .lat(Angle::degrees(10.0))
            .lon(Angle::degrees(20.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_event_marker(marker("sensor/sample", 9));

    let nav_file = recorder.finish().unwrap();
    let event_marker = &nav_file.event_markers()[0];
    assert!(
        (event_marker.lat.as_degrees() - 11.0).abs() < 1e-9,
        "lat is {}, expected 11",
        event_marker.lat.as_degrees()
    );
    assert!(
        (event_marker.lon.as_degrees() - 22.0).abs() < 1e-9,
        "lon is {}, expected 22",
        event_marker.lon.as_degrees()
    );
}

#[test]
fn an_event_marker_between_two_fixes_across_the_antimeridian_is_placed_on_the_short_arc() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 0.0, 179.95));
    recorder.add_nav_fix(fix(10, 0.0, -179.95));
    recorder.add_event_marker(marker("navigation/antimeridian", 5));

    let nav_file = recorder.finish().unwrap();
    let lon_deg = nav_file.event_markers()[0].lon.as_degrees();
    assert!(
        (lon_deg - (-180.0)).abs() < 1e-9,
        "lon is {lon_deg}, expected -180"
    );
}

#[rstest]
#[case::before_the_first_fix(0, 55.0)]
#[case::after_the_last_fix(30, 56.0)]
fn an_event_marker_outside_the_fix_time_range_is_clamped_to_the_endpoint_in_lenient_mode(
    #[case] marker_offset_secs: i64,
    #[case] expected_lat_deg: f64,
) {
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    recorder.add_nav_fix(fix(10, 55.0, 12.0));
    recorder.add_nav_fix(fix(20, 56.0, 13.0));
    recorder.add_event_marker(marker("boot", marker_offset_secs));

    let nav_file = recorder.finish().unwrap();
    let em = &nav_file.event_markers()[0];
    assert!(
        (em.lat.as_degrees() - expected_lat_deg).abs() < 1e-9,
        "lat is {}, expected {expected_lat_deg}",
        em.lat.as_degrees()
    );
}

#[rstest]
#[case::before_the_first_fix(0)]
#[case::after_the_last_fix(30)]
fn an_event_marker_outside_the_fix_time_range_fails_the_build_in_strict_mode(
    #[case] marker_offset_secs: i64,
) {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(10, 55.0, 12.0));
    recorder.add_nav_fix(fix(20, 56.0, 13.0));
    recorder.add_event_marker(marker("boot", marker_offset_secs));

    let error = recorder
        .finish()
        .expect_err("the marker is outside the range");
    assert!(
        matches!(error, BuildError::EventMarkersOutsideRange { count: 1 }),
        "got {error:?}"
    );
    assert_eq!(
        error.to_string(),
        "1 event marker(s) fall outside the nav fix time range"
    );
}

#[rstest]
#[case::strict(NavFileBuilder::new())]
#[case::lenient(NavFileBuilder::new().with_lenient_errors())]
fn an_event_marker_without_any_nav_fix_fails_the_build(#[case] builder: NavFileBuilder) {
    let mut recorder = builder.open();
    recorder.add_event_marker(marker("boot", 0));

    let error = recorder
        .finish()
        .expect_err("a marker needs a fix to place it");
    assert!(
        matches!(
            error,
            BuildError::NoNavFixes(UnplacedRecordCounts {
                event_markers: 1,
                ..
            })
        ),
        "got {error:?}"
    );
}

// Styles
#[test]
fn a_style_with_a_well_formed_path_and_color_round_trips() {
    let style = EventMarkerStyle::builder()
        .variant_path("power/on")
        .icon(EventMarkerIconChoice::Icon(MarkerIcon::Lightning))
        .color("#FFAA00")
        .build()
        .expect("the variant path and the color are well formed");
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(marker("power/on", 0));
    recorder.add_event_marker_style(style.clone());

    let loaded = test_util::round_trip(&recorder.finish().unwrap()).unwrap();

    assert_eq!(loaded.event_marker_styles(), [style]);
}

#[test]
fn an_empty_style_color_is_auto() {
    let style = EventMarkerStyle::builder()
        .variant_path("power/on")
        .color("")
        .build()
        .expect("an empty color is accepted");
    assert_eq!(style.color(), &EventMarkerColor::Auto);
}

#[rstest]
#[case::a_color_name("red")]
#[case::hex_digits_without_the_hash("FF9900")]
#[case::whitespace_only("   ")]
fn a_style_color_outside_the_rrggbb_form_is_rejected(#[case] color: &str) {
    let error = EventMarkerStyle::builder()
        .variant_path("power/on")
        .color(color)
        .build()
        .expect_err("the color is not of the #RRGGBB form");
    assert_eq!(
        error.to_string(),
        format!("invalid event marker color {color:?}: expected the #RRGGBB form")
    );
}

#[rstest]
#[case::empty("")]
#[case::a_non_ascii_character("über_lang")]
#[case::a_nul_byte("power/\0boot")]
#[case::one_byte_past_the_capacity(&"a".repeat(VariantPathField::CONTENT_CAPACITY + 1))]
fn a_style_variant_path_is_rejected_by_the_event_marker_rules(#[case] variant_path: &str) {
    let marker_error = EventMarker::builder()
        .variant_path(variant_path)
        .sys_time(test_util::t_s(0))
        .build()
        .expect_err("the event marker rules reject the variant path");
    let style_error = EventMarkerStyle::builder()
        .variant_path(variant_path)
        .build()
        .expect_err("the style takes the event marker rules");
    assert_eq!(style_error.to_string(), marker_error.to_string());
}

#[rstest]
#[case::one_byte_past_the_capacity(
    &"a".repeat(IconNameField::CONTENT_CAPACITY + 1),
    "is 32 bytes, past the 31 bytes the field holds"
)]
#[case::a_nul_byte("hover\0craft", "has a nul byte at offset 5")]
fn a_style_icon_name_that_does_not_fit_the_field_is_rejected(
    #[case] icon_name: &str,
    #[case] expected_reason: &str,
) {
    let error = EventMarkerStyle::builder()
        .variant_path("power/on")
        .icon(EventMarkerIconChoice::Unrecognized(icon_name.to_owned()))
        .build()
        .expect_err("the icon_name field cannot hold the name");
    assert_eq!(
        error.to_string(),
        format!("invalid event marker icon name: {icon_name:?} {expected_reason}")
    );
}

#[rstest]
#[case::an_unrecognized_name_at_the_field_capacity(
    EventMarkerIconChoice::Unrecognized("a".repeat(IconNameField::CONTENT_CAPACITY)),
    EventMarkerIconChoice::Unrecognized("a".repeat(IconNameField::CONTENT_CAPACITY))
)]
#[case::the_name_of_a_known_icon(
    EventMarkerIconChoice::Unrecognized("wrench".to_owned()),
    EventMarkerIconChoice::Icon(MarkerIcon::Wrench)
)]
#[case::an_empty_name(
    EventMarkerIconChoice::Unrecognized(String::new()),
    EventMarkerIconChoice::Auto
)]
fn a_style_icon_is_built_as_the_reader_returns_it(
    #[case] icon: EventMarkerIconChoice,
    #[case] expected_icon: EventMarkerIconChoice,
) {
    let style = EventMarkerStyle::builder()
        .variant_path("power/on")
        .icon(icon)
        .build()
        .expect("the icon_name field holds the name");
    assert_eq!(style.icon(), &expected_icon);
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker_style(style.clone());

    let loaded = test_util::round_trip(&recorder.finish().unwrap()).unwrap();

    assert_eq!(loaded.event_marker_styles(), [style]);
}

// The `enum` types used only by the icon tests below.
#[derive(Debug, EventKind)]
#[event_kind(note = none)]
enum IconLeaf {
    #[event_kind(icon = Error)]
    Failed,
    #[event_kind(icon = Lightning)]
    TurnOn,
}

#[derive(Debug, EventKind)]
#[event_kind(note = none)]
enum IconOuter {
    Power(IconLeaf),
}

#[test]
fn add_event_auto_registers_icon_for_derived_enum() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_nav_fix(fix(1, 55.1, 12.1));
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(0));
    recorder.add_event(&IconOuter::Power(IconLeaf::Failed), test_util::t_s(1));

    let nav_file = recorder.finish().unwrap();
    let styles = nav_file.event_marker_styles();

    assert_eq!(styles.len(), 2, "expected one style per unique path");

    let turn_on = styles.iter().find(|s| s.variant_path() == "power/turn_on");
    assert!(turn_on.is_some(), "no style registered for power/turn_on");
    assert_eq!(
        turn_on.unwrap().icon(),
        &EventMarkerIconChoice::Icon(MarkerIcon::Lightning),
        "power/turn_on should have Lightning icon"
    );

    let failed = styles.iter().find(|s| s.variant_path() == "power/failed");
    assert!(failed.is_some(), "no style registered for power/failed");
    assert_eq!(
        failed.unwrap().icon(),
        &EventMarkerIconChoice::Icon(MarkerIcon::Error),
        "power/failed should have Error icon"
    );
}

#[test]
fn add_event_icon_survives_round_trip() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(0));

    let loaded = test_util::round_trip(&recorder.finish().unwrap()).unwrap();

    let styles = loaded.event_marker_styles();
    assert_eq!(styles.len(), 1);
    assert_eq!(
        styles[0].icon(),
        &EventMarkerIconChoice::Icon(MarkerIcon::Lightning),
        "Lightning icon must survive write/read round-trip"
    );
}

#[rstest]
#[case::style_before_the_events(record_the_style_before_the_events)]
#[case::style_between_the_events(record_the_style_between_the_events)]
fn an_explicit_style_wins_over_the_derived_icon_of_its_path(
    #[case] record: fn(&mut NavRecorder, EventMarkerStyle),
) {
    let style = EventMarkerStyle::builder()
        .variant_path("power/turn_on")
        .icon(EventMarkerIconChoice::Icon(MarkerIcon::Check))
        .color("#00FF00")
        .build()
        .expect("the variant path and the color are well formed");
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_nav_fix(fix(1, 55.1, 12.1));
    record(&mut recorder, style.clone());

    let loaded = test_util::round_trip(&recorder.finish().unwrap()).unwrap();

    assert_eq!(loaded.event_marker_styles(), [style]);
}

#[test]
fn a_later_explicit_style_replaces_an_earlier_one_for_its_path() {
    let earlier = EventMarkerStyle::builder()
        .variant_path("power/on")
        .icon(EventMarkerIconChoice::Icon(MarkerIcon::Warning))
        .color("#FF9900")
        .build()
        .expect("the variant path and the color are well formed");
    let later = EventMarkerStyle::builder()
        .variant_path("power/on")
        .icon(EventMarkerIconChoice::Icon(MarkerIcon::Check))
        .color("#00FF00")
        .build()
        .expect("the variant path and the color are well formed");
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker_style(earlier);
    recorder.add_event_marker_style(later.clone());

    let loaded = test_util::round_trip(&recorder.finish().unwrap()).unwrap();

    assert_eq!(loaded.event_marker_styles(), [later]);
}

#[test]
fn the_recorder_writes_the_styles_in_variant_path_order() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_nav_fix(fix(1, 55.1, 12.1));
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(0));
    for variant_path in ["power/sleep", "power/boot"] {
        recorder.add_event_marker_style(
            EventMarkerStyle::builder()
                .variant_path(variant_path)
                .build()
                .expect("the variant path is well formed"),
        );
    }
    recorder.add_event(&IconOuter::Power(IconLeaf::Failed), test_util::t_s(1));

    let loaded = test_util::round_trip(&recorder.finish().unwrap()).unwrap();

    let written_paths: Vec<&str> = loaded
        .event_marker_styles()
        .iter()
        .map(EventMarkerStyle::variant_path)
        .collect();
    assert_eq!(
        written_paths,
        ["power/boot", "power/failed", "power/sleep", "power/turn_on"]
    );
}

struct HandWrittenEvent {
    variant_path: &'static str,
}

impl Sealed for HandWrittenEvent {}

impl EventKind for HandWrittenEvent {
    fn variant_path(&self) -> Option<String> {
        Some(self.variant_path.to_owned())
    }

    fn marker_icon(&self) -> Option<MarkerIcon> {
        Some(MarkerIcon::Warning)
    }
}

#[rstest]
#[case::a_leading_slash(
    "/power/boot",
    |error: &VariantPathError| matches!(error, VariantPathError::LeadingSlash { .. })
)]
#[case::an_empty_segment(
    "power//boot",
    |error: &VariantPathError| matches!(error, VariantPathError::EmptySegment { .. })
)]
#[case::a_non_ascii_character(
    "power/größe",
    |error: &VariantPathError| matches!(error, VariantPathError::InvalidChars { .. })
)]
fn an_event_with_a_malformed_variant_path_fails_the_build_in_strict_mode(
    #[case] variant_path: &'static str,
    #[case] is_expected_rejection: fn(&VariantPathError) -> bool,
    #[values(record_through_add_event, record_through_add_event_with_note)] record_event: fn(
        &mut NavRecorder,
        &HandWrittenEvent,
    ),
) {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    record_event(&mut recorder, &HandWrittenEvent { variant_path });

    let error = recorder
        .finish()
        .expect_err("the variant path is malformed");
    assert!(
        matches!(
            &error,
            BuildError::InvalidEventMarkerVariantPath { source } if is_expected_rejection(source)
        ),
        "got {error:?}"
    );
}

#[rstest]
#[case::a_leading_slash("/power/boot")]
#[case::an_empty_segment("power//boot")]
#[case::a_non_ascii_character("power/größe")]
fn an_event_with_a_malformed_variant_path_is_dropped_in_lenient_mode(
    #[case] variant_path: &'static str,
    #[values(record_through_add_event, record_through_add_event_with_note)] record_event: fn(
        &mut NavRecorder,
        &HandWrittenEvent,
    ),
) {
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_nav_fix(fix(2, 55.2, 12.2));
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(0));
    record_event(&mut recorder, &HandWrittenEvent { variant_path });
    recorder.add_event(&IconOuter::Power(IconLeaf::Failed), test_util::t_s(2));

    let nav_file = recorder.finish().expect("lenient mode keeps the recording");
    let recorded_paths: Vec<&str> = nav_file
        .event_markers()
        .iter()
        .map(|event_marker| event_marker.variant_path.as_str())
        .collect();
    assert_eq!(recorded_paths, ["power/turn_on", "power/failed"]);
    let registered_icons: Vec<(&str, &EventMarkerIconChoice)> = nav_file
        .event_marker_styles()
        .iter()
        .map(|style| (style.variant_path(), style.icon()))
        .collect();
    assert_eq!(
        registered_icons,
        [
            (
                "power/failed",
                &EventMarkerIconChoice::Icon(MarkerIcon::Error)
            ),
            (
                "power/turn_on",
                &EventMarkerIconChoice::Icon(MarkerIcon::Lightning)
            ),
        ]
    );
}

fn record_the_style_before_the_events(recorder: &mut NavRecorder, style: EventMarkerStyle) {
    recorder.add_event_marker_style(style);
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(0));
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(1));
}

fn record_the_style_between_the_events(recorder: &mut NavRecorder, style: EventMarkerStyle) {
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(0));
    recorder.add_event_marker_style(style);
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), test_util::t_s(1));
}

fn record_through_add_event(recorder: &mut NavRecorder, event: &HandWrittenEvent) {
    recorder.add_event(event, test_util::t_s(0));
}

fn record_through_add_event_with_note(recorder: &mut NavRecorder, event: &HandWrittenEvent) {
    recorder.add_event_with_note(event, test_util::t_s(0), "note");
}
