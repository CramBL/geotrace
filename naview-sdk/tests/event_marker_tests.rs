use naview_sdk::{
    Angle, DateTime, Duration, EventMarker, EventMarkerError, EventMarkerStyle, NavFileBuilder,
    NavFix, Utc, degree,
};

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
        .lat(Angle::new::<degree>(lat))
        .lon(Angle::new::<degree>(lon))
        .heading(Angle::new::<degree>(0.0))
        .build()
}

fn marker(variant_path: &str, offset_secs: i64) -> EventMarker {
    EventMarker::builder()
        .variant_path(variant_path.to_owned())
        .sys_time(t(offset_secs))
        .build()
}

// Validation — accepted paths

#[test]
fn valid_simple() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    b.add_event_marker(marker("power/turn_on", 0)).unwrap();
}

#[test]
fn valid_kebab() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    b.add_event_marker(marker("agps/request-epo/gps", 0))
        .unwrap();
}

#[test]
fn valid_single_segment() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    b.add_event_marker(marker("boot", 0)).unwrap();
}

#[test]
fn valid_mixed_case_and_digits() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    b.add_event_marker(marker("sensor/GPS3/lock", 0)).unwrap();
}

#[test]
fn valid_exactly_256_bytes() {
    let path = "a".repeat(256);
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    b.add_event_marker(marker(&path, 0)).unwrap();
}

// Validation — rejected paths

#[test]
fn empty_path_is_rejected() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    let err = b
        .add_event_marker(marker("", 0))
        .err()
        .expect("should fail");
    assert!(matches!(err, EventMarkerError::Empty { .. }), "got {err}");
}

#[test]
fn leading_slash_is_rejected() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    let err = b
        .add_event_marker(marker("/power/on", 0))
        .err()
        .expect("should fail");
    assert!(
        matches!(err, EventMarkerError::LeadingSlash { .. }),
        "got {err}"
    );
}

#[test]
fn trailing_slash_is_rejected() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    let err = b
        .add_event_marker(marker("power/on/", 0))
        .err()
        .expect("should fail");
    assert!(
        matches!(err, EventMarkerError::TrailingSlash { .. }),
        "got {err}"
    );
}

#[test]
fn double_slash_is_rejected() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    let err = b
        .add_event_marker(marker("power//on", 0))
        .err()
        .expect("should fail");
    assert!(
        matches!(err, EventMarkerError::EmptySegment { .. }),
        "got {err}"
    );
}

#[test]
fn too_long_is_rejected() {
    let path = "a".repeat(257);
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    let err = b
        .add_event_marker(marker(&path, 0))
        .err()
        .expect("should fail");
    assert!(
        matches!(err, EventMarkerError::TooLong { len: 257, .. }),
        "got {err}"
    );
}

#[test]
fn space_in_path_is_rejected() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    let err = b
        .add_event_marker(marker("power/turn on", 0))
        .err()
        .expect("should fail");
    assert!(
        matches!(err, EventMarkerError::InvalidChars { .. }),
        "got {err}"
    );
}

#[test]
fn dot_in_path_is_rejected() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    let err = b
        .add_event_marker(marker("power/v1.2", 0))
        .err()
        .expect("should fail");
    assert!(
        matches!(err, EventMarkerError::InvalidChars { .. }),
        "got {err}"
    );
}

// Error messages include the offending path

#[test]
fn error_messages_include_path() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));

    let cases: &[(&str, &str)] = &[
        ("", "\"\""),
        ("/leading", "\"/leading\""),
        ("trailing/", "\"trailing/\""),
        ("dbl//slash", "\"dbl//slash\""),
        ("a b", "\"a b\""),
    ];
    for (path, expected_fragment) in cases {
        let err = b
            .add_event_marker(marker(path, 0))
            .err()
            .expect("should fail");
        let msg = err.to_string();
        assert!(
            msg.contains(expected_fragment),
            "error for {path:?} should contain {expected_fragment}, got: {msg}"
        );
    }
}

// Builder — counts and round-trip

#[test]
fn markers_are_stored_in_nav_file() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 10.0, 20.0));
    b.add_nav_fix(fix(100, 12.0, 22.0));
    b.add_event_marker(marker("power/on", 0)).unwrap();
    b.add_event_marker(marker("power/off", 100)).unwrap();

    let nav_file = b.finish().unwrap();
    assert_eq!(nav_file.event_markers().len(), 2);
    assert_eq!(nav_file.event_markers()[0].variant_path, "power/on");
    assert_eq!(nav_file.event_markers()[1].variant_path, "power/off");
}

#[test]
fn annotation_is_preserved() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    b.add_event_marker(
        EventMarker::builder()
            .variant_path("status/alert".to_owned())
            .sys_time(t(0))
            .annotation("motor overtemp".to_owned())
            .build(),
    )
    .unwrap();

    let nav_file = b.finish().unwrap();
    assert_eq!(
        nav_file.event_markers()[0].annotation.as_deref(),
        Some("motor overtemp")
    );
}

// Position interpolation

#[test]
fn position_interpolated_at_midpoint() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 10.0, 20.0));
    b.add_nav_fix(fix(100, 12.0, 24.0));
    b.add_event_marker(marker("sensor/mid", 50)).unwrap();

    let nav_file = b.finish().unwrap();
    let em = &nav_file.event_markers()[0];
    assert!(
        (em.lat.get::<degree>() - 11.0).abs() < 1e-9,
        "lat should be 11.0, got {}",
        em.lat.get::<degree>()
    );
    assert!(
        (em.lon.get::<degree>() - 22.0).abs() < 1e-9,
        "lon should be 22.0, got {}",
        em.lon.get::<degree>()
    );
}

#[test]
fn position_clamped_to_first_fix_when_before_track() {
    let mut b = NavFileBuilder::new().with_continue_on_error(true);
    b.add_nav_fix(fix(10, 55.0, 12.0));
    b.add_event_marker(marker("boot", 0)).unwrap();

    let nav_file = b.finish().unwrap();
    let em = &nav_file.event_markers()[0];
    assert!(
        (em.lat.get::<degree>() - 55.0).abs() < 1e-9,
        "pre-track marker should be clamped to first fix"
    );
}

// Styles

#[test]
fn event_marker_styles_are_stored() {
    let mut b = NavFileBuilder::new();
    b.add_nav_fix(fix(0, 55.0, 12.0));
    b.add_event_marker(marker("power/on", 0)).unwrap();
    b.add_event_marker_style(
        EventMarkerStyle::builder()
            .variant_path("power/on".to_owned())
            .icon_name("lightning".to_owned())
            .color_hex("#FFAA00".to_owned())
            .build(),
    );

    let nav_file = b.finish().unwrap();
    assert_eq!(nav_file.event_marker_styles().len(), 1);
    assert_eq!(nav_file.event_marker_styles()[0].icon_name, "lightning");
    assert_eq!(nav_file.event_marker_styles()[0].color_hex, "#FFAA00");
}
