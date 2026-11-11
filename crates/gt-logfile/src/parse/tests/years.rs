use std::fmt::Write as _;

use chrono::{DateTime, Datelike as _, Duration, TimeZone as _, Utc};
use proptest::prelude::*;
use rstest::rstest;

use crate::format::LogFormat;
use crate::parse;
use crate::parse::tests::fixtures::{self, REBOOT};
use crate::test_util::{self, EXPORTED_SUMMARY};
use crate::{
    EligibleLogTime, LogChronology, OrderAnomaly, UnresolvedYear, YearConflict, YearResolution,
};

#[rstest]
#[case::seconds(
    "Dec 31 23:59:59 before\nfiller\n--- Device reboot ---\nJan  1 00:00:01 after\n",
    0
)]
#[case::microseconds(
    "Dec 31 23:59:59.500000 before\nfiller\n--- Device reboot ---\nJan  1 00:00:01.500000 after\n",
    500_000
)]
fn a_year_transition_across_a_reboot_precedes_interpolation(
    #[case] text: &str,
    #[case] micros: i64,
) {
    let parsed = parse::parse_log(text.into(), test_util::utc(2026, 10, 1, 0, 0, 0))
        .expect("parse the year transition");
    let before = test_util::utc(2025, 12, 31, 23, 59, 59) + Duration::microseconds(micros);
    let after = test_util::utc(2026, 1, 1, 0, 0, 1) + Duration::microseconds(micros);
    let resolved = fixtures::resolved(&parsed);
    assert_eq!(resolved.timestamps(), [before, before, after]);
    assert_eq!(parsed.year_resolution(), YearResolution::Infer);
    assert_eq!(
        (0..parsed.entries().len())
            .map(|index| resolved
                .eligible_time(index)
                .map(EligibleLogTime::timestamp))
            .collect::<Vec<_>>(),
        [Some(before), Some(before), Some(after)]
    );
    assert_eq!(resolved.order_anomalies(), []);
    assert_eq!(parsed.boot_sessions().len(), 2);
    assert_eq!(
        resolved.session_bounds(0).map(|bounds| bounds.last),
        Some(before)
    );
    assert_eq!(
        resolved.session_bounds(1).map(|bounds| bounds.first),
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
    assert_eq!(fixtures::resolved(&chunked).order_anomalies(), []);
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
#[case::minimum(DateTime::<Utc>::MIN_UTC, Err(UnresolvedYear {
    line_number: 1,
    written_timestamp: "Dec 31 23:59:59".to_owned(),
    conflict: YearConflict::FollowYearReference {
        candidate: test_util::utc(DateTime::<Utc>::MIN_UTC.year(), 12, 31, 23, 59, 59),
        year_reference: DateTime::<Utc>::MIN_UTC,
    },
}))]
#[case::maximum(DateTime::<Utc>::MAX_UTC, Ok(vec![test_util::utc(DateTime::<Utc>::MAX_UTC.year(), 12, 31, 23, 59, 59)]))]
fn reference_limits_return_valid_timestamps_or_leave_the_years_unresolved(
    #[case] reference: DateTime<Utc>,
    #[case] expected: Result<Vec<DateTime<Utc>>, UnresolvedYear>,
) {
    let parsed = parse::parse_log("Dec 31 23:59:59 only".into(), reference).expect("parses");
    assert_eq!(fixtures::resolved_timestamps(&parsed), expected);
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
    let timestamps = fixtures::timestamps(&fixtures::parsed_log(text));
    assert_eq!(
        timestamps.first().map(|timestamp| timestamp.year()),
        Some(years[0])
    );
    assert_eq!(
        timestamps.last().map(|timestamp| timestamp.year()),
        Some(years[1])
    );
}

