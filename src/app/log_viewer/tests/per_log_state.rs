use super::*;

#[test]
fn unresolved_logs_restore_their_unapplied_years_and_details_after_switching() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("first.log", gt_test_utils::UNRESOLVED_YEARS_LOG),
            ("second.log", gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG),
        ],
    );
    for (name, year) in [("second.log", 2028), ("first.log", 2032)] {
        harness.get_by_label(name).click();
        harness.run_steps(2);
        open_date_inspector(&mut harness);
        select_date_policy(&mut harness, unresolved_years::SET_YEAR_LABEL);
        chronology_policies::edit_last_entry_year(&mut harness, year);
    }
    unfold_the_summary_panel(&mut harness);
    for (name, year, details) in [("second.log", 2028.0, false), ("first.log", 2032.0, true)] {
        harness.get_by_label(name).click();
        harness.run_steps(2);
        assert_eq!(
            harness.state().viewer.active_state().inspector == Some(InspectorSection::ShowSummary),
            details
        );
        if details {
            open_date_inspector(&mut harness);
        }
        let field = harness.get(
            By::new()
                .predicate(|node| node.author_id() == Some(unresolved_years::LAST_ENTRY_YEAR_ID)),
        );
        assert_eq!(field.accesskit_node().numeric_value(), Some(year));
    }
    assert_eq!(harness.state().requests.resolve_years, None);
    assert!(
        harness
            .state()
            .logs
            .iter_with_ids()
            .all(|(_, log)| !log.parsed().chronology().is_resolved())
    );
}

#[test]
fn closing_and_external_unloading_prune_only_the_removed_log_contexts() {
    let mut state = viewer_state(
        Vec::new(),
        &[
            ("first.log", SECOND_LOG),
            ("second.log", LOG_WITH_EVERY_ROW_KIND),
        ],
    );
    let first = state.first_loaded_log();
    let second = state.viewer.selected_log().unwrap();
    state.viewer.log_state_mut(first).inspector = Some(InspectorSection::ShowSummary);
    state.viewer.log_state_mut(second).association_window_unit = AssociationWindowUnit::Minutes;
    state.viewer.log_state_mut(second).filter_editor_open = true;
    state
        .viewer
        .close_log(&mut state.logs, first, &[first, second]);
    assert!(!state.viewer.log_states.contains_key(&first));
    assert_eq!(state.viewer.selected_log(), Some(second));
    assert_eq!(
        state.viewer.active_state().association_window_unit,
        AssociationWindowUnit::Minutes
    );
    assert!(state.viewer.active_state().filter_editor_open);
    state.viewer.open = false;
    state.logs.remove_by_id(second);
    let mut harness = harness_from(state);
    harness.step();
    assert!(harness.state().viewer.log_states.is_empty());
    assert_eq!(harness.state().viewer.selected_log(), None);
}
