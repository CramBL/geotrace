use std::sync::Arc;

use chrono::{Datelike as _, Duration};
use rstest::rstest;

use crate::parse::tests::fixtures;
use crate::{ClockEligibility, LogChronology, YearConflict, YearResolution, parse, test_util};

#[rstest]
#[case::automatically_resolved("Feb 29 12:00:00 first\nMar 01 12:00:00 last\n", true)]
#[case::automatically_unresolved(gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG, false)]
fn a_successful_year_choice_can_be_corrected_and_reset(
    #[case] text: &str,
    #[case] initially_resolved: bool,
) {
    let original = parse::parse_log(text.into(), test_util::utc(2026, 11, 9, 0, 0, 0))
        .expect("the source parses");
    assert_eq!(original.chronology().is_resolved(), initially_resolved);

    let mut edited = original.clone();
    for year in [2024, 2028] {
        edited = edited
            .resolve_years_as(YearResolution::PlaceLastEntryInYear(year))
            .expect("retained anchors support another choice");
        assert_eq!(
            edited.year_resolution(),
            YearResolution::PlaceLastEntryInYear(year)
        );
        assert_eq!(
            fixtures::resolved(&edited)
                .timestamps()
                .last()
                .map(|timestamp| timestamp.year()),
            Some(year)
        );
        assert!(Arc::ptr_eq(original.text(), edited.text()));
        assert_eq!(edited.entries(), original.entries());
        assert_eq!(edited.structural_lines(), original.structural_lines());
        assert_eq!(edited.boot_sessions(), original.boot_sessions());
    }

    assert_eq!(
        edited.resolve_years_as(YearResolution::Infer),
        Some(original)
    );
}

#[rstest]
#[case::invalid_leap_year(2025)]
#[case::below_calendar_range(i32::MIN)]
#[case::above_calendar_range(i32::MAX)]
fn an_invalid_year_after_success_preserves_evidence_for_correction(#[case] year: i32) {
    let original = fixtures::parsed_log("Feb 29 12:00:00 only\n");
    let invalid = original
        .resolve_years_as(YearResolution::PlaceLastEntryInYear(year))
        .expect("a conflict remains a parsed log");
    let LogChronology::Unresolved(unresolved) = invalid.chronology() else {
        panic!("the selected year has no matching date");
    };
    assert_eq!(
        unresolved.unresolved_year().conflict,
        YearConflict::LackDate { year }
    );
    assert_eq!(
        unresolved
            .written_timestamp(0)
            .map(|timestamp| (timestamp.month(), timestamp.day())),
        Some((2, 29))
    );

    let corrected = invalid
        .resolve_years_as(YearResolution::PlaceLastEntryInYear(2028))
        .expect("the written timestamp is retained");
    assert_eq!(
        fixtures::resolved(&corrected).timestamp(0),
        Some(test_util::utc(2028, 2, 29, 12, 0, 0))
    );
    assert_eq!(
        corrected.resolve_years_as(YearResolution::Infer),
        Some(original)
    );
}

#[test]
fn ignoring_exporter_bounds_can_be_reversed_after_success() {
    let text = "May 20 12:00:00 host svc: first\nMay 20 12:00:02 host svc: last\n\
        ----------- Journal summary -----------\n\
        Logs begin at: Tue 20-May-2025 12:00:00 UTC\n\
        Logs end at: Tue 20-May-2025 12:00:02 UTC\n";
    let original = fixtures::parsed_log(text);
    assert_eq!(
        fixtures::resolved(&original).timestamp(0),
        Some(test_util::utc(2025, 5, 20, 12, 0, 0))
    );

    let ignored = original
        .resolve_years_as(YearResolution::IgnoreSummaryBounds)
        .expect("retained source supports ignoring exporter bounds");
    assert_eq!(
        fixtures::resolved(&ignored).timestamp(0),
        Some(test_util::utc(2026, 5, 20, 12, 0, 0))
    );
    assert_eq!(ignored.summary_block(), original.summary_block());
    assert_eq!(
        ignored.resolve_years_as(YearResolution::Infer),
        Some(original)
    );
}

#[test]
fn policy_changes_preserve_explicit_dates_and_interpolation() {
    let original = fixtures::parsed_log(
        "2024-02-29T12:00:00.123456789+02:00 first\ncontinuation\n2024-02-29T12:00:02.123456789+02:00 last\n",
    );
    let expected = test_util::utc(2024, 2, 29, 10, 0, 0) + Duration::nanoseconds(123_456_789);
    assert_eq!(
        fixtures::resolved(&original).timestamps(),
        [
            expected,
            expected + Duration::seconds(1),
            expected + Duration::seconds(2)
        ]
    );

    let mut edited = original.clone();
    for policy in [
        YearResolution::PlaceLastEntryInYear(i32::MAX),
        YearResolution::IgnoreSummaryBounds,
        YearResolution::Infer,
    ] {
        edited = edited
            .resolve_years_as(policy)
            .expect("explicit dates remain valid");
        assert_eq!(edited.chronology(), original.chronology());
    }
    assert_eq!(edited, original);
}

#[test]
fn policy_changes_preserve_reset_clock_eligibility_and_session_interpolation() {
    let text = "Nov 09 10:00:00 host svc: before reboot\n\
        --- Device reboot ---\n\
        Jan 01 00:00:05 host svc: after reset\n\
        continuation\n\
        Jan 01 00:00:40 host systemd-timesyncd[1]: Initial clock synchronization to Mon 2026-11-09 12:00:00 UTC.\n\
        Nov 09 12:00:00 host svc: time synced\n\
        continuation\n\
        Nov 09 12:30:00 host svc: later\n";
    let original = parse::parse_log(text.into(), test_util::utc(2026, 11, 10, 0, 0, 0))
        .expect("the clock reset parses");
    let edited = original
        .resolve_years_as(YearResolution::PlaceLastEntryInYear(2028))
        .expect("retained timestamps support a new year");
    let resolved = fixtures::resolved(&edited);
    assert_eq!(edited.boot_sessions(), original.boot_sessions());
    assert_eq!(
        edited
            .entries()
            .iter()
            .map(|entry| entry.line_number)
            .collect::<Vec<_>>(),
        [1, 3, 4, 5, 6, 7, 8]
    );
    for index in 1..4 {
        assert_eq!(
            resolved.clock_eligibility(index),
            Some(ClockEligibility::BeforeSynchronization)
        );
        assert_eq!(resolved.eligible_time(index), None);
    }
    assert_eq!(
        resolved.timestamp(4),
        Some(test_util::utc(2028, 11, 9, 12, 0, 0))
    );
    assert_eq!(
        resolved.timestamp(5),
        Some(test_util::utc(2028, 11, 9, 12, 15, 0))
    );
    assert_eq!(resolved.session_uptime(1), Some(Duration::minutes(30)));
    assert_eq!(
        edited.resolve_years_as(YearResolution::Infer),
        Some(original)
    );
}