#[rstest]
#[case::outside_bounds(
    "May 20 18:00:00 entry\n",
    "Logs begin at: Thu 01-Jan-2026 00:00:00 UTC\nLogs end at: Sun 01-Feb-2026 00:00:00 UTC\n",
    Err(UnresolvedYear {
        line_number: 1,
        written_timestamp: "May 20 18:00:00".to_owned(),
        conflict: YearConflict::FollowLogsEndAt {
            candidate: test_util::utc(2026, 5, 20, 18, 0, 0),
            logs_end_at: test_util::utc(2026, 2, 1, 0, 0, 0),
        },
    }),
    vec![]
)]
#[case::reversed_bounds(
    "Jan  1 00:00:00 entry\n",
    "Logs begin at: Sun 01-Feb-2026 00:00:00 UTC\nLogs end at: Thu 01-Jan-2026 00:00:00 UTC\n",
    Err(UnresolvedYear {
        line_number: 1,
        written_timestamp: "Jan  1 00:00:00".to_owned(),
        conflict: YearConflict::PrecedeLogsBeginAt {
            candidate: test_util::utc(2026, 1, 1, 0, 0, 0),
            logs_begin_at: test_util::utc(2026, 2, 1, 0, 0, 0),
        },
    }),
    vec![]
)]
#[case::clock_regression_within_bounds(
    "Dec 31 23:59:59 first\nJan  1 00:00:00 second\n",
    "Logs begin at: Thu 01-Jan-2026 00:00:00 UTC\nLogs end at: Thu 31-Dec-2026 23:59:59 UTC\n",
    Ok(vec![test_util::utc(2026, 12, 31, 23, 59, 59), test_util::utc(2026, 1, 1, 0, 0, 0)]),
    vec![OrderAnomaly { line_number: 2, timestamp_step: test_util::utc(2026, 1, 1, 0, 0, 0) - test_util::utc(2026, 12, 31, 23, 59, 59) }]
)]
#[case::middle_entry_after_logs_end_at(
    "May 29 18:48:25 first\nSep 03 21:11:30 middle\nMay 29 18:50:11 last\n",
    "Logs begin at: Fri 29-May-2026 18:48:25 UTC\nLogs end at: Fri 29-May-2026 18:50:11 UTC\n",
    Ok(vec![test_util::utc(2026, 5, 29, 18, 48, 25), test_util::utc(2026, 9, 3, 21, 11, 30), test_util::utc(2026, 5, 29, 18, 50, 11)]),
    vec![OrderAnomaly { line_number: 3, timestamp_step: test_util::utc(2026, 5, 29, 18, 50, 11) - test_util::utc(2026, 9, 3, 21, 11, 30) }]
)]
#[case::fractional_end(
    "Jan  1 00:00:00.500000 entry\n",
    "Logs begin at: Thu 01-Jan-2026 00:00:00 UTC\nLogs end at: Thu 01-Jan-2026 00:00:00 UTC\n",
    Ok(vec![test_util::utc(2026, 1, 1, 0, 0, 0) + Duration::microseconds(500_000)]),
    vec![]
)]
#[case::leap_day_clock_regression_within_bounds(
    "Dec 31 23:59:59 first\nFeb 29 00:00:00 second\n",
    "Logs begin at: Mon 01-Jan-2024 00:00:00 UTC\nLogs end at: Tue 31-Dec-2024 23:59:59 UTC\n",
    Ok(vec![test_util::utc(2024, 12, 31, 23, 59, 59), test_util::utc(2024, 2, 29, 0, 0, 0)]),
    vec![OrderAnomaly { line_number: 2, timestamp_step: test_util::utc(2024, 2, 29, 0, 0, 0) - test_util::utc(2024, 12, 31, 23, 59, 59) }]
)]
fn exporter_bounds_constrain_year_candidates(
    #[case] entries: &str,
    #[case] bounds: &str,
    #[case] timestamps: Result<Vec<DateTime<Utc>>, UnresolvedYear>,
    #[case] anomalies: Vec<OrderAnomaly>,
) {
    let text = format!("{entries}----------- Journal summary -----------\n{bounds}");
    let parsed = parse::parse_log(text.as_str().into(), fixtures::now()).expect("parses");
    assert_eq!(fixtures::resolved_timestamps(&parsed), timestamps);
    assert_eq!(parsed.chronology().order_anomalies(), anomalies);
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
        prop_assert_eq!(fixtures::resolved(&parsed).order_anomalies(), []);
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
    assert_eq!(fixtures::resolved(&parsed).order_anomalies(), []);
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
    assert_eq!(
        fixtures::timestamps(&fixtures::parsed_log(text))
            .iter()
            .map(|timestamp| timestamp.year())
            .collect::<Vec<_>>(),
        years
    );
}

#[test]
fn a_leap_day_outside_exporter_bounds_leaves_the_years_unresolved() {
    let text = "Feb 29 23:59:59 first\nMar  1 00:00:00 second\n----------- Journal summary -----------\nLogs begin at: Wed 01-Jan-2025 00:00:00 UTC\nLogs end at: Thu 31-Dec-2026 23:59:59 UTC\n";
    assert_eq!(
        fixtures::resolved_timestamps(&fixtures::parsed_log(text)),
        Err(UnresolvedYear {
            line_number: 1,
            written_timestamp: "Feb 29 23:59:59".to_owned(),
            conflict: YearConflict::LackDate { year: 2025 },
        })
    );
}

