//! What the loader makes of a custom marker whose `markers/icon` code is
//! outside the set this build draws, read back through the real file path.

#![expect(
    clippy::expect_used,
    reason = "the fixture helper beside the tests is not covered by clippy's in-test relaxations"
)]

use geotrace_sdk::{
    Angle, Annotation, AnnotationIcon, DateTime, Duration, NavFileBuilder, NavFix, NavFixTime, Utc,
};
use gt_types::MarkerIcon;

/// A `markers/icon` code outside the set this build has, as a newer build
/// could write it.
const UNRECOGNIZED_ICON_CODE: u8 = 200;

const MARKER_LABEL: &str = "from a newer build";

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp is within range")
}

fn recording_with_an_unrecognized_marker_icon() -> Vec<u8> {
    let mut recorder = NavFileBuilder::new().open();
    for second in 0..2i64 {
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(
                    base_time() + Duration::seconds(second),
                ))
                .lat(Angle::degrees(55.0))
                .lon(Angle::degrees(12.0 + second as f64 * 0.001))
                .heading(Angle::degrees(90.0))
                .build(),
        );
    }
    recorder.add_annotation(
        Annotation::builder()
            .time(base_time())
            .label(MARKER_LABEL)
            .icon(AnnotationIcon::Unrecognized(UNRECOGNIZED_ICON_CODE))
            .build()
            .expect("the marker label fits its field"),
    );

    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("the fixes build a nav file")
        .write(&mut bytes)
        .expect("the nav file writes");
    bytes
}

#[test]
fn a_marker_with_an_icon_code_this_build_does_not_have_is_drawn_as_a_pin_and_reported() {
    let file = gt_loader::load_bytes(
        &recording_with_an_unrecognized_marker_icon(),
        "unrecognized_marker_icon.gtd".to_owned(),
    )
    .expect("the recording loads");

    let markers: Vec<_> = file
        .tracks
        .iter()
        .flat_map(|track| track.custom_markers.iter())
        .collect();
    let [marker] = markers.as_slice() else {
        panic!("expected one custom marker, got {}", markers.len());
    };
    assert_eq!(marker.icon, MarkerIcon::Pin);
    assert_eq!(marker.label, MARKER_LABEL);

    let [warning] = file.load_warnings.as_slice() else {
        panic!(
            "expected one load warning, got {:?}",
            file.load_warnings
                .iter()
                .map(|warning| &warning.issue)
                .collect::<Vec<_>>()
        );
    };
    assert_eq!(warning.count, 1);
    assert_eq!(warning.issue, "custom marker icon(s) replaced with the pin");
    assert_eq!(
        warning.description,
        "\"from a newer build\": 200. Those markers are drawn as a pin: the file holds an icon \
         code this version of GeoTrace does not have."
    );
}
