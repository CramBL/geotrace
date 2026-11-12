use std::ops::RangeInclusive;

use gt_loaded_files::LoadedFiles;
use gt_logfile::YearResolution;
use rstest::rstest;

use super::row_plan::SOURCE;
use super::*;

#[rstest]
#[case::hidden_entry(3, false, Some(SourceRow::ShowEntry(1)))]
#[case::hidden_structural(2, false, Some(SourceRow::ShowStructuralLine(0)))]
#[case::visible_structural(2, true, None)]
#[case::summary_tail(8, false, Some(SourceRow::ShowStructuralLine(3)))]
fn physical_targets_resolve_exact_rows_and_return_to_the_filtered_source(
    #[case] target_line: u32,
    #[case] show_structural: bool,
    #[case] revealed: Option<SourceRow>,
) {
    let mut logs = LoadedLogs::default();
    let id = logs.push(super::source_log(SOURCE)).id();
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text("keep");
    filters.wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    let stored = log.filters().to_stored_stack();
    let visible = log.filters().visible_entries().clone();
    let mut viewer = LogViewerWindow::new();
    viewer.open_on_log(id);
    viewer.show_structural_lines = show_structural;
    let prior = TableReadingAnchor {
        log: id,
        source_line: 4,
        row: TableRowIdentity::Entry(2),
        offset_within_row_px: 3.5,
    };
    viewer.log_state_mut(id).table_reading_anchor = Some(prior);
    assert!(viewer.navigate_to_source(log, id, SourceRequest::NavigateToLine(target_line)));
    let state = viewer.log_state_mut(id);
    assert_eq!(state.source_reveal.map(SourceReveal::source), revealed);
    let pending = state.scroll_to_source_line.unwrap();
    assert_eq!(pending.source_line, target_line);
    let rows = viewer.table_rows(log, id);
    let row = rows.row_at_reading_anchor(log.parsed(), pending);
    assert_eq!(
        rows.at(row).unwrap().source_line(log.parsed()),
        Some(target_line)
    );
    assert_eq!(log.filters().to_stored_stack(), stored);
    assert_eq!(*log.filters().visible_entries(), visible);
    if revealed.is_some() {
        viewer.return_to_filtered_results(id);
        assert_eq!(viewer.log_state_mut(id).scroll_to_source_line, Some(prior));
        assert_eq!(viewer.log_state_mut(id).source_reveal, None);
    }
}

#[rstest]
fn replacing_a_reveal_retains_the_original_filtered_return_anchor(
    #[values(false, true)] empty: bool,
) {
    let mut logs = LoadedLogs::default();
    let id = logs.push(super::source_log(SOURCE)).id();
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text(if empty { "absent" } else { "keep" });
    filters.wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    let mut viewer = LogViewerWindow::new();
    viewer.open_on_log(id);
    let prior = (!empty).then_some(TableReadingAnchor {
        log: id,
        source_line: 4,
        row: TableRowIdentity::Entry(2),
        offset_within_row_px: 2.0,
    });
    viewer.log_state_mut(id).table_reading_anchor = prior;
    assert!(viewer.navigate_to_source(log, id, SourceRequest::NavigateToEntry(1)));
    viewer.log_state_mut(id).table_reading_anchor = viewer.log_state_mut(id).scroll_to_source_line;
    assert!(viewer.navigate_to_source(log, id, SourceRequest::NavigateToEntry(3)));
    viewer.return_to_filtered_results(id);
    assert_eq!(viewer.log_state_mut(id).scroll_to_source_line, prior);
    assert_eq!(viewer.log_state_mut(id).source_reveal, None);
    let filtered = viewer.table_rows(log, id);
    assert_eq!(
        filtered.entries.len(),
        log.filters().visible_entries().len()
    );
}

#[test]
fn inactive_chronology_rebuilds_preserve_the_target_and_filtered_return_source() {
    let files = LoadedFiles::new();
    let mut logs = LoadedLogs::default();
    let id = logs
        .push(super::source_log(gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG))
        .id();
    assert!(
        logs.get_mut_by_id(id)
            .unwrap()
            .resolve_years_as(YearResolution::PlaceLastEntryInYear(2024), &files.view())
    );
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text("leap");
    filters.wait_for_queries();
    let mut viewer = LogViewerWindow::new();
    viewer.open_on_log(id);
    let prior = TableReadingAnchor {
        log: id,
        source_line: 4,
        row: TableRowIdentity::Entry(3),
        offset_within_row_px: 3.25,
    };
    viewer.log_state_mut(id).table_reading_anchor = Some(prior);
    assert!(viewer.navigate_to_source(
        logs.get_by_id(id).unwrap(),
        id,
        SourceRequest::NavigateToEntry(1)
    ));
    let reveal = viewer.log_state_mut(id).source_reveal;
    let other = logs.push(super::source_log(SOURCE)).id();
    viewer.open_on_log(other);
    assert!(
        logs.get_mut_by_id(id)
            .unwrap()
            .resolve_years_as(YearResolution::Infer, &files.view())
    );
    let log = logs.get_by_id(id).unwrap();
    viewer.preserve_source_position_after_chronology_change(log, id);
    assert_eq!(viewer.log_state_mut(id).source_reveal, reveal);
    let pending = viewer.log_state_mut(id).scroll_to_source_line.unwrap();
    let rows = viewer.table_rows(log, id);
    assert_eq!(
        rows.row_at_reading_anchor(log.parsed(), pending),
        rows.row_of_exact_entry(1).unwrap()
    );
    viewer.return_to_filtered_results(id);
    let restored = viewer.log_state_mut(id).scroll_to_source_line.unwrap();
    assert_eq!(restored, prior);
    let rows = viewer.table_rows(log, id);
    assert_eq!(
        rows.row_at_reading_anchor(log.parsed(), restored),
        rows.row_of_exact_entry(3).unwrap()
    );
    assert_eq!(viewer.selected_log(), Some(other));
}