#[test]
fn a_backward_clock_step_before_logs_begin_at_keeps_the_year_of_its_neighbours() {
    let parsed = fixtures::parsed_log(BACKWARD_CLOCK_STEP_BEFORE_LOGS_BEGIN_AT);
    let backward_step = parsed
        .entries()
        .iter()
        .position(|entry| entry.line_number == 14)
        .and_then(|entry_index| fixtures::resolved(&parsed).timestamp(entry_index));
    assert_eq!(backward_step, Some(test_util::utc(2026, 5, 20, 11, 0, 0)));
    assert_eq!(
        fixtures::resolved(&parsed).order_anomalies(),
        [OrderAnomaly {
            line_number: 14,
            timestamp_step: test_util::utc(2026, 5, 20, 11, 0, 0)
                - test_util::utc(2026, 5, 20, 12, 0, 9),
        }]
    );
}

#[rstest]
#[case::beside_a_clock_adjustment(
    "Nov 09 10:00:00 host svc: before reboot\n--- Device reboot ---\n\
     Jan 01 00:00:05 host svc: after reset\n\
     Jan 01 00:00:40 host systemd-timesyncd[1]: Initial clock synchronization to Mon 2026-11-09 12:00:00 UTC.\n\
     Nov 09 12:00:00 host svc: time synced\n",
    vec![
        test_util::utc(2026, 11, 9, 10, 0, 0),
        test_util::utc(2026, 1, 1, 0, 0, 5),
        test_util::utc(2026, 1, 1, 0, 0, 40),
        test_util::utc(2026, 11, 9, 12, 0, 0),
    ]
)]
#[case::without_a_clock_adjustment(
    "Nov 09 10:00:00 host svc: before reboot\n--- Device reboot ---\n\
     Jan 01 00:00:05 host svc: after reset\n\
     Nov 09 12:00:00 host svc: time synced\n",
    vec![
        test_util::utc(2025, 11, 9, 10, 0, 0),
        test_util::utc(2026, 1, 1, 0, 0, 5),
        test_util::utc(2026, 11, 9, 12, 0, 0),
    ]
)]
#[case::without_a_reboot_separator(
    "Nov 09 10:00:00 host svc: before reset\n\
     Jan 01 00:00:05 host svc: after reset\n\
     Jan 01 00:00:40 host systemd-timesyncd[1]: Initial clock synchronization to Mon 2026-11-09 12:00:00 UTC.\n\
     Nov 09 12:00:00 host svc: time synced\n",
    vec![
        test_util::utc(2025, 11, 9, 10, 0, 0),
        test_util::utc(2026, 1, 1, 0, 0, 5),
        test_util::utc(2026, 1, 1, 0, 0, 40),
        test_util::utc(2026, 11, 9, 12, 0, 0),
    ]
)]
fn a_clock_reset_after_a_reboot_leaves_its_year_chain_only_beside_a_clock_adjustment(
    #[case] text: &str,
    #[case] expected: Vec<DateTime<Utc>>,
) {
    let parsed = parse::parse_log(text.into(), test_util::utc(2026, 11, 10, 0, 0, 0))
        .expect("parse the clock reset");
    assert_eq!(fixtures::timestamps(&parsed), expected);
}

#[test]
fn clock_resets_after_several_reboots_resolve_between_the_summary_block_bounds() {
    let parsed = fixtures::parsed_log(gt_test_utils::MULTI_REBOOT_CLOCK_RESET_LOG);
    assert_eq!(
        fixtures::timestamps(&parsed),
        [
            test_util::utc(2026, 11, 9, 8, 0, 0),
            test_util::utc(2026, 1, 1, 0, 0, 5),
            test_util::utc(2026, 1, 1, 0, 0, 40),
            test_util::utc(2026, 11, 9, 9, 0, 0),
            test_util::utc(2026, 1, 1, 0, 0, 5),
            test_util::utc(2026, 1, 1, 0, 0, 41),
            test_util::utc(2026, 11, 9, 10, 0, 0),
        ]
    );
    assert_eq!(
        parsed
            .boot_sessions()
            .iter()
            .map(|session| session.first_synchronised_entry)
            .collect::<Vec<_>>(),
        [0, 3, 6]
    );
    assert_eq!(fixtures::resolved(&parsed).order_anomalies(), []);
}

