use gt_store::StoredLogFilterStack;

use super::*;

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
    let generation = harness.state().logs.map_matches_generation();
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
        assert_eq!(harness.state().logs.map_matches_generation(), generation);
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
