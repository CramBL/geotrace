use gt_logfile::YearResolution;
use rstest::rstest;

use super::*;
use crate::{LoadedLog, test_util};

#[rstest]
#[case::utc_seconds("2026-01-01 10:00:05 UTC", "2026-01-01T10:00:05Z".parse().unwrap())]
#[case::utc_minutes("2026-01-01 10:00 UTC", "2026-01-01T10:00:00Z".parse().unwrap())]
#[case::positive_offset("2026-01-01 12:00:05 +02:00", "2026-01-01T10:00:05Z".parse().unwrap())]
#[case::negative_offset("2026-01-01T05:00:05-05:00", "2026-01-01T10:00:05Z".parse().unwrap())]
#[case::fraction("2026-01-01T10:00:05.123456789Z", "2026-01-01T10:00:05.123456789Z".parse().unwrap())]
fn time_requests_normalize_explicit_zones_and_preserve_subseconds(
    #[case] text: &str,
    #[case] expected: DateTime<Utc>,
) {
    let LogJumpRequest::GoToTime(time) = LogJumpRequest::parse_time(text).unwrap() else {
        panic!("time request");
    };
    assert_eq!(time, expected);
}

#[test]
fn the_time_parser_requires_supported_fractional_precision() {
    assert_eq!(
        LogJumpRequest::parse_time("2026-01-01T10:00:05.1234567891Z").unwrap_err(),
        InvalidLogJump::RequireSupportedPrecision
    );
}

#[rstest]
#[case::zero("0")]
#[case::negative("-1")]
#[case::overflow("4294967296")]
#[case::fraction("1.5")]
fn invalid_line_input_remains_separate_from_source_targets(#[case] text: &str) {
    assert_eq!(
        LogJumpRequest::parse_line(text).unwrap_err(),
        InvalidLogJump::RequirePhysicalLine
    );
}

#[rstest]
#[case::missing_zone("2026-01-01 10:00:05")]
#[case::missing_date("10:00 UTC")]
#[case::invalid_date("2026-02-30 10:00 UTC")]
#[case::named_zone("2026-01-01 10:00 Europe/Copenhagen")]
fn incomplete_or_invalid_time_input_has_no_request(#[case] text: &str) {
    assert_eq!(
        LogJumpRequest::parse_time(text).unwrap_err(),
        InvalidLogJump::RequireZonedDateTime
    );
}

#[rstest]
#[case::backward_step("2026-01-01 10:00:02 UTC", 3)]
#[case::equal_source_order("2026-01-01 10:00:05 UTC", 1)]
#[case::subsecond_boundary("2026-01-01 10:00:05.000000001 UTC", 2)]
fn time_jumps_select_the_earliest_eligible_time_in_source_order(
    #[case] text: &str,
    #[case] expected_line: u32,
) {
    let mut logs = LoadedLogs::default();
    let id = logs
        .push(LoadedLog::new(
            None,
            test_util::parsed_log_of_text(BACKWARD_SOURCE),
            test_util::association_window(),
        ))
        .id();
    let target = LogJumpRequest::parse_time(text)
        .unwrap()
        .resolve(&logs, id)
        .unwrap();
    assert_eq!(target.log(), id);
    assert_eq!(target.line_number(), expected_line);
}

#[test]
fn line_jumps_resolve_first_and_last_source_lines_and_report_unloaded_logs() {
    let mut logs = LoadedLogs::default();
    let id = logs.push(LoadedLog::new(None, test_util::parsed_log_of_text("2026-01-01 10:00:00 svc: first\n \t\n--- Device reboot ---\n2026-01-01 10:00:01 svc: last"), test_util::association_window())).id();
    for line in [1, 4] {
        assert_eq!(
            LogJumpRequest::parse_line(&line.to_string())
                .unwrap()
                .resolve(&logs, id)
                .unwrap()
                .line_number(),
            line
        );
    }
    assert_eq!(
        LogJumpRequest::parse_line("5")
            .unwrap()
            .resolve(&logs, id)
            .unwrap_err(),
        UnresolvedLogJump::ReportNoLine
    );
    logs.remove_by_id(id);
    assert_eq!(
        LogJumpRequest::parse_line("1")
            .unwrap()
            .resolve(&logs, id)
            .unwrap_err(),
        UnresolvedLogJump::ReportUnavailableLog
    );
}

#[test]
fn reset_clock_readings_are_excluded_from_time_jumps() {
    let mut logs = LoadedLogs::default();
    let id = logs
        .push(LoadedLog::new(
            None,
            test_util::parsed_log_of_text(gt_test_utils::MULTI_REBOOT_CLOCK_RESET_LOG),
            test_util::association_window(),
        ))
        .id();
    let target = LogJumpRequest::parse_time("2026-01-01 00:00 UTC")
        .unwrap()
        .resolve(&logs, id)
        .unwrap();
    assert_eq!(target.line_number(), 1);
}

#[test]
fn time_jumps_use_current_chronology_and_line_jumps_remain_available_unresolved() {
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_with_unresolved_years()).id();
    let request = LogJumpRequest::parse_time("2026-05-20 11:30 UTC").unwrap();
    assert_eq!(
        request.resolve(&logs, id).unwrap_err(),
        UnresolvedLogJump::ReportUnavailableDates
    );
    assert_eq!(
        LogJumpRequest::parse_line("2")
            .unwrap()
            .resolve(&logs, id)
            .unwrap()
            .line_number(),
        2
    );
    let files = test_util::loaded(Vec::new());
    assert!(
        logs.get_mut_by_id(id)
            .unwrap()
            .resolve_years_as(YearResolution::PlaceLastEntryInYear(2026), &files.view())
    );
    assert_eq!(request.resolve(&logs, id).unwrap().line_number(), 2);
    assert!(
        logs.get_mut_by_id(id)
            .unwrap()
            .resolve_years_as(YearResolution::PlaceLastEntryInYear(2025), &files.view())
    );
    assert_eq!(
        request.resolve(&logs, id).unwrap_err(),
        UnresolvedLogJump::ReportNoTime
    );
}

const BACKWARD_SOURCE: &str = "2026-01-01 10:00:05 svc: first\n2026-01-01 10:00:10 svc: later\n2026-01-01 10:00:03 svc: backward\n2026-01-01 10:00:05 svc: tied\n";
