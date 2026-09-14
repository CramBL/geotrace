use geotrace_sdk::{
    Angle, AnnotationField, BuildError, EventKind, EventMarker, EventMarkerColor, EventMarkerError,
    EventMarkerIconChoice, EventMarkerStyle, MarkerIcon, NavFileBuilder, NavFix, NavFixTime,
    UnplacedRecordCounts, VariantPathField,
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
    |error: &EventMarkerError| matches!(error, EventMarkerError::Empty { .. }),
    r#"invalid event marker variant path "": path is empty"#
)]
#[case::a_leading_slash(
    "/power/on",
    |error: &EventMarkerError| matches!(error, EventMarkerError::LeadingSlash { .. }),
    r#"invalid event marker variant path "/power/on": starts with '/'"#
)]
#[case::a_trailing_slash(
    "power/on/",
    |error: &EventMarkerError| matches!(error, EventMarkerError::TrailingSlash { .. }),
    r#"invalid event marker variant path "power/on/": ends with '/'"#
)]
#[case::a_double_slash(
    "power//on",
    |error: &EventMarkerError| matches!(error, EventMarkerError::EmptySegment { .. }),
    r#"invalid event marker variant path "power//on": contains '//'"#
)]
#[case::a_space(
    "power/turn on",
    |error: &EventMarkerError| matches!(error, EventMarkerError::InvalidChars { .. }),
    r#"invalid event marker variant path "power/turn on": contains characters outside ASCII alphanumeric, hyphen, underscore, and slash"#
)]
#[case::a_dot(
    "power/v1.2",
    |error: &EventMarkerError| matches!(error, EventMarkerError::InvalidChars { .. }),
    r#"invalid event marker variant path "power/v1.2": contains characters outside ASCII alphanumeric, hyphen, underscore, and slash"#
)]
fn a_malformed_variant_path_is_rejected(
    #[case] path: &str,
    #[case] is_expected_error: fn(&EventMarkerError) -> bool,
    #[case] expected_message: &str,
) {
    let error = EventMarker::builder()
        .variant_path(path)
        .sys_time(test_util::t_s(0))
        .build()
        .expect_err("the variant path is malformed");
    assert!(is_expected_error(&error), "got {error:?}");
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
fn event_marker_styles_are_stored() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(marker("power/on", 0));
    recorder.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/on")
            .icon(EventMarkerIconChoice::Icon(MarkerIcon::Lightning))
            .color("#FFAA00")
            .build()
            .expect("valid hex color"),
    );

    let nav_file = recorder.finish().unwrap();
    assert_eq!(nav_file.event_marker_styles().len(), 1);
    assert_eq!(
        nav_file.event_marker_styles()[0].icon,
        EventMarkerIconChoice::Icon(MarkerIcon::Lightning)
    );
    assert_eq!(
        nav_file.event_marker_styles()[0].color,
        EventMarkerColor::hex("#FFAA00")
    );
}

// The `enum` types used only by the icon tests below.
#[derive(Debug, EventKind)]
#[event_kind(note = none)]
enum IconLeaf {
    #[event_kind(icon = Lightning)]
    TurnOn,
    #[event_kind(icon = Error)]
    Failed,
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

    let turn_on = styles.iter().find(|s| s.variant_path == "power/turn_on");
    assert!(turn_on.is_some(), "no style registered for power/turn_on");
    assert_eq!(
        turn_on.unwrap().icon,
        EventMarkerIconChoice::Icon(MarkerIcon::Lightning),
        "power/turn_on should have Lightning icon"
    );

    let failed = styles.iter().find(|s| s.variant_path == "power/failed");
    assert!(failed.is_some(), "no style registered for power/failed");
    assert_eq!(
        failed.unwrap().icon,
        EventMarkerIconChoice::Icon(MarkerIcon::Error),
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
        styles[0].icon,
        EventMarkerIconChoice::Icon(MarkerIcon::Lightning),
        "Lightning icon must survive write/read round-trip"
    );
}

#[test]
fn an_icon_name_and_color_outside_the_known_sets_are_written_back_verbatim() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker_style(EventMarkerStyle {
        variant_path: "power/on".to_owned(),
        icon: EventMarkerIconChoice::Unrecognized("hovercraft".to_owned()),
        color: EventMarkerColor::Unrecognized("FFAA00".to_owned()),
    });

    let loaded = test_util::round_trip(&recorder.finish().unwrap()).unwrap();

    let styles = loaded.event_marker_styles();
    assert_eq!(
        styles[0].icon,
        EventMarkerIconChoice::Unrecognized("hovercraft".to_owned())
    );
    assert_eq!(
        styles[0].color,
        EventMarkerColor::Unrecognized("FFAA00".to_owned())
    );
}
