use super::*;
use crate::app::log_viewer::line_table::DiagnosticTarget;

#[rstest]
#[case::single_entry(vec![CLICKED_ENTRY])]
#[case::multiple_entries(vec![CLICKED_ENTRY, CLICKED_ENTRY + 1])]
fn hidden_first_glyph_entries_preserve_table_scroll(
    #[case] entries: Vec<usize>,
    #[values(false, true)] diagnostic_reveal: bool,
) {
    let text = long_log(LONG_LOG_ENTRIES)
        .replace("navsyncd: entry", "navsyncd: keep entry")
        .replace("keep entry 120\n", "hidden entry 120\n");
    let mut harness = harness_of(Vec::new(), &[("long.log", &text)]);
    type_into_live_filter(&mut harness, "keep");
    let scroll = TableScroll::of(&harness);
    let id = harness.state().first_loaded_log();
    if diagnostic_reveal {
        let state = harness.state_mut();
        let log = state.logs.get_by_id(id).unwrap();
        state
            .viewer
            .navigate_to_diagnostic(log, id, DiagnosticTarget::Entry(CLICKED_ENTRY));
        harness.run_steps(3);
        harness.get_by_label("navsyncd: hidden entry 120");
    }
    harness.state_mut().viewer.scroll_to_row = Some(10);
    harness.run_steps(3);
    let before = scroll.offset_px(&harness);
    let visible = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .visible_entries()
        .clone();
    harness.state_mut().clicked_glyph = Some(LogMatchGlyph {
        log: id,
        color: LogMatchColor::LiveFilter,
        entry_indices: entries.clone(),
    });
    harness.run_steps(3);

    assert!((scroll.offset_px(&harness) - before).abs() < SCROLL_READING_TOLERANCE_PX);
    assert_eq!(harness.state().viewer.diagnostic_reveal, None);
    assert_eq!(harness.state().viewer.scroll_to_row, None);
    assert_eq!(
        harness
            .state()
            .viewer
            .clicked_glyph
            .as_ref()
            .unwrap()
            .entry_indices,
        entries
    );
    assert_eq!(
        *harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .visible_entries(),
        visible
    );
    harness.get_by_label(long_log_entry_timestamp(10).as_str());
    assert!(
        harness
            .query_by_label(long_log_entry_timestamp(CLICKED_ENTRY + 1).as_str())
            .is_none()
    );
}

#[rstest]
fn visible_first_glyph_entries_scroll_to_exact_source_rows(
    #[values(false, true)] structural: bool,
) {
    let text: String = (0..LONG_LOG_ENTRIES)
        .map(|index| {
            let separator = if index == CLICKED_ENTRY {
                "--- Device reboot ---\n"
            } else {
                ""
            };
            format!(
                "{separator}{} navsyncd: entry {index}\n",
                (log_start() + Duration::hours(index as i64))
                    .format(super::super::TIMESTAMP_FORMAT)
            )
        })
        .collect();
    let mut harness = harness_of(Vec::new(), &[("days.log", &text)]);
    harness.state_mut().viewer.show_structural_lines = structural;
    harness.run_steps(3);
    let first_time = long_log_entry_timestamp(0);
    let first_y = harness.get_by_label(&first_time).rect().top();
    let row_height = harness.ctx.global_style().spacing.interact_size.y;
    let id = harness.state().first_loaded_log();
    let state = harness.state_mut();
    let log = state.logs.get_by_id(id).unwrap();
    let first_row = state
        .viewer
        .table_rows(log, id)
        .row_of_exact_entry(0)
        .unwrap();
    let target_row = state
        .viewer
        .table_rows(log, id)
        .row_of_exact_entry(CLICKED_ENTRY)
        .unwrap();
    assert!(target_row > CLICKED_ENTRY + 2);
    harness.state_mut().clicked_glyph = Some(LogMatchGlyph {
        log: id,
        color: LogMatchColor::LiveFilter,
        entry_indices: vec![CLICKED_ENTRY, CLICKED_ENTRY + 1],
    });
    harness.run_steps(3);

    let target_time = format!(
        " {}",
        (log_start() + Duration::hours(CLICKED_ENTRY as i64))
            .format(super::super::TIMESTAMP_FORMAT)
    );
    let target_y = harness.get_by_label(&target_time).rect().top();
    assert!(
        (target_y - (first_y - first_row as f32 * row_height)).abs() < SCROLL_READING_TOLERANCE_PX
    );
    assert_eq!(harness.state().viewer.diagnostic_reveal, None);
}