#[rstest]
#[case::excluded("hidden", true)]
#[case::still_visible("first", false)]
fn the_viewer_retains_checked_targets_during_pending_table_scans(
    #[case] next_filter: &str,
    #[case] excluded: bool,
) {
    let mut logs = LoadedLogs::default();
    let id = logs.push(super::source_log(SOURCE)).id();
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text("keep");
    filters.wait_for_queries();
    let prior = TableReadingAnchor {
        log: id,
        source_line: 4,
        row: TableRowIdentity::Entry(2),
        offset_within_row_px: 2.0,
    };
    let mut viewer = LogViewerWindow::new();
    viewer.open_on_log(id);
    viewer.log_state_mut(id).table_reading_anchor = Some(prior);
    logs.filter_stack_mut_by_id(id)
        .unwrap()
        .0
        .set_live_filter_text(next_filter);
    let log = logs.get_by_id(id).unwrap();
    assert!(log.filters().is_table_query_pending());
    assert!(viewer.navigate_to_source(log, id, SourceRequest::NavigateToEntry(0)));
    logs.filter_stack_mut_by_id(id)
        .unwrap()
        .0
        .wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    let rows = viewer.table_rows(log, id);
    let target = viewer.log_state_mut(id).scroll_to_source_line.unwrap();
    assert_eq!(
        rows.row_at_reading_anchor(log.parsed(), target),
        rows.row_of_exact_entry(0).unwrap()
    );
    assert_eq!(viewer.log_state_mut(id).source_reveal.is_some(), excluded);
    if excluded {
        viewer.return_to_filtered_results(id);
        assert_eq!(viewer.log_state_mut(id).scroll_to_source_line, Some(prior));
        let filtered = viewer.table_rows(log, id);
        assert_eq!(filtered.row_of_exact_entry(0), None);
    }
}

#[rstest]
fn semantic_invalidation_preserves_the_return_origin_before_replacement_navigation(
    #[values(false, true)] invalidate_before_navigation: bool,
    #[values(false, true)] empty_origin: bool,
) {
    let mut logs = LoadedLogs::default();
    let id = logs.push(super::source_log(SOURCE)).id();
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text(if empty_origin { "absent" } else { "keep" });
    filters.wait_for_queries();
    let prior = (!empty_origin).then_some(TableReadingAnchor {
        log: id,
        source_line: 4,
        row: TableRowIdentity::Entry(2),
        offset_within_row_px: 1.5,
    });
    let mut viewer = LogViewerWindow::new();
    viewer.open_on_log(id);
    viewer.log_state_mut(id).table_reading_anchor = prior;
    assert!(viewer.navigate_to_source(
        logs.get_by_id(id).unwrap(),
        id,
        SourceRequest::NavigateToEntry(1)
    ));
    viewer.log_state_mut(id).table_reading_anchor = viewer.log_state_mut(id).scroll_to_source_line;
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_regex(true);
    filters.set_live_filter_text("middle|second");
    filters.wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    if invalidate_before_navigation {
        viewer.clear_invalid_source_reveal(log, id);
    }
    assert!(viewer.navigate_to_source(log, id, SourceRequest::NavigateToEntry(3)));
    viewer.return_to_filtered_results(id);
    assert_eq!(viewer.log_state_mut(id).scroll_to_source_line, prior);
}

