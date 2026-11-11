use chrono::{DateTime, Duration};
use gt_types::AssociationWindow;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::Value;

use super::{self as tests, LogAttachment, LogAttachmentDecodeErrorReason, StoredYearResolution};

#[rstest]
#[case::zero(Duration::zero(), "0")]
#[case::nanosecond(Duration::nanoseconds(1), "1")]
#[case::fractional_seconds(Duration::nanoseconds(1_234_567_890), "1234567890")]
#[case::maximum(Duration::days(365), "31536000000000000")]
fn explicit_windows_round_trip_as_integer_nanoseconds(
    #[case] duration: Duration,
    #[case] expected: &str,
) {
    let window = AssociationWindow::try_from(duration).expect("valid window");
    assert_eq!(serde_json::to_string(&window).expect("encode"), expected);
    let mut expected_attachment = tests::attachment();
    expected_attachment.settings.association_window = Some(window);
    let json = expected_attachment.to_attribute_json().expect("encode");
    assert_eq!(
        LogAttachment::from_attribute_json(&json).expect("decode"),
        expected_attachment
    );
    assert_eq!(
        serde_json::from_str::<LogAttachment>(&json).expect("Serde decode"),
        expected_attachment
    );
}

#[rstest]
#[case::null("null")]
#[case::negative("-1")]
#[case::above_maximum("31536000000000001")]
#[case::integer_overflow("18446744073709551616")]
#[case::float("1.0")]
#[case::string(r#""1""#)]
#[case::object("{}")]
#[case::array("[]")]
#[case::boolean("true")]
fn malformed_windows_return_a_window_error(#[case] window: &str) {
    let json = format!(
        r#"{{"format_version":7,"name":"broken.log","content_hash":"0","filters":[],"association_window_ns":{window}}}"#
    );
    let error = LogAttachment::from_attribute_json(&json).expect_err("malformed window");
    assert_eq!(error.name.as_deref(), Some("broken.log"));
    assert!(matches!(
        error.reason,
        LogAttachmentDecodeErrorReason::MalformedAssociationWindow(_)
    ));
    serde_json::from_str::<LogAttachment>(&json).expect_err("Serde window check");
    serde_json::from_str::<AssociationWindow>(window).expect_err("bounded Serde type");
}

#[test]
fn duplicate_window_fields_return_a_metadata_error() {
    let json = r#"{"format_version":7,"name":"broken.log","content_hash":"0","filters":[],"association_window_ns":0,"association_window_ns":1}"#;
    let error = LogAttachment::from_attribute_json(json).expect_err("duplicate window");
    assert_eq!(error.name.as_deref(), Some("broken.log"));
    assert!(matches!(
        error.reason,
        LogAttachmentDecodeErrorReason::MalformedMetadata(_)
    ));
    serde_json::from_str::<LogAttachment>(json).expect_err("Serde duplicate check");
}

#[rstest]
fn missing_windows_remain_unresolved_during_metadata_upgrades(
    #[values(1, 2, 3, 4, 5, 6)] version: u32,
) {
    let mut expected = tests::attachment();
    expected.settings.year_reference = Some(DateTime::UNIX_EPOCH);
    expected.settings.year_resolution = StoredYearResolution::IgnoreSummaryBounds;
    let mut wire = serde_json::to_value(&expected).expect("metadata");
    wire["format_version"] = serde_json::json!(version);
    let restored = LogAttachment::from_attribute_json(&wire.to_string()).expect("legacy metadata");
    assert_eq!(restored.settings.association_window, None);
    assert_eq!(restored, expected);
    let rewritten = restored.to_attribute_json().expect("upgrade");
    let upgraded: Value = serde_json::from_str(&rewritten).expect("JSON");
    assert_eq!(upgraded.get("format_version"), Some(&serde_json::json!(7)));
    assert_eq!(upgraded.get("association_window_ns"), None);
    assert_eq!(
        LogAttachment::from_attribute_json(&rewritten).expect("restored upgrade"),
        expected
    );
}

#[rstest]
fn legacy_versions_with_window_semantics_return_an_error(#[values(1, 2, 3, 4, 5, 6)] version: u32) {
    let json = format!(
        r#"{{"format_version":{version},"name":"invalid.log","content_hash":"0","filters":[],"association_window_ns":0}}"#
    );
    assert!(matches!(
        LogAttachment::from_attribute_json(&json).expect_err("window version gate").reason,
        LogAttachmentDecodeErrorReason::AssociationWindowBeforeVersionSeven { version: found } if found == version
    ));
    serde_json::from_str::<LogAttachment>(&json).expect_err("Serde version gate");
}

proptest! {
    #[test]
    fn arbitrary_integer_windows_preserve_valid_values_or_return_an_error(nanoseconds in any::<i64>()) {
        let json = format!(r#"{{"format_version":7,"name":"property.log","content_hash":"0","filters":[],"association_window_ns":{nanoseconds}}}"#);
        match LogAttachment::from_attribute_json(&json) {
            Ok(restored) => {
                let window = restored.settings.association_window.expect("explicit window");
                prop_assert_eq!(Duration::from(window), Duration::nanoseconds(nanoseconds));
                prop_assert!((Duration::zero()..=Duration::days(365)).contains(&Duration::from(window)));
            }
            Err(error) => {
                prop_assert!(!(0..=31_536_000_000_000_000).contains(&nanoseconds));
                prop_assert!(matches!(error.reason, LogAttachmentDecodeErrorReason::MalformedAssociationWindow(_)));
            }
        }
    }
}
