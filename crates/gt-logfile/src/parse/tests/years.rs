use std::fmt::Write as _;

use chrono::{DateTime, Datelike as _, Duration, TimeZone as _, Utc};
use proptest::prelude::*;
use rstest::rstest;

use crate::OrderAnomaly;
use crate::format::LogFormat;
use crate::parse::tests::fixtures;
use crate::parse::{self, LogParseError};
use crate::test_util::{self, EXPORTED_SUMMARY};

#[rstest]
#[case::seconds(
    "Dec 31 23:59:59 before\nfiller\n--- Device reboot ---\nJan  1 00:00:01 after\n",
    0
)]
#[case::microseconds(
    "Dec 31 23:59:59.500000 before\nfiller\n--- Device reboot ---\nJan  1 00:00:01.500000 after\n",
    500_000
)]
fn a_year_transition_precedes_boot_segmentation_and_interpolation(
    #[case] text: &str,
    #[case] micros: i64,
) {
    let parsed = parse::parse_log(text.into(), test_util::utc(2026, 10, 1, 0, 0, 0))
        .expect("parse the year transition");
    let before = test_util::utc(2025, 12, 31, 23, 59, 59) + Duration::microseconds(micros);
    let after = test_util::utc(2026, 1, 1, 0, 0, 1) + Duration::microseconds(micros);
    assert_eq!(fixtures::timestamps(&parsed), [before, before, after]);
    assert_eq!(parsed.order_anomalies(), []);
    assert_eq!(parsed.boot_sessions().len(), 2);
    assert_eq!(
        parsed
            .boot_sessions()
            .first()
            .and_then(|boot| boot.anchored)
            .map(|bounds| bounds.last),
        Some(before)
    );
    assert_eq!(
        parsed
            .boot_sessions()
            .last()
            .and_then(|boot| boot.anchored)
            .map(|bounds| bounds.first),
        Some(after)
    );
}

#[rstest]
#[case::both_bounds(
    "Logs begin at: Wed 31-Dec-2025 23:59:59 UTC\nLogs end at: Thu 01-Jan-2026 00:00:01 UTC\n"
)]
#[case::begin_only("Logs begin at: Wed 31-Dec-2025 23:59:59 UTC\n")]
#[case::end_only("Logs end at: Thu 01-Jan-2026 00:00:01 UTC\n")]
fn exporter_bounds_determine_years_under_later_reference_dates(#[case] bounds: &str) {
    let text = format!(
        "Dec 31 23:59:59 before\nJan  1 00:00:01 after\n----------- Journal summary -----------\n{bounds}"
    );
    for reference_year in [2026, 2032] {
        let parsed = parse::parse_log(
            text.as_str().into(),
            test_util::utc(reference_year, 10, 1, 0, 0, 0),
        )
        .expect("parse the export");
        assert_eq!(
            fixtures::timestamps(&parsed),
            [
                test_util::utc(2025, 12, 31, 23, 59, 59),
                test_util::utc(2026, 1, 1, 0, 0, 1)
            ]
        );
    }
}

#[test]
fn a_real_summary_constrains_a_log_longer_than_twelve_months_across_chunks() {
    let text = format!(
        "May 29 18:48:25 first\nDec 31 23:59:59 winter\nJan  1 00:00:00 next year\nJun 26 07:59:50 last\n{EXPORTED_SUMMARY}\n"
    );
    let one_chunk = parse::parse_log_in_chunks_of(
        text.as_str().into(),
        fixtures::now(),
        fixtures::chunk_bytes(text.len() + 1),
    )
    .expect("parse the export");
    let chunked = parse::parse_log_in_chunks_of(
        text.as_str().into(),
        fixtures::now(),
        fixtures::chunk_bytes(1),
    )
    .expect("parse the export in chunks");
    assert_eq!(chunked, one_chunk);
    assert_eq!(
        fixtures::timestamps(&chunked),
        [
            test_util::utc(2025, 5, 29, 18, 48, 25),
            test_util::utc(2025, 12, 31, 23, 59, 59),
            test_util::utc(2026, 1, 1, 0, 0, 0),
            test_util::utc(2026, 6, 26, 7, 59, 50)
        ]
    );
    assert_eq!(chunked.order_anomalies(), []);
}

#[test]
fn the_exact_exporter_end_dates_the_last_entry_without_an_observed_year_transition() {
    let text = format!("May 29 18:48:25 first\nJun 26 07:59:50 last\n{EXPORTED_SUMMARY}\n");
    let parsed = fixtures::parsed_log(&text);
    assert_eq!(
        fixtures::timestamps(&parsed),
        [
            test_util::utc(2025, 5, 29, 18, 48, 25),
            test_util::utc(2026, 6, 26, 7, 59, 50)
        ]
    );
}