#[rstest]
#[case::first(1, 1..=6)]
#[case::middle(9, 4..=14)]
#[case::last(18, 13..=18)]
fn bounded_context_merges_physical_rows_without_duplicates(
    #[case] target: u32,
    #[case] expected: RangeInclusive<u32>,
    #[values(false, true)] structural: bool,
) {
    let text = "2026-01-01 23:59:59 keep\n\n--- Device reboot ---\n2026-01-02 00:00:01 hidden\n \t\n2026-01-02 00:01:01 keep\n".repeat(3);
    let mut logs = LoadedLogs::default();
    let id = logs.push(source_log(&text)).id();
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text("absent");
    filters.wait_for_queries();
    let log = logs.get_by_id(id).unwrap();
    let mut viewer = LogViewerWindow::new();
    viewer.show_structural_lines = structural;
    assert!(viewer.navigate_to_source(log, id, SourceRequest::ShowSurroundingLines(target)));
    let rows = viewer.table_rows(log, id);
    let mut physical = (0..rows.len())
        .filter_map(|row| match rows.at(row).unwrap() {
            LineTableRow::Entry { .. }
            | LineTableRow::Structural { .. }
            | LineTableRow::RenderRawSource { .. } => {
                rows.at(row).unwrap().source_line(log.parsed())
            }
            LineTableRow::BootDivider {
                structural_index: Some(index),
                ..
            } => Some(
                log.parsed()
                    .structural_lines()
                    .get(index)
                    .unwrap()
                    .line_number,
            ),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(physical.windows(2).all(|pair| pair.first() < pair.get(1)));
    if structural {
        physical.retain(|line| expected.contains(line));
    }
    assert_eq!(physical, expected.clone().collect::<Vec<_>>());
    assert_eq!(
        rows.additional_source_count(),
        expected
            .filter(|line| !structural || !matches!(line, 3 | 9 | 15))
            .count()
    );
    assert_eq!(
        rows.at(rows
            .row_of_identity(TableRowIdentity::RenderRawSource(5))
            .unwrap_or(usize::MAX)),
        if (target.saturating_sub(5)..=target + 5).contains(&5) {
            Some(LineTableRow::RenderRawSource { line_number: 5 })
        } else {
            None
        }
    );
}

#[test]
fn raw_source_requests_preserve_return_origins_and_reject_missing_lines() {
    let mut logs = LoadedLogs::default();
    let id = logs
        .push(source_log(
            "2026-01-01 00:00:00 keep\n \t\n2026-01-01 00:00:01 hidden\n",
        ))
        .id();
    let log = logs.get_by_id(id).unwrap();
    let mut viewer = LogViewerWindow::new();
    let prior = TableReadingAnchor {
        log: id,
        source_line: 1,
        row: TableRowIdentity::Entry(0),
        offset_within_row_px: 2.0,
    };
    viewer.log_state_mut(id).table_reading_anchor = Some(prior);
    assert!(viewer.navigate_to_source(log, id, SourceRequest::NavigateToLine(2)));
    assert_eq!(
        viewer.log_state_mut(id).source_reveal.unwrap().source(),
        SourceRow::RenderRawLine(2)
    );
    let pending = viewer.log_state_mut(id).scroll_to_source_line;
    for line in [0, 4] {
        assert!(!viewer.navigate_to_source(log, id, SourceRequest::NavigateToLine(line)));
        assert_eq!(viewer.log_state_mut(id).scroll_to_source_line, pending);
    }
    assert!(viewer.navigate_to_source(log, id, SourceRequest::ShowSurroundingLines(2)));
    viewer.return_to_filtered_results(id);
    assert_eq!(viewer.log_state_mut(id).scroll_to_source_line, Some(prior));
    assert_eq!(viewer.table_rows(log, id).additional_source_count(), 0);
}

#[test]
fn context_extensions_remain_bounded_across_pending_scans_and_chronology_rebuilds() {
    let files = LoadedFiles::new();
    let text = (0..200)
        .map(|index| format!("Jan 01 00:00:00 kernel: entry {index}\n"))
        .collect::<String>();
    let mut logs = LoadedLogs::default();
    let id = logs.push(source_log(&text)).id();
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text("absent");
    let mut viewer = LogViewerWindow::new();
    assert!(viewer.navigate_to_source(
        logs.get_by_id(id).unwrap(),
        id,
        SourceRequest::ShowSurroundingLines(100)
    ));
    logs.filter_stack_mut_by_id(id)
        .unwrap()
        .0
        .wait_for_queries();
    for _ in 0..20 {
        let log = logs.get_by_id(id).unwrap();
        assert!(viewer.navigate_to_source(log, id, SourceRequest::ShowSurroundingLines(100)));
    }
    let log = logs.get_by_id(id).unwrap();
    let reveal = viewer.log_state_mut(id).source_reveal.unwrap();
    assert!(!reveal.can_extend(log.parsed()));
    assert_eq!(viewer.table_rows(log, id).additional_source_count(), 101);
    viewer.preserve_source_position_after_chronology_change(log, id);
    assert!(
        logs.get_mut_by_id(id)
            .unwrap()
            .resolve_years_as(YearResolution::PlaceLastEntryInYear(2024), &files.view())
    );
    assert_eq!(viewer.log_state_mut(id).source_reveal, Some(reveal));
    assert_eq!(
        viewer
            .table_rows(logs.get_by_id(id).unwrap(), id)
            .additional_source_count(),
        101
    );
    let (filters, _) = logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text("entry");
    viewer.table_rows(logs.get_by_id(id).unwrap(), id);
    assert_eq!(viewer.log_state_mut(id).source_reveal, None);
}
