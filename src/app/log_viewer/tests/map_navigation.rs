use gt_store::StoredLogFilterStack;

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

#[test]
fn grouped_structured_filters_preserve_highlights_and_diagnostic_navigation_after_reload() {
    let mut state = viewer_state_over_a_stored_recording(stored_attachment());
    let parsed = gt_logfile::parse_log(LOG_WITH_EVERY_ROW_KIND.into(), log_start()).unwrap();
    let id = state
        .logs
        .push(LoadedLog::new(
            Some("navsyncd.log".to_owned()),
            parsed.clone(),
            Duration::seconds(ASSOCIATION_WINDOW_SECS),
        ))
        .id();
    let (stack, slots) = state.logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_scope(FilterScope::Service);
    stack.set_live_filter_text("navsyncd");
    stack.add_live_filter_as_chip();
    let group = stack.create_group();
    stack.set_group_operator(group, FilterGroupOperator::Any);
    stack.set_live_filter_scope(FilterScope::Message);
    stack.set_live_filter_text("fix acquired");
    stack.add_live_filter_as_chip();
    stack.set_live_filter_text("starting");
    let highlight = stack.add_live_filter_as_chip().unwrap();
    stack.add_chip_effect(highlight, FilterEffect::Map, slots);
    stack.remove_chip_effect(highlight, FilterEffect::Table, slots);
    stack.wait_for_queries();
    let stored = stack.to_stored_stack();
    let reference = attachment_ref();
    state.logs.save_attachment(
        id,
        reference.clone(),
        stored.clone(),
        &state.recordings.view(),
    );
    let saved = state
        .logs
        .get_by_id(id)
        .unwrap()
        .filters()
        .to_stored_stack();
    let serialized = serde_json::to_vec(&saved).unwrap();
    let restored: StoredLogFilterStack = serde_json::from_slice(&serialized).unwrap();
    state.logs.remove_by_id(id);
    let restored_id = state
        .logs
        .restore_attachment(
            LoadedLog::new(
                Some("navsyncd.log".to_owned()),
                parsed,
                Duration::seconds(ASSOCIATION_WINDOW_SECS),
            ),
            reference,
            restored,
            &state.recordings.view(),
        )
        .id();
    state
        .logs
        .filter_stack_mut_by_id(restored_id)
        .unwrap()
        .0
        .wait_for_queries();
    state.viewer.open_on_log(restored_id);
    let mut harness = harness_from(state);
    assert_eq!(match_count(&harness), "2 of 6");
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        stored
    );
    let cached = harness.state_mut().map_matches();
    assert_eq!(cached.layers().len(), 1);
    assert_eq!(cached.match_count(), 2);
    let expected = cached.clone();
    let allocation = cached.layers().as_ptr();
    unfold_the_summary_panel(&mut harness);
    harness.get_by_label("Line 6").click();
    harness.run_steps(3);
    harness.get_by_label(ORDER_ANOMALY_ENTRY_TIMESTAMP);
    assert_eq!(
        harness
            .state()
            .viewer
            .diagnostic_reveal
            .unwrap()
            .entry_index,
        4
    );
    assert_eq!(match_count(&harness), "2 of 6");
    for _ in 0..3 {
        harness.step();
        let cached = harness.state_mut().map_matches();
        assert_eq!(*cached, expected);
        assert_eq!(cached.layers().as_ptr(), allocation);
    }
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        stored
    );
}