#[rstest]
#[case::both_bounds("May 29 18:48:25 first\nJan  1 00:00:00 middle\nJun 26 07:59:50 last\n", EXPORTED_SUMMARY, vec![test_util::utc(2025, 5, 29, 18, 48, 25), test_util::utc(2026, 1, 1, 0, 0, 0), test_util::utc(2026, 6, 26, 7, 59, 50)])]
#[case::begin_only("May 29 18:48:25 first\nJan  1 00:00:00 middle\nJun 26 07:59:50 last\n", "----------- Journal summary -----------\nLogs begin at: Thu 29-May-2025 18:48:25 UTC\n", vec![test_util::utc(2025, 5, 29, 18, 48, 25), test_util::utc(2026, 1, 1, 0, 0, 0), test_util::utc(2026, 6, 26, 7, 59, 50)])]
#[case::end_only("May 29 18:48:25 first\nJan  1 00:00:00 last\n", "----------- Journal summary -----------\nLogs end at: Thu 01-Jan-2026 00:00:00 UTC\n", vec![test_util::utc(2025, 5, 29, 18, 48, 25), test_util::utc(2026, 1, 1, 0, 0, 0)])]
fn exporter_bounds_resolve_sparse_year_transitions_across_chunks(
    #[case] entries: &str,
    #[case] summary: &str,
    #[case] expected: Vec<DateTime<Utc>>,
) {
    let text = format!("{entries}{summary}");
    let one_chunk = parse::parse_log_in_chunks_of(
        text.as_str().into(),
        fixtures::now(),
        fixtures::chunk_bytes(text.len() + 1),
    );
    let chunked = parse::parse_log_in_chunks_of(
        text.as_str().into(),
        fixtures::now(),
        fixtures::chunk_bytes(1),
    );
    assert_eq!(chunked, one_chunk);
    assert_eq!(
        fixtures::timestamps(&chunked.expect("resolve sparse timestamps")),
        expected
    );
}

#[rstest]
#[case::minimum(DateTime::<Utc>::MIN_UTC, Err(LogParseError::UnresolvedYear { line_number: 1 }))]
#[case::maximum(DateTime::<Utc>::MAX_UTC, Ok(Some(test_util::utc(DateTime::<Utc>::MAX_UTC.year(), 12, 31, 23, 59, 59))))]
fn reference_limits_return_valid_timestamps_or_a_year_resolution_error(
    #[case] reference: DateTime<Utc>,
    #[case] expected: Result<Option<DateTime<Utc>>, LogParseError>,
) {
    let parsed = parse::parse_log("Dec 31 23:59:59 only".into(), reference);
    assert_eq!(
        parsed.map(|parsed| parsed.first_anchored_timestamp()),
        expected
    );
}

#[rstest]
#[case::same_day("May 20 12:00:00 first\nMay 20 11:00:00 second\n", [2026, 2026])]
#[case::previous_month("May 20 12:00:00 first\nApr 20 11:00:00 second\n", [2026, 2026])]
#[case::rollover_threshold("Jul  3 00:00:00 first\nJan  1 00:00:00 second\n", [2026, 2026])]
#[case::past_rollover_threshold("Jul  4 00:00:00 first\nJan  1 00:00:00 second\n", [2025, 2026])]
#[case::leap_day("Feb 29 23:59:59 first\nMar  1 00:00:00 second\n", [2024, 2024])]
#[case::leap_day_at_end("Feb 29 23:59:59 only\n", [2024, 2024])]
fn clock_regressions_and_leap_days_resolve_to_valid_years(
    #[case] text: &str,
    #[case] years: [i32; 2],
) {
    let parsed = fixtures::parsed_log(text);
    assert_eq!(
        parsed.entries().first().map(|entry| entry.timestamp.year()),
        Some(years[0])
    );
    assert_eq!(
        parsed.entries().last().map(|entry| entry.timestamp.year()),
        Some(years[1])
    );
}

