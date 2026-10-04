use gt_log_view::VisibleEntries;
use rstest::rstest;

use super::*;

#[derive(Debug, PartialEq, Eq)]
enum RowIdentity {
    Entry(usize),
    Day(usize),
    Boot(usize, Option<usize>),
    Structural(usize),
}

impl From<LineTableRow> for RowIdentity {
    fn from(row: LineTableRow) -> Self {
        match row {
            LineTableRow::Entry { entry_index, .. } => Self::Entry(entry_index),
            LineTableRow::DayDivider { entry_index } => Self::Day(entry_index),
            LineTableRow::BootDivider {
                session_index,
                structural_index,
            } => Self::Boot(session_index, structural_index),
            LineTableRow::Structural { structural_index } => Self::Structural(structural_index),
        }
    }
}

use RowIdentity::{Boot, Day, Entry, Structural};

const SOURCE: &str = "2026-01-01 23:59:59 keep first\n\
--- Device reboot ---\n\
2026-01-02 00:00:01 hidden middle\n\
2026-01-02 00:01:01 keep second\n\
--- Device reboot ---\n\
2026-01-03 00:00:01 hidden last\n\
----------- Journal summary -----------\n\
trailing summary\n";

#[rstest]
#[case::all("", None,
    vec![Boot(0, None), Entry(0), Day(1), Boot(1, None), Entry(1), Entry(2), Day(3), Boot(2, None), Entry(3)],
    vec![Boot(0, None), Entry(0), Day(1), Boot(1, Some(0)), Entry(1), Entry(2), Day(3), Boot(2, Some(1)), Entry(3), Structural(2), Structural(3)])]
#[case::matching("keep", None,
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, None), Entry(2)],
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, Some(0)), Entry(2), Boot(2, Some(1)), Structural(2), Structural(3)])]
#[case::hidden_first_boot("hidden", None,
    vec![Boot(1, None), Entry(1), Day(3), Boot(2, None), Entry(3)],
    vec![Boot(1, Some(0)), Entry(1), Day(3), Boot(2, Some(1)), Entry(3), Structural(2), Structural(3)])]
#[case::all_hidden("absent", None,
    vec![], vec![Boot(1, Some(0)), Boot(2, Some(1)), Structural(2), Structural(3)])]
#[case::reveal_before_matches("hidden", Some(0),
    vec![Boot(0, None), Entry(0), Day(1), Boot(1, None), Entry(1), Day(3), Boot(2, None), Entry(3)],
    vec![Boot(0, None), Entry(0), Day(1), Boot(1, Some(0)), Entry(1), Day(3), Boot(2, Some(1)), Entry(3), Structural(2), Structural(3)])]
#[case::reveal_between_matches("keep", Some(1),
    vec![Boot(0, None), Entry(0), Day(1), Boot(1, None), Entry(1), Entry(2)],
    vec![Boot(0, None), Entry(0), Day(1), Boot(1, Some(0)), Entry(1), Entry(2), Boot(2, Some(1)), Structural(2), Structural(3)])]
#[case::reveal_after_matches("keep", Some(3),
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, None), Entry(2), Day(3), Boot(2, None), Entry(3)],
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, Some(0)), Entry(2), Day(3), Boot(2, Some(1)), Entry(3), Structural(2), Structural(3)])]
#[case::reveal_in_empty_view("absent", Some(2),
    vec![Boot(1, None), Entry(2)],
    vec![Boot(1, Some(0)), Entry(2), Boot(2, Some(1)), Structural(2), Structural(3)])]
#[case::already_visible_reveal("keep", Some(2),
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, None), Entry(2)],
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, Some(0)), Entry(2), Boot(2, Some(1)), Structural(2), Structural(3)])]
#[case::past_end_reveal("keep", Some(4),
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, None), Entry(2)],
    vec![Boot(0, None), Entry(0), Day(2), Boot(1, Some(0)), Entry(2), Boot(2, Some(1)), Structural(2), Structural(3)])]
