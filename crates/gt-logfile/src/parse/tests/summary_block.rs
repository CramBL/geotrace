use crate::parse::tests::fixtures;
use crate::parse::{self, LogParseError};
use crate::structure::StructuralLineKind;
use crate::summary::{EntryCountMismatch, ServiceCount};

#[test]
fn the_summary_block_ends_the_entries_and_is_read_as_structure() {
    let parsed = fixtures::parsed_log(
        "2026-01-01 00:00:00 a\n2026-01-01 00:00:01 b\n\
         ----------- Journal summary -----------\nDevice type: nav-devkit-mk2\n\
         Log entries: 2\n--- Service error count ---\nhal-powerd   -> 7 Errors\n\
         2026-01-01 00:00:02 not an entry any more\n",
    );

    assert_eq!(fixtures::messages(&parsed), ["a", "b"]);
    assert_eq!(parsed.structural_lines().len(), 6);
    assert!(
        parsed
            .structural_lines()
            .iter()
            .all(|line| line.kind == StructuralLineKind::SummaryBlock)
    );
    let summary = parsed.summary_block().expect("the block is recognized");
    assert_eq!(summary.device_type.as_deref(), Some("nav-devkit-mk2"));
    assert_eq!(
        summary.service_error_counts,
        [ServiceCount {
            service: "hal-powerd".to_owned(),
            count: 7,
        }]
    );
    assert_eq!(parsed.exporter_entry_count_mismatch(), None);
}

#[test]
fn an_exporter_count_the_parse_disagrees_with_is_reported() {
    let parsed = fixtures::parsed_log(
        "2026-01-01 00:00:00 a\n----------- Journal summary -----------\nLog entries: 9\n",
    );
    assert_eq!(
        parsed.exporter_entry_count_mismatch(),
        Some(EntryCountMismatch {
            stated_by_exporter: 9,
            anchored_by_parse: 1,
        })
    );
}

/// A summary block swallowing the one line that anchored leaves nothing to
/// interpolate from.
#[test]
fn a_log_whose_only_anchor_the_summary_block_swallowed_fails_to_load() {
    let error = parse::parse_log(
        "kernel output\n----------- Journal summary -----------\n2026-01-01 00:00:00 a\n".into(),
        fixtures::now(),
    )
    .expect_err("fails to parse");
    assert_eq!(
        error,
        LogParseError::NoRecognisedFormat {
            first_line: "kernel output".to_owned(),
        }
    );
}