#[rstest]
#[case::outside_bounds(
    "May 20 18:00:00 entry\n",
    "Logs begin at: Thu 01-Jan-2026 00:00:00 UTC\nLogs end at: Sun 01-Feb-2026 00:00:00 UTC\n",
    None,
    vec![]
)]
#[case::reversed_bounds(
    "Jan  1 00:00:00 entry\n",
    "Logs begin at: Sun 01-Feb-2026 00:00:00 UTC\nLogs end at: Thu 01-Jan-2026 00:00:00 UTC\n",
    None,
    vec![]
)]
#[case::clock_regression_within_bounds(
    "Dec 31 23:59:59 first\nJan  1 00:00:00 second\n",
    "Logs begin at: Thu 01-Jan-2026 00:00:00 UTC\nLogs end at: Thu 31-Dec-2026 23:59:59 UTC\n",
    Some(vec![test_util::utc(2026, 12, 31, 23, 59, 59), test_util::utc(2026, 1, 1, 0, 0, 0)]),
    vec![OrderAnomaly { line_number: 2, timestamp_step: test_util::utc(2026, 1, 1, 0, 0, 0) - test_util::utc(2026, 12, 31, 23, 59, 59) }]
)]
#[case::fractional_end(
    "Jan  1 00:00:00.500000 entry\n",
    "Logs begin at: Thu 01-Jan-2026 00:00:00 UTC\nLogs end at: Thu 01-Jan-2026 00:00:00 UTC\n",
    Some(vec![test_util::utc(2026, 1, 1, 0, 0, 0) + Duration::microseconds(500_000)]),
    vec![]
)]
#[case::leap_day_clock_regression_within_bounds(
    "Dec 31 23:59:59 first\nFeb 29 00:00:00 second\n",
    "Logs begin at: Mon 01-Jan-2024 00:00:00 UTC\nLogs end at: Tue 31-Dec-2024 23:59:59 UTC\n",
    Some(vec![test_util::utc(2024, 12, 31, 23, 59, 59), test_util::utc(2024, 2, 29, 0, 0, 0)]),
    vec![OrderAnomaly { line_number: 2, timestamp_step: test_util::utc(2024, 2, 29, 0, 0, 0) - test_util::utc(2024, 12, 31, 23, 59, 59) }]
)]
fn exporter_bounds_constrain_year_candidates(
    #[case] entries: &str,
    #[case] bounds: &str,
    #[case] timestamps: Option<Vec<DateTime<Utc>>>,
    #[case] anomalies: Vec<OrderAnomaly>,
) {
    let text = format!("{entries}----------- Journal summary -----------\n{bounds}");
    let parsed = parse::parse_log(text.as_str().into(), fixtures::now());
    match timestamps {
        Some(timestamps) => {
            let parsed = parsed.expect("parse within bounds");
            assert_eq!(fixtures::timestamps(&parsed), timestamps);
            assert_eq!(parsed.order_anomalies(), anomalies);
        }
        None => assert_eq!(
            parsed,
            Err(LogParseError::UnresolvedYear { line_number: 1 })
        ),
    }
}

proptest! {
    #[test]
    fn monthly_syslog_entries_resolve_across_years_and_chunk_boundaries(
        first_year in 2000i32..2095,
        month_count in 1usize..60,
        microseconds in any::<bool>(),
        chunk_bytes in 1usize..256,
    ) {
        let format = if microseconds { LogFormat::SyslogShortMicro } else { LogFormat::SyslogShort };
        let mut timestamps = Vec::new();
        let mut text = String::new();
        for month in 0..month_count {
            let timestamp = Utc.with_ymd_and_hms(first_year + i32::try_from(month / 12).expect("small year count"), u32::try_from(month % 12 + 1).expect("month"), 1, 0, 0, 0).single().expect("valid date");
            timestamps.push(timestamp);
            writeln!(text, "{} entry", format.written_timestamp(timestamp)).expect("write the entry");
        }
        let begin = timestamps.first().expect("at least one month");
        let end = timestamps.last().expect("at least one month");
        writeln!(text, "----------- Journal summary -----------\nLogs begin at: {}\nLogs end at: {}", begin.format(crate::summary::EXPORTER_TIME_FORMAT), end.format(crate::summary::EXPORTER_TIME_FORMAT)).expect("write the summary");
        let parsed = parse::parse_log_in_chunks_of(text.as_str().into(), fixtures::now(), fixtures::chunk_bytes(chunk_bytes)).expect("parse monthly entries");
        prop_assert_eq!(fixtures::timestamps(&parsed), timestamps);
        prop_assert_eq!(parsed.order_anomalies(), []);
    }
}

