use crate::parse::TimestampKind;
use crate::parse::tests::fixtures;
use crate::structure::StructuralLineKind;

/// The three kinds a non-empty line can be read as, in one log.
#[test]
fn a_line_is_anchored_structural_or_interpolated() {
    let parsed =
        fixtures::parsed_log("2026-01-01 00:00:00 anchored\n--- Device reboot ---\nno timestamp\n");

    assert_eq!(fixtures::messages(&parsed), ["anchored", "no timestamp"]);
    assert_eq!(
        fixtures::timestamp_kinds(&parsed),
        [TimestampKind::Anchored, TimestampKind::Interpolated]
    );
    assert_eq!(parsed.anchored_entry_count(), 1);
    assert_eq!(parsed.interpolated_entry_count(), 1);
    assert_eq!(
        parsed
            .structural_lines()
            .iter()
            .map(|line| (line.kind, line.line_number))
            .collect::<Vec<_>>(),
        [(StructuralLineKind::RebootSeparator, 2)]
    );
    assert_eq!(parsed.unindexable_line_count(), 0);
}

#[test]
fn blank_lines_are_neither_entries_nor_structure() {
    let parsed = fixtures::parsed_log("\n\n2026-01-01 00:00:00 only\n\n   \n");
    assert_eq!(fixtures::messages(&parsed), ["only"]);
    assert!(parsed.structural_lines().is_empty());
    assert_eq!(parsed.unindexable_line_count(), 0);
}

/// A separator idiom no registry pattern knows is not an error: it loads as
/// an entry like any other untimestamped line.
#[test]
fn an_unknown_separator_is_read_as_an_entry() {
    let parsed = fixtures::parsed_log("2026-01-01 00:00:00 a\n=== Power cycle ===\n");
    assert_eq!(fixtures::messages(&parsed), ["a", "=== Power cycle ==="]);
    assert!(parsed.structural_lines().is_empty());
    assert_eq!(parsed.boot_sessions().len(), 1);
}

/// A file whose lines a later timestamp reordered stays in the order it was
/// written in.
#[test]
fn entries_stay_in_the_order_the_file_wrote_them() {
    let parsed = fixtures::parsed_log(
        "2026-01-01 00:00:05 late\n2026-01-01 00:00:00 alpha\n\
         2026-01-01 00:00:00 beta\n2026-01-01 00:00:02 middle\n",
    );
    assert_eq!(
        fixtures::messages(&parsed),
        ["late", "alpha", "beta", "middle"]
    );
    assert_eq!(
        parsed
            .entries()
            .iter()
            .map(|entry| entry.line_number)
            .collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
}

/// A message is a slice of the log text: indented and CRLF lines shift where
/// it starts and ends within its line.
#[test]
fn an_entry_indexes_its_message_within_the_log_text() {
    let text = "2026-01-01 00:00:00 alpha\r\n   2026-01-01 00:00:01 beta gamma\n";
    let parsed = fixtures::parsed_log(text);
    assert_eq!(parsed.text().as_ref(), text);
    assert_eq!(fixtures::messages(&parsed), ["alpha", "beta gamma"]);

    let entry = parsed.entries().get(1).copied().expect("two entries");
    assert_eq!(entry.message.in_text(text), "beta gamma");
}

#[test]
fn a_line_without_a_message_yields_an_empty_one() {
    let parsed = fixtures::parsed_log("2026-01-01 00:00:00\n");
    let entry = parsed.entries().first().copied().expect("one entry");
    assert_eq!(entry.message.len, 0);
    assert_eq!(parsed.message(&entry), "");
}