fn indexed_rows_preserve_source_order_and_navigation(
    #[case] pattern: &str,
    #[case] reveal: Option<usize>,
    #[case] without_structural: Vec<RowIdentity>,
    #[case] with_structural: Vec<RowIdentity>,
    #[values(false, true)] structural: bool,
) {
    let mut logs = LoadedLogs::default();
    let id = logs.push(source_log(SOURCE)).id();
    let (stack, _) = logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text(pattern);
    stack.wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    let rows = LineTableRows::with_overlay(log, structural, reveal);
    let expected = if structural {
        with_structural
    } else {
        without_structural
    };
    let actual: Vec<_> = (0..rows.len())
        .map(|row| RowIdentity::from(rows.at(row).unwrap()))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(rows.at(rows.len()), None);
    assert_eq!(rows.at(usize::MAX), None);
    for entry_index in 0..=log.parsed().entries().len() {
        assert_eq!(
            rows.row_of_exact_entry(entry_index),
            expected.iter().position(|row| *row == Entry(entry_index))
        );
        assert_eq!(
            rows.row_of_entry_at_or_after(entry_index),
            expected
                .iter()
                .position(|row| matches!(row, Entry(entry) if *entry >= entry_index))
        );
    }
    for session_index in 0..=log.parsed().boot_sessions().len() {
        assert_eq!(
            rows.row_of_boot_divider(session_index),
            expected
                .iter()
                .position(|row| matches!(row, Boot(session, _) if *session == session_index))
        );
    }
    let displayed: Vec<_> = expected
        .iter()
        .filter_map(|row| match row {
            Entry(index) => Some(*index),
            _ => None,
        })
        .collect();
    let ticks = ClockTicks::of(log.parsed(), &VisibleEntries::Matching(displayed.clone()));
    for (visible_row, entry_index) in displayed.into_iter().enumerate() {
        let table_row = rows.row_of_exact_entry(entry_index).unwrap();
        assert_eq!(
            rows.at(table_row),
            Some(LineTableRow::Entry {
                entry_index,
                visible_row
            })
        );
        assert_eq!(rows.ticks.tick(visible_row), ticks.tick(visible_row));
    }
    let largest = expected
        .iter()
        .filter_map(|row| match row {
            Entry(index) => Some(log.parsed().entries()[*index].line_number),
            Structural(index) | Boot(_, Some(index)) => {
                Some(log.parsed().structural_lines()[*index].line_number)
            }
            _ => None,
        })
        .max()
        .unwrap_or_default();
    assert_eq!(rows.largest_line_number, largest);
}

