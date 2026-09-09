use chrono::{DateTime, Duration, Utc};
use rstest::rstest;

use crate::format::LogFormat;
use crate::parse::tests::fixtures;
use crate::parse::{self, FORMAT_DETECTION_LINE_LIMIT, LogParseError};
use crate::test_util;

/// A log of one format keeps the lines written in another: they are entries
/// timestamped from their neighbours, not a second format.
#[test]
fn only_the_detected_format_is_parsed() {
    let parsed = fixtures::parsed_log("2026-01-01 00:00:00 iso\nMay 29 18:48:24 syslog\n");
    assert_eq!(parsed.format(), LogFormat::Iso8601Space);
    assert_eq!(
        fixtures::messages(&parsed),
        ["iso", "May 29 18:48:24 syslog"]
    );
    assert_eq!(parsed.interpolated_entry_count(), 1);
}

/// A banner longer than the detector's sample hides the format that follows.
#[test]
fn the_format_is_detected_within_the_head_of_the_log() {
    let mut within_head = "banner\n".repeat(FORMAT_DETECTION_LINE_LIMIT - 1);
    within_head.push_str("2026-01-01 00:00:00 body\n");
    let parsed = fixtures::parsed_log(&within_head);
    assert_eq!(parsed.anchored_entry_count(), 1);
    assert_eq!(
        parsed.interpolated_entry_count(),
        FORMAT_DETECTION_LINE_LIMIT - 1
    );

    let past_head = "banner\n".repeat(FORMAT_DETECTION_LINE_LIMIT) + "2026-01-01 00:00:00 body";
    assert_eq!(
        parse::parse_log(past_head.as_str().into(), fixtures::now()),
        Err(LogParseError::NoRecognisedFormat {
            first_line: "banner".to_owned(),
        })
    );
}

#[rstest]
#[case::syslog_short("May 20 18:48:24 msg", test_util::utc(2026, 5, 20, 18, 48, 24), 0)]
#[case::syslog_short_micro(
    "May 20 18:48:24.500000 msg",
    test_util::utc(2026, 5, 20, 18, 48, 24),
    500_000
)]
#[case::iso_8601_space("2026-05-20 18:48:24 msg", test_util::utc(2026, 5, 20, 18, 48, 24), 0)]
#[case::iso_8601_t("2026-05-20T18:48:24Z msg", test_util::utc(2026, 5, 20, 18, 48, 24), 0)]
fn every_format_yields_its_timestamp_and_message(
    #[case] line: &str,
    #[case] expected_second: DateTime<Utc>,
    #[case] expected_micros: i64,
) {
    let parsed = fixtures::parsed_log(line);
    let entry = parsed.entries().first().copied().expect("one entry");
    assert_eq!(
        entry.timestamp,
        expected_second + Duration::microseconds(expected_micros)
    );
    assert_eq!(parsed.message(&entry), "msg");
}

#[test]
fn a_year_less_format_resolves_against_now() {
    let parsed = parse::parse_log(
        "Dec 31 23:59:59 rollover\n".into(),
        test_util::utc(2026, 1, 1, 0, 0, 0),
    )
    .expect("parses");
    assert_eq!(
        parsed.entries().first().map(|entry| entry.timestamp),
        Some(test_util::utc(2025, 12, 31, 23, 59, 59))
    );
}
