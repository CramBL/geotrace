use rstest::rstest;

use crate::parse::tests::fixtures;
use crate::parse::{self, ERROR_LINE_EXCERPT_CHARS, LogParseError};
use crate::test_util;
use crate::{BoundComparison, YearConflict};

#[test]
fn a_log_without_any_recognised_timestamp_names_its_first_line() {
    let error = parse::parse_log("nothing here\nnor here\n".into(), fixtures::now())
        .expect_err("fails to parse");
    assert_eq!(
        error.to_string(),
        "Not a recognised log: no line has a timestamp in a known format \
         (first line: \"nothing here\")"
    );
}

#[test]
fn a_very_long_first_line_is_quoted_up_to_an_excerpt() {
    let text = "x".repeat(ERROR_LINE_EXCERPT_CHARS.get() + 50);
    let error =
        parse::parse_log(text.as_str().into(), fixtures::now()).expect_err("fails to parse");
    assert_eq!(
        error,
        LogParseError::NoRecognisedFormat {
            first_line: format!(
                "{}{}",
                "x".repeat(ERROR_LINE_EXCERPT_CHARS.get()),
                gt_fmt::ELLIPSIS
            ),
        }
    );
}

#[rstest]
#[case::no_bytes("")]
#[case::only_blank_lines("\n\n   \n")]
fn a_log_without_any_line_is_empty(#[case] text: &str) {
    assert_eq!(
        parse::parse_log(text.into(), fixtures::now()),
        Err(LogParseError::Empty)
    );
}

#[rstest]
#[case::follow_logs_end_at(
    YearConflict::FollowLogsEndAt {
        candidate: test_util::utc(2027, 5, 20, 11, 0, 0),
        logs_end_at: test_util::utc(2026, 5, 20, 12, 0, 10),
    },
    "Cannot resolve the year of log line 14 (\"May 20 11:00:00\"): 2027-05-20 11:00:00 UTC is \
     after the summary block's \"Logs end at\" 2026-05-20 12:00:10 UTC"
)]
#[case::follow_year_reference(
    YearConflict::FollowYearReference {
        candidate: test_util::utc(2026, 5, 20, 11, 0, 0),
        year_reference: test_util::utc(2026, 5, 20, 9, 30, 0),
    },
    "Cannot resolve the year of log line 14 (\"May 20 11:00:00\"): 2026-05-20 11:00:00 UTC is \
     more than 1h after the reference date 2026-05-20 09:30:00 UTC"
)]
#[case::lack_date(
    YearConflict::LackDate { year: 2023 },
    "Cannot resolve the year of log line 14 (\"May 20 11:00:00\"): the date does not exist in 2023"
)]
#[case::precede_logs_begin_at(
    YearConflict::PrecedeLogsBeginAt {
        candidate: test_util::utc(2026, 5, 20, 11, 0, 0),
        logs_begin_at: test_util::utc(2026, 5, 20, 12, 0, 0),
    },
    "Cannot resolve the year of log line 14 (\"May 20 11:00:00\"): 2026-05-20 11:00:00 UTC is \
     before the summary block's \"Logs begin at\" 2026-05-20 12:00:00 UTC"
)]
fn a_year_resolution_error_states_the_line_its_timestamp_and_the_conflict(
    #[case] conflict: YearConflict,
    #[case] expected: &str,
) {
    let error = LogParseError::UnresolvedYear {
        line_number: 14,
        written_timestamp: "May 20 11:00:00".to_owned(),
        conflict,
    };
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::follow_logs_end_at(
    YearConflict::FollowLogsEndAt {
        candidate: test_util::utc(2027, 5, 20, 11, 0, 0),
        logs_end_at: test_util::utc(2026, 5, 20, 12, 0, 10),
    },
    "After the summary block's \"Logs end at\"",
    Some(BoundComparison {
        candidate: test_util::utc(2027, 5, 20, 11, 0, 0),
        bound_name: "Logs end at",
        bound: test_util::utc(2026, 5, 20, 12, 0, 10),
    })
)]
#[case::follow_year_reference(
    YearConflict::FollowYearReference {
        candidate: test_util::utc(2026, 5, 20, 11, 0, 0),
        year_reference: test_util::utc(2026, 5, 20, 9, 30, 0),
    },
    "More than 1h after the reference date",
    Some(BoundComparison {
        candidate: test_util::utc(2026, 5, 20, 11, 0, 0),
        bound_name: "Reference date",
        bound: test_util::utc(2026, 5, 20, 9, 30, 0),
    })
)]
#[case::lack_date(YearConflict::LackDate { year: 2023 }, "The date does not exist in 2023", None)]
#[case::precede_logs_begin_at(
    YearConflict::PrecedeLogsBeginAt {
        candidate: test_util::utc(2026, 5, 20, 11, 0, 0),
        logs_begin_at: test_util::utc(2026, 5, 20, 12, 0, 0),
    },
    "Before the summary block's \"Logs begin at\"",
    Some(BoundComparison {
        candidate: test_util::utc(2026, 5, 20, 11, 0, 0),
        bound_name: "Logs begin at",
        bound: test_util::utc(2026, 5, 20, 12, 0, 0),
    })
)]
fn a_year_conflict_states_its_reason_and_the_bound_its_candidate_conflicts_with(
    #[case] conflict: YearConflict,
    #[case] reason: &str,
    #[case] bound_comparison: Option<BoundComparison>,
) {
    assert_eq!(
        (conflict.reason().as_str(), conflict.bound_comparison()),
        (reason, bound_comparison)
    );
}

#[test]
fn the_headline_of_a_year_resolution_error_ends_at_its_line_number() {
    let error = LogParseError::UnresolvedYear {
        line_number: 14,
        written_timestamp: "May 20 11:00:00".to_owned(),
        conflict: YearConflict::LackDate { year: 2023 },
    };
    assert_eq!(error.headline(), "Cannot resolve the year of log line 14");
}

#[test]
fn a_very_long_unresolved_timestamp_is_quoted_up_to_an_excerpt() {
    let fraction = "0".repeat(ERROR_LINE_EXCERPT_CHARS.get());
    let text = format!(
        "Jan  1 00:00:00.{fraction} entry\n----------- Journal summary -----------\n\
         Logs begin at: Sun 01-Feb-2026 00:00:00 UTC\nLogs end at: Sat 28-Feb-2026 00:00:00 UTC\n"
    );
    let written = format!("Jan  1 00:00:00.{fraction}");
    let excerpt: String = written
        .chars()
        .take(ERROR_LINE_EXCERPT_CHARS.get())
        .collect();
    assert_eq!(
        parse::parse_log(text.as_str().into(), fixtures::now()),
        Err(LogParseError::UnresolvedYear {
            line_number: 1,
            written_timestamp: format!("{excerpt}{}", gt_fmt::ELLIPSIS),
            conflict: YearConflict::PrecedeLogsBeginAt {
                candidate: test_util::utc(2026, 1, 1, 0, 0, 0),
                logs_begin_at: test_util::utc(2026, 2, 1, 0, 0, 0),
            },
        })
    );
}