#[rstest]
#[case::one_entry("2026-01-01 00:00:01 only\n", None, vec![Boot(0, None), Entry(0)])]
#[case::one_hidden_entry("2026-01-01 00:00:01 only\n", Some("absent"), vec![])]
#[case::leading_reboots("--- Device reboot ---\n--- Device reboot ---\n2026-01-01 00:00:01 only\n", None, vec![Structural(0), Boot(0, Some(1)), Entry(0)])]
fn indexed_rows_preserve_empty_single_entry_and_leading_source_boundaries(
    #[case] source: &str,
    #[case] pattern: Option<&str>,
    #[case] expected: Vec<RowIdentity>,
) {
    let mut logs = LoadedLogs::default();
    let id = logs.push(source_log(source)).id();
    if let Some(pattern) = pattern {
        let (stack, _) = logs.filter_stack_mut_by_id(id).unwrap();
        stack.set_live_filter_text(pattern);
        stack.wait_for_queries();
    }
    let rows = LineTableRows::with_overlay(logs.get_by_id(id).unwrap(), true, None);
    assert_eq!(
        (0..rows.len())
            .map(|row| RowIdentity::from(rows.at(row).unwrap()))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(rows.at(rows.len()), None);
    assert_eq!(rows.row_of_exact_entry(usize::MAX), None);
    assert_eq!(rows.row_of_entry_at_or_after(usize::MAX), None);
}

#[test]
fn large_row_plans_share_entry_indices_and_ticks_with_one_reveal_overlay() {
    let source = "2026-01-01 00:00:01 keep\n2026-01-01 00:00:01 hidden\n".repeat(50_000);
    let mut logs = LoadedLogs::default();
    let id = logs.push(source_log(&source)).id();
    let all = LineTableRows::of(logs.get_by_id(id).unwrap());
    assert_eq!(
        (all.len(), all.insertions.len(), all.boot_rows.len()),
        (100_001, 1, 1)
    );
    let (stack, _) = logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text("keep");
    stack.wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    let before = LineTableRows::of(log);
    let revealed = LineTableRows::with_overlay(log, false, Some(50_001));
    assert_eq!((before.len(), before.insertions.len()), (50_001, 1));
    assert_eq!(
        (
            revealed.len(),
            revealed.insertions.len(),
            revealed.boot_rows.len()
        ),
        (50_002, 1, 1)
    );
    assert!(Arc::ptr_eq(
        &revealed.entries.filtered,
        &log.filters().shared_visible_entries()
    ));
    assert!(Arc::ptr_eq(
        &revealed.ticks.filtered,
        &log.filters().shared_clock_ticks()
    ));
    assert!(Arc::ptr_eq(
        &before.entries.filtered,
        &revealed.entries.filtered
    ));
    assert!(Arc::ptr_eq(
        &before.ticks.filtered,
        &revealed.ticks.filtered
    ));
    let overlay = revealed.entries.reveal.as_ref().unwrap();
    assert_eq!(overlay.entry_index, 50_001);
    assert_eq!(overlay.visible_row, 25_001);
    assert_eq!(revealed.row_of_exact_entry(50_001), Some(25_002));
    assert_eq!(revealed.row_of_entry_at_or_after(50_003), Some(25_004));
}

struct ClockBoundaryCase {
    preceding: &'static str,
    revealed: &'static str,
    following: &'static str,
}

#[rstest]
#[case::new_day_and_back(ClockBoundaryCase {
    preceding: "2026-01-01 00:00:01",
    revealed: "2026-01-02 00:00:01",
    following: "2026-01-01 00:00:02",
}, vec![Boot(0, None), Entry(0), Day(1), Entry(1), Day(2), Entry(2)])]
#[case::boundary_after_reveal(ClockBoundaryCase {
    preceding: "2026-01-01 00:00:01",
    revealed: "2026-01-01 00:00:02",
    following: "2026-01-02 00:00:01",
}, vec![Boot(0, None), Entry(0), Entry(1), Day(2), Entry(2)])]
#[case::boundary_before_reveal(ClockBoundaryCase {
    preceding: "2026-01-01 00:00:01",
    revealed: "2026-01-02 00:00:01",
    following: "2026-01-02 00:00:02",
}, vec![Boot(0, None), Entry(0), Day(1), Entry(1), Entry(2)])]
#[case::minute_boundary(ClockBoundaryCase {
    preceding: "2026-01-01 00:00:01",
    revealed: "2026-01-01 00:01:01",
    following: "2026-01-01 00:01:02",
}, vec![Boot(0, None), Entry(0), Entry(1), Entry(2)])]
fn reveal_overlays_recompute_adjacent_clock_boundaries(
    #[case] timestamps: ClockBoundaryCase,
    #[case] expected: Vec<RowIdentity>,
) {
    let source = format!(
        "{} keep\n{} hidden\n{} keep\n",
        timestamps.preceding, timestamps.revealed, timestamps.following
    );
    let mut logs = LoadedLogs::default();
    let id = logs.push(source_log(&source)).id();
    let (stack, _) = logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text("keep");
    stack.wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    let rows = LineTableRows::with_overlay(log, false, Some(1));
    assert_eq!(
        (0..rows.len())
            .map(|row| RowIdentity::from(rows.at(row).unwrap()))
            .collect::<Vec<_>>(),
        expected
    );
    let reference = ClockTicks::of(log.parsed(), &VisibleEntries::All { entry_count: 3 });
    for row in 0..3 {
        assert_eq!(rows.ticks.tick(row), reference.tick(row));
    }
}