#[rstest]
#[case::whole_log(0, 14)]
#[case::first_ten_lines_removed(10, 4)]
fn a_last_entry_before_logs_begin_at_leaves_the_years_unresolved_at_its_line(
    #[case] removed_line_count: usize,
    #[case] line_number: u32,
) {
    let text: String = BACKWARD_CLOCK_STEP_BEFORE_LOGS_BEGIN_AT
        .split_inclusive('\n')
        .skip(removed_line_count)
        .filter(|line| !line.contains("clock restored"))
        .collect();
    assert_eq!(
        fixtures::resolved_timestamps(&fixtures::parsed_log(&text)),
        Err(UnresolvedYear {
            line_number,
            written_timestamp: "May 20 11:00:00".to_owned(),
            conflict: YearConflict::PrecedeLogsBeginAt {
                candidate: test_util::utc(2026, 5, 20, 11, 0, 0),
                logs_begin_at: test_util::utc(2026, 5, 20, 12, 0, 0),
            },
        })
    );
}

#[test]
fn logs_begin_at_with_the_reset_clock_of_the_first_entry_does_not_bound_the_synchronised_entries() {
    let text = format!(
        "{REBOOT}Jan 01 00:00:05 host svc: after reset\n\
         Jan 01 00:00:40 host systemd-timesyncd[1]: Initial clock synchronization to Mon 2026-11-09 12:00:00 UTC.\n\
         Nov 09 12:00:00 host svc: time synced\n\
         ----------- Journal summary -----------\n\
         Logs begin at: Thu 01-Jan-1970 00:00:05 UTC\n\
         Logs end at: Mon 09-Nov-2026 12:00:00 UTC\n"
    );
    assert_eq!(
        fixtures::timestamps(&fixtures::parsed_log(&text)),
        [
            test_util::utc(2026, 1, 1, 0, 0, 5),
            test_util::utc(2026, 1, 1, 0, 0, 40),
            test_util::utc(2026, 11, 9, 12, 0, 0),
        ]
    );
}

#[test]
fn a_boot_session_across_two_new_years_rolls_over_twice_with_a_clock_adjustment_beside_the_second()
{
    let text = format!(
        "{REBOOT}Mar 01 00:00:00 host svc: start\n\
         Aug 01 00:00:00 host svc: summer\n\
         Dec 31 23:59:59 host svc: before the first midnight\n\
         Jan  1 00:00:01 host svc: after the first midnight\n\
         Jun 01 00:00:00 host svc: summer\n\
         Oct 01 00:00:00 host svc: autumn\n\
         Dec 31 23:59:58 host svc: before the second midnight\n\
         Dec 31 23:59:59 host chronyd[610]: System clock wrong by 0.2 seconds, adjustment started\n\
         Jan  1 00:00:01 host svc: after the second midnight\n"
    );
    assert_eq!(
        fixtures::timestamps(&fixtures::parsed_log(&text)),
        [
            test_util::utc(2024, 3, 1, 0, 0, 0),
            test_util::utc(2024, 8, 1, 0, 0, 0),
            test_util::utc(2024, 12, 31, 23, 59, 59),
            test_util::utc(2025, 1, 1, 0, 0, 1),
            test_util::utc(2025, 6, 1, 0, 0, 0),
            test_util::utc(2025, 10, 1, 0, 0, 0),
            test_util::utc(2025, 12, 31, 23, 59, 58),
            test_util::utc(2025, 12, 31, 23, 59, 59),
            test_util::utc(2026, 1, 1, 0, 0, 1),
        ]
    );
}

#[rstest]
#[case::ordinary_entries("host svc: midnight")]
#[case::beside_a_clock_adjustment(
    "host chronyd[610]: System clock wrong by 0.2 seconds, adjustment started"
)]
fn a_new_year_inside_one_boot_session_rolls_over(#[case] message_before_midnight: &str) {
    let text = format!(
        "{REBOOT}Dec 31 23:59:30 host svc: before midnight\n\
         Dec 31 23:59:59 {message_before_midnight}\n\
         Jan  1 00:00:01 host svc: after midnight\n"
    );
    assert_eq!(
        fixtures::timestamps(&fixtures::parsed_log(&text)),
        [
            test_util::utc(2025, 12, 31, 23, 59, 30),
            test_util::utc(2025, 12, 31, 23, 59, 59),
            test_util::utc(2026, 1, 1, 0, 0, 1),
        ]
    );
}

