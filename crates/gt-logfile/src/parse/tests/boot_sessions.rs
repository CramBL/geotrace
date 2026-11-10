use chrono::Duration;
use gt_types::TimeRange;
use rstest::rstest;

use crate::parse;
use crate::parse::tests::fixtures::{self, REBOOT};
use crate::session::OrderAnomaly;
use crate::test_util;

/// The clock steps back over the reboot, so the log's span ends at an
/// entry the file wrote before its last one.
#[test]
fn the_time_range_spans_the_earliest_and_latest_entry_across_a_reboot() {
    let parsed = fixtures::parsed_log(&format!(
        "2026-05-23 10:00:00 first\n2026-05-23 12:00:00 last\n{REBOOT}2026-05-23 11:00:00 after the reboot\n"
    ));
    assert_eq!(
        fixtures::resolved(&parsed).time_range(),
        Some(TimeRange::new(
            test_util::utc(2026, 5, 23, 10, 0, 0),
            test_util::utc(2026, 5, 23, 12, 0, 0)
        ))
    );
}

#[rstest]
#[case::no_separator("2026-01-01 00:00:00 a\n2026-01-01 00:00:01 b\n", &[2])]
#[case::one_separator("2026-01-01 00:00:00 a\n--- Device reboot ---\n2026-01-01 00:00:01 b\n", &[1, 1])]
#[case::three_separators(
    "2026-01-01 00:00:00 a\n--- Device reboot ---\n2026-01-01 00:00:01 b\n\
     --- Device reboot ---\n2026-01-01 00:00:02 c\n--- Device reboot ---\n\
     2026-01-01 00:00:03 d\n",
    &[1, 1, 1, 1]
)]
#[case::separator_before_the_first_entry("--- Device reboot ---\n2026-01-01 00:00:00 a\n", &[1])]
#[case::two_separators_in_a_row(
    "2026-01-01 00:00:00 a\n--- Device reboot ---\n--- Device reboot ---\n2026-01-01 00:00:01 b\n",
    &[1, 1]
)]
fn reboot_separators_cut_the_log_into_boot_sessions(
    #[case] text: &str,
    #[case] expected_entry_counts: &[usize],
) {
    let parsed = fixtures::parsed_log(text);
    assert_eq!(
        parsed
            .boot_sessions()
            .iter()
            .map(|session| (session.boot_number, session.entry_count()))
            .collect::<Vec<_>>(),
        expected_entry_counts
            .iter()
            .enumerate()
            .map(|(index, count)| (index as u32 + 1, *count))
            .collect::<Vec<_>>()
    );
}

/// A session's uptime spans its own anchors. A clock adjustment that lands
/// the session's last anchor before its first leaves it without one.
#[test]
fn a_boot_session_spans_its_own_anchors() {
    let parsed = fixtures::parsed_log(&format!(
        "2026-01-01 00:00:00 a\n2026-01-01 02:30:00 b\n{REBOOT}2026-01-01 00:05:00 c\n\
         2026-01-01 00:04:00 systemd-timedated: Time has been changed\n"
    ));
    let resolved = fixtures::resolved(&parsed);
    let uptimes: Vec<Option<Duration>> = (0..parsed.boot_sessions().len())
        .map(|session_index| resolved.session_uptime(session_index))
        .collect();
    assert_eq!(uptimes, [Some(Duration::minutes(150)), None]);
}

#[test]
fn the_uptime_of_a_boot_session_starts_at_its_first_entry_after_a_clock_jump() {
    let text = format!(
        "Nov 09 10:00:00 host svc: before reboot\n{REBOOT}\
         Jan 01 00:00:05 host svc: after reset\n\
         Jan 01 00:00:40 host systemd-timesyncd[1]: Initial clock synchronization to Mon 2026-11-09 12:00:00 UTC.\n\
         Nov 09 12:00:00 host svc: time synced\n\
         Nov 09 12:30:00 host svc: later\n"
    );
    let parsed = parse::parse_log(text.as_str().into(), test_util::utc(2026, 11, 10, 0, 0, 0))
        .expect("parse the clock reset");
    let resolved = fixtures::resolved(&parsed);
    assert_eq!(resolved.session_uptime(1), Some(Duration::minutes(30)));
    assert_eq!(
        resolved.session_bounds(1).map(|bounds| bounds.first),
        Some(test_util::utc(2026, 11, 9, 12, 0, 0))
    );
}

#[rstest]
#[case::unexplained("2026-01-01 00:00:10 a\n2026-01-01 00:00:05 b\n", 1)]
#[case::explained_by_the_stepping_line_itself(
    "2026-01-01 00:00:10 a\n2026-01-01 00:00:05 systemd-timedated: Time has been changed\n",
    0
)]
#[case::explained_by_a_later_line(
    "2026-01-01 00:00:10 a\n2026-01-01 00:00:05 b\n2026-01-01 00:00:05 systemd-journald: Time jumped backwards, rotating.\n",
    0
)]
#[case::explanation_too_far_away(
    "2026-01-01 00:00:10 a\n2026-01-01 00:00:05 b\n2026-01-01 00:00:06 c\n\
     2026-01-01 00:00:07 d\n2026-01-01 00:00:08 e\n\
     2026-01-01 00:00:09 systemd-journald: Time jumped backwards, rotating.\n",
    1
)]
#[case::across_a_reboot(
    "2026-01-01 00:00:10 a\n--- Device reboot ---\n2026-01-01 00:00:05 b\n",
    0
)]
fn a_backwards_step_is_an_anomaly_only_when_nothing_nearby_reports_a_clock_change(
    #[case] text: &str,
    #[case] expected_anomalies: usize,
) {
    assert_eq!(
        fixtures::resolved(&fixtures::parsed_log(text))
            .order_anomalies()
            .len(),
        expected_anomalies
    );
}

#[test]
fn an_order_anomaly_names_the_line_it_steps_back_on_and_how_far() {
    let parsed = fixtures::parsed_log("2026-01-01 03:12:00 a\nfiller\n2026-01-01 00:00:00 b\n");
    assert_eq!(
        fixtures::resolved(&parsed).order_anomalies(),
        [OrderAnomaly {
            line_number: 3,
            timestamp_step: -Duration::minutes(192),
        }]
    );
}