#[rstest]
#[case::ordinary_leap_year(2026, 2024)]
#[case::century_exception(2103, 2096)]
#[case::four_hundred_year_boundary(2001, 2000)]
#[case::after_century_exception(2105, 2104)]
fn adjacent_leap_day_lines_resolve_one_second_apart(
    #[case] reference_year: i32,
    #[case] expected_year: i32,
) {
    let reference = test_util::utc(reference_year, 10, 1, 0, 0, 0);
    let text = "Feb 29 23:59:59.123456 first\ncontinuation\n--- Device reboot ---\nMar  1 00:00:00.123456 second\n";
    let parsed = parse::parse_log_in_chunks_of(text.into(), reference, fixtures::chunk_bytes(1))
        .expect("resolve adjacent leap-day entries");
    let before = test_util::utc(expected_year, 2, 29, 23, 59, 59) + Duration::microseconds(123_456);
    let after = before + Duration::seconds(1);
    assert_eq!(fixtures::timestamps(&parsed), [before, before, after]);
    assert_eq!(parsed.order_anomalies(), []);
    assert_eq!(parsed.boot_sessions().len(), 2);
    assert_eq!(
        parse::parse_log(text.into(), parsed.year_reference()),
        Ok(parsed)
    );
}

#[rstest]
#[case::end_only("Logs end at: Sun 01-Mar-2026 00:00:00 UTC\n", [2024, 2024])]
#[case::begin_only("Logs begin at: Sun 01-Jan-2023 00:00:00 UTC\n", [2024, 2024])]
#[case::exact_begin_and_end("Logs begin at: Thu 29-Feb-2024 23:59:59 UTC\nLogs end at: Fri 01-Mar-2024 00:00:00 UTC\n", [2024, 2024])]
#[case::explicit_wider_span("Logs begin at: Thu 29-Feb-2024 23:59:59 UTC\nLogs end at: Sun 01-Mar-2026 00:00:00 UTC\n", [2024, 2026])]
fn exporter_bounds_constrain_the_complete_leap_day_sequence(
    #[case] bounds: &str,
    #[case] years: [i32; 2],
) {
    let text = format!(
        "Feb 29 23:59:59 first\nMar  1 00:00:00 second\n----------- Journal summary -----------\n{bounds}"
    );
    let parsed = fixtures::parsed_log(&text);
    assert_eq!(
        fixtures::timestamps(&parsed),
        [
            test_util::utc(years[0], 2, 29, 23, 59, 59),
            test_util::utc(years[1], 3, 1, 0, 0, 0)
        ]
    );
}

#[rstest]
#[case::rollover_after_leap_day("Feb 29 23:59:59 first\nDec 31 23:59:59 middle\nJan  1 00:00:00 last\n", [2024, 2024, 2025])]
#[case::clock_regression("Mar  1 00:00:00 first\nFeb 29 23:59:59 middle\nMar  1 00:00:01 last\n", [2024, 2024, 2024])]
#[case::rollover_before_leap_day("Dec 31 23:59:59 first\nFeb 29 23:59:59 middle\nMar  1 00:00:00 last\n", [2023, 2024, 2024])]
fn leap_day_resolution_revises_years_on_both_sides_of_a_rollover(
    #[case] text: &str,
    #[case] years: [i32; 3],
) {
    let parsed = fixtures::parsed_log(text);
    assert_eq!(
        parsed
            .entries()
            .iter()
            .map(|entry| entry.timestamp.year())
            .collect::<Vec<_>>(),
        years
    );
}

#[test]
fn leap_day_outside_exporter_bounds_returns_a_year_resolution_error() {
    let text = "Feb 29 23:59:59 first\nMar  1 00:00:00 second\n----------- Journal summary -----------\nLogs begin at: Wed 01-Jan-2025 00:00:00 UTC\nLogs end at: Thu 31-Dec-2026 23:59:59 UTC\n";
    assert_eq!(
        parse::parse_log(text.into(), fixtures::now()),
        Err(LogParseError::UnresolvedYear { line_number: 1 })
    );
}

proptest! {
    #[test]
    fn yearless_leap_day_pairs_preserve_their_elapsed_time(
        reference_year in 1900i32..2200,
        fraction in 0u32..1_000_000,
        chunk_bytes in 1usize..256,
    ) {
        let reference = test_util::utc(reference_year, 10, 1, 0, 0, 0);
        let text = format!("Feb 29 23:59:59.{fraction:06} first\nMar  1 00:00:00.{fraction:06} second\n");
        let parsed = parse::parse_log_in_chunks_of(text.as_str().into(), reference, fixtures::chunk_bytes(chunk_bytes)).expect("resolve leap-day pair");
        let first = parsed.entries().first().expect("first entry").timestamp;
        let last = parsed.entries().last().expect("last entry").timestamp;
        prop_assert_eq!(last - first, Duration::seconds(1));
        prop_assert_eq!(first.timestamp_subsec_micros(), fraction);
        prop_assert!(last <= reference);
        prop_assert!((reference_year - first.year()) <= 8);
        prop_assert_eq!(parse::parse_log(text.as_str().into(), parsed.year_reference()), Ok(parsed));
    }
}