#[test]
fn unresolved_years_report_the_line_of_the_attempt_that_resolved_the_most_entries() {
    let parsed = parse::parse_log(
        gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG.into(),
        test_util::utc(2026, 11, 9, 0, 0, 0),
    )
    .expect("an unresolved chronology parses");
    assert_eq!(
        fixtures::resolved_timestamps(&parsed),
        Err(UnresolvedYear {
            line_number: 1,
            written_timestamp: "Feb 29 12:00:00".to_owned(),
            conflict: YearConflict::LackDate { year: 2023 },
        })
    );
    let LogChronology::Unresolved(unresolved) = parsed.chronology() else {
        panic!("the years of the fixture are unresolved");
    };
    assert_eq!(
        (0..parsed.entries().len())
            .map(|entry_index| unresolved
                .written_timestamp(entry_index)
                .map(|written| (written.month(), written.day())))
            .collect::<Vec<_>>(),
        [Some((2, 29)), Some((12, 31)), Some((1, 1)), Some((2, 29))]
    );
}

#[rstest]
#[case::a_chosen_year_places_the_leap_days_in_leap_years(
    gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG.to_owned(),
    YearResolution::PlaceLastEntryInYear(2024),
    Ok(vec![
        test_util::utc(2020, 2, 29, 12, 0, 0),
        test_util::utc(2023, 12, 31, 23, 59, 59),
        test_util::utc(2024, 1, 1, 0, 0, 0),
        test_util::utc(2024, 2, 29, 12, 0, 0),
    ])
)]
#[case::a_chosen_year_without_the_date_of_the_last_entry(
    gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG.to_owned(),
    YearResolution::PlaceLastEntryInYear(2025),
    Err(UnresolvedYear {
        line_number: 4,
        written_timestamp: "Feb 29 12:00:00".to_owned(),
        conflict: YearConflict::LackDate { year: 2025 },
    })
)]
#[case::ignoring_the_summary_bounds_keeps_a_backward_clock_step_in_its_year(
    BACKWARD_CLOCK_STEP_BEFORE_LOGS_BEGIN_AT
        .split_inclusive('\n')
        .filter(|line| !line.contains("clock restored"))
        .collect::<String>(),
    YearResolution::IgnoreSummaryBounds,
    Ok([0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
        .map(|second| test_util::utc(2026, 5, 20, 12, 0, second))
        .into_iter()
        .chain([test_util::utc(2026, 5, 20, 11, 0, 0)])
        .collect())
)]
fn a_year_resolution_resolves_the_same_on_parse_and_on_an_unresolved_log(
    #[case] text: String,
    #[case] year_resolution: YearResolution,
    #[case] expected: Result<Vec<DateTime<Utc>>, UnresolvedYear>,
) {
    let year_reference = test_util::utc(2026, 11, 9, 0, 0, 0);
    let inferred = parse::parse_log(text.as_str().into(), year_reference).expect("parses");
    let parsed = parse::parse_log_with_year_resolution(
        text.as_str().into(),
        year_reference,
        year_resolution,
    )
    .expect("parses");

    assert!(!inferred.chronology().is_resolved());
    assert_eq!(fixtures::resolved_timestamps(&parsed), expected);
    assert_eq!(parsed.year_resolution(), year_resolution);
    assert_eq!(inferred.resolve_years_as(year_resolution), Some(parsed));
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
        let timestamps = fixtures::timestamps(&parsed);
        let first = *timestamps.first().expect("first entry");
        let last = *timestamps.last().expect("last entry");
        prop_assert_eq!(last - first, Duration::seconds(1));
        prop_assert_eq!(first.timestamp_subsec_micros(), fraction);
        prop_assert!(last <= reference);
        prop_assert!((reference_year - first.year()) <= 8);
        prop_assert_eq!(parse::parse_log(text.as_str().into(), parsed.year_reference()), Ok(parsed));
    }
}

const BACKWARD_CLOCK_STEP_BEFORE_LOGS_BEGIN_AT: &str = "\
# export header
# host: example

May 20 12:00:00 host service: ordinary entry
May 20 12:00:01 host service: ordinary entry
May 20 12:00:02 host service: ordinary entry
May 20 12:00:03 host service: ordinary entry
May 20 12:00:04 host service: ordinary entry
May 20 12:00:05 host service: ordinary entry
May 20 12:00:06 host service: ordinary entry
May 20 12:00:07 host service: ordinary entry
May 20 12:00:08 host service: ordinary entry
May 20 12:00:09 host service: ordinary entry
May 20 11:00:00 host service: clock moved backwards
May 20 12:00:10 host service: clock restored
----------- Journal summary -----------
Logs begin at: Wed 20-May-2026 12:00:00 UTC
Logs end at: Wed 20-May-2026 12:00:10 UTC
";
