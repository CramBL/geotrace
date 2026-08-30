use geotrace_sdk::{
    Angle, DateTime, Duration, EventKind, EventMarker, EventMarkerColor, EventMarkerError,
    EventMarkerIconChoice, EventMarkerStyle, MarkerIcon, NavFileBuilder, NavFix, Utc,
};
use rstest::rstest;

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

fn t(offset_secs: i64) -> DateTime<Utc> {
    base() + Duration::seconds(offset_secs)
}

fn fix(offset_secs: i64, lat: f64, lon: f64) -> NavFix {
    NavFix::builder()
        .gps_time(t(offset_secs))
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
        .sys_time(t(offset_secs))
        .build()
        .expect("test marker path should be valid")
}

// Validation - accepted paths
#[test]
fn valid_simple() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(marker("power/turn_on", 0));
}

#[test]
fn valid_kebab() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(marker("agps/request-epo/gps", 0));
}

#[test]
fn valid_single_segment() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(marker("boot", 0));
}

#[test]
fn valid_mixed_case_and_digits() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(marker("sensor/GPS3/lock", 0));
}

#[test]
fn valid_at_the_variant_path_capacity() {
    let path = "a".repeat(255);
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(marker(&path, 0));
}

// Validation - rejected paths (validation now happens in EventMarker::builder().build())
#[test]
fn empty_path_is_rejected() {
    let err = EventMarker::builder()
        .variant_path("")
        .sys_time(t(0))
        .build()
        .expect_err("should fail");
    assert!(matches!(err, EventMarkerError::Empty { .. }), "got {err}");
}

#[test]
fn leading_slash_is_rejected() {
    let err = EventMarker::builder()
        .variant_path("/power/on")
        .sys_time(t(0))
        .build()
        .expect_err("should fail");
    assert!(
        matches!(err, EventMarkerError::LeadingSlash { .. }),
        "got {err}"
    );
}

#[test]
fn trailing_slash_is_rejected() {
    let err = EventMarker::builder()
        .variant_path("power/on/")
        .sys_time(t(0))
        .build()
        .expect_err("should fail");
    assert!(
        matches!(err, EventMarkerError::TrailingSlash { .. }),
        "got {err}"
    );
}

#[test]
fn double_slash_is_rejected() {
    let err = EventMarker::builder()
        .variant_path("power//on")
        .sys_time(t(0))
        .build()
        .expect_err("should fail");
    assert!(
        matches!(err, EventMarkerError::EmptySegment { .. }),
        "got {err}"
    );
}

#[test]
fn a_variant_path_one_byte_past_the_capacity_is_rejected() {
    let path = "a".repeat(256);
    let err = EventMarker::builder()
        .variant_path(path.clone())
        .sys_time(t(0))
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
#[case::one_byte_past_the_capacity("a".repeat(512))]
#[case::a_multi_byte_character_straddling_the_capacity(format!("{}é", "a".repeat(510)))]
fn an_annotation_the_field_cannot_hold_is_rejected(#[case] annotation: String) {
    let err = EventMarker::builder()
        .variant_path("power/boot")
        .sys_time(t(0))
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

#[test]
fn space_in_path_is_rejected() {
    let err = EventMarker::builder()
        .variant_path("power/turn on")
        .sys_time(t(0))
        .build()
        .expect_err("should fail");
    assert!(
        matches!(err, EventMarkerError::InvalidChars { .. }),
        "got {err}"
    );
}

#[test]
fn dot_in_path_is_rejected() {
    let err = EventMarker::builder()
        .variant_path("power/v1.2")
        .sys_time(t(0))
        .build()
        .expect_err("should fail");
    assert!(
        matches!(err, EventMarkerError::InvalidChars { .. }),
        "got {err}"
    );
}

// Error messages include the offending path
#[test]
fn error_messages_include_path() {
    let cases: &[(&str, &str)] = &[
        ("", "\"\""),
        ("/leading", "\"/leading\""),
        ("trailing/", "\"trailing/\""),
        ("dbl//slash", "\"dbl//slash\""),
        ("a b", "\"a b\""),
    ];
    for (path, expected_fragment) in cases {
        let err = EventMarker::builder()
            .variant_path(*path)
            .sys_time(t(0))
            .build()
            .expect_err("should fail");
        let msg = err.to_string();
        assert!(
            msg.contains(expected_fragment),
            "error for {path:?} should contain {expected_fragment}, got: {msg}"
        );
    }
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

#[test]
fn annotation_is_preserved() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 55.0, 12.0));
    recorder.add_event_marker(
        EventMarker::builder()
            .variant_path("status/alert")
            .sys_time(t(0))
            .annotation("motor overtemp")
            .build()
            .unwrap(),
    );

    let nav_file = recorder.finish().unwrap();
    assert_eq!(
        nav_file.event_markers()[0].annotation.as_deref(),
        Some("motor overtemp")
    );
}

// Position interpolation
#[test]
fn position_interpolated_at_midpoint() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix(0, 10.0, 20.0));
    recorder.add_nav_fix(fix(100, 12.0, 24.0));
    recorder.add_event_marker(marker("sensor/mid", 50));

    let nav_file = recorder.finish().unwrap();
    let em = &nav_file.event_markers()[0];
    assert!(
        (em.lat.as_degrees() - 11.0).abs() < 1e-9,
        "lat should be 11.0, got {}",
        em.lat.as_degrees()
    );
    assert!(
        (em.lon.as_degrees() - 22.0).abs() < 1e-9,
        "lon should be 22.0, got {}",
        em.lon.as_degrees()
    );
}

#[test]
fn position_clamped_to_first_fix_when_before_track() {
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    recorder.add_nav_fix(fix(10, 55.0, 12.0));
    recorder.add_event_marker(marker("boot", 0));

    let nav_file = recorder.finish().unwrap();
    let em = &nav_file.event_markers()[0];
    assert!(
        (em.lat.as_degrees() - 55.0).abs() < 1e-9,
        "pre-track marker should be clamped to first fix"
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

// Enums used only by the icon tests below.
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
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), t(0));
    recorder.add_event(&IconOuter::Power(IconLeaf::Failed), t(1));

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
    recorder.add_event(&IconOuter::Power(IconLeaf::TurnOn), t(0));

    let nav_file = recorder.finish().unwrap();
    let bytes = {
        let mut v = Vec::new();
        nav_file.write(&mut v).unwrap();
        v
    };
    let loaded = geotrace_sdk::NavFile::read(bytes.as_slice()).unwrap();

    let styles = loaded.event_marker_styles();
    assert_eq!(styles.len(), 1);
    assert_eq!(
        styles[0].icon,
        EventMarkerIconChoice::Icon(MarkerIcon::Lightning),
        "Lightning icon must survive write/read round-trip"
    );
}
