use gt_fmt::MIDDLE_DOT;

use super::*;

#[test]
fn a_literal_filename_matching_a_generated_title_remains_distinct() {
    let mut state = viewer_state(
        vec![recording("walk.gtd", 55.0)],
        &[("nav.log", SECOND_LOG), ("nav.log", THIRD_LOG)],
    );
    let first = state.first_loaded_log();
    let literal = format!("nav.log {MIDDLE_DOT} walk.gtd {MIDDLE_DOT} #{first}");
    let parsed = gt_logfile::parse_log(LOG_WITH_EVERY_ROW_KIND.into(), log_start()).unwrap();
    state.logs.push(LoadedLog::new(
        Some(literal),
        parsed,
        AssociationWindow::try_from_seconds(60).unwrap(),
    ));
    let ids: Vec<_> = state.logs.iter_with_ids().map(|(id, _)| id).collect();
    let recording = state.recordings.view().entries().next().unwrap().id();
    for id in &ids {
        state
            .logs
            .get_mut_by_id(*id)
            .unwrap()
            .anchor_to_loaded_recording(Some(recording), &state.recordings.view());
    }
    let names = RecordingNames::resolve(state.recordings.view(), "{filename}");
    let groups = log_list::group_logs_by_recording(
        &state.logs,
        state.recordings.view(),
        &names,
        &state.attachments,
    );
    let tabs = log_list::tabs_in_session_order(&state.logs, &groups);
    assert_eq!(tabs.len(), ids.len());
    for (index, tab) in tabs.iter().enumerate() {
        assert!(
            tabs.iter()
                .skip(index + 1)
                .all(|other| tab.title != other.title)
        );
    }
}

#[test]
fn saved_contexts_with_equal_names_have_distinct_tabs_and_origin_help() {
    let first = stored_attachment();
    let second = stored_attachment();
    let mut state = viewer_state_over_a_stored_recording(first.clone());
    load_the_stored_log(&mut state, &first);
    load_the_stored_log(&mut state, &second);
    let ids: Vec<_> = state.logs.iter_with_ids().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2);
    let names = RecordingNames::resolve(state.recordings.view(), "{filename}");
    let groups = log_list::group_logs_by_recording(
        &state.logs,
        state.recordings.view(),
        &names,
        &state.attachments,
    );
    let tabs = log_list::tabs_in_session_order(&state.logs, &groups);
    assert_eq!(tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(), ids);
    assert_ne!(tabs[0].title, tabs[1].title);
    assert!(tabs[0].title.contains("walk.gtd"));
    assert!(tabs[0].hover.contains(&first.id.to_string()));
    assert!(tabs[1].hover.contains(&second.id.to_string()));
}

#[test]
fn overflow_tabs_remain_one_row_and_the_picker_activates_only_its_chosen_log() {
    let logs: Vec<_> = (0..20)
        .map(|index| {
            (
                format!("log-{index}.log"),
                format!("2026-05-29 18:48:25 service{index}: entry\n"),
            )
        })
        .collect();
    let borrowed: Vec<_> = logs
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let mut rendering = rendering_harness_of(Vec::new(), &borrowed);
    let harness = &mut rendering.inner;
    let ids: Vec<_> = harness
        .state()
        .logs
        .iter_with_ids()
        .map(|(id, _)| id)
        .collect();
    let active = harness.state().viewer.selected_log();
    for id in [ids[0], ids[1]] {
        harness
            .state_mut()
            .viewer
            .restored_logs
            .note_log_loaded_with_a_recording(id);
    }
    harness.run_steps(3);
    assert_eq!(harness.state().viewer.selected_log(), active);
    assert_eq!(harness.state().viewer.restored_logs.count(), 2);
    let tops: Vec<_> = harness
        .get_all(By::new().role(Role::Tab))
        .map(|node| node.rect().top())
        .collect();
    assert!(tops.len() > 1);
    assert!(tops.iter().all(|y| (y - tops[0]).abs() < 1.0));
    harness
        .get_by_label_contains(log_list::OPEN_LOG_LABEL)
        .click();
    harness.run_steps(2);
    harness.get_by_label("Find a log").click();
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("log-0".to_owned()));
    harness.run_steps(2);
    harness
        .get(By::new().role(Role::Button).label("log-0.log"))
        .click();
    harness.run_steps(4);
    assert_eq!(harness.state().viewer.selected_log(), Some(ids[0]));
    assert_eq!(harness.state().viewer.restored_logs.count(), 1);
    let selected_tab = harness.get(By::new().role(Role::Tab).label("log-0.log"));
    assert_eq!(selected_tab.accesskit_node().is_selected(), Some(true));
    let tab = selected_tab.rect();
    assert!(
        harness
            .window_rect(LOG_VIEWER_TITLE)
            .unwrap()
            .contains(tab.center())
    );
    rendering.snapshot("log_viewer_tabs_overflow");
}
