use gt_log_view::{AttachmentSettingsStatus, FilterPattern, FilterStack, PreparedFilterCondition};
use gt_store::{DatabaseRef, LogAttachmentId, StoredLogFilterCondition};

use super::message_readability::RenderedMessage;
use super::*;

fn replace_condition_text(harness: &mut Harness<ViewerState>, text: &str) {
    let field =
        By::new().predicate(|node| node.author_id() == Some(filters::CONDITION_EDIT_FIELD_ID));
    harness.get(field.clone()).scroll_to_me();
    harness.run_steps(3);
    harness.get(field).click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.into()));
    harness.run_steps(3);
}

fn click_condition_action(harness: &mut Harness<ViewerState>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    run_until_the_scans_land(harness);
}

#[rstest]
#[case::default(VIEWER_SIZE, "log_condition_replacement")]
#[case::narrow(NARROW_VIEWPORT, "log_condition_replacement_narrow")]
#[case::short(SHORT_VIEWPORT, "log_condition_replacement_short")]
fn condition_edits_preserve_shared_effects_and_selected_text_highlights(
    #[case] viewport: egui::Vec2,
    #[case] snapshot: &str,
) {
    let mut rendering = TestHarness::builder().size(viewport).ui_state(
        viewer_ui,
        viewer_state(Vec::new(), &[("conditions.log", BOOLEAN_FILTER_LOG)]),
    );
    let harness = &mut rendering.inner;
    harness.run_steps(3);
    type_into_live_filter(harness, "alpha");
    harness.get_by_label(filters::ADD_FILTER_LABEL).click();
    run_until_the_scans_land(harness);
    let highlight =
        gt_ui_theme::LOG_LIVE_FILTER.resolve(harness.ctx.global_style().visuals.dark_mode);
    assert_eq!(
        RenderedMessage::of(harness, "receiver: alpha beta gamma").color_of("alpha"),
        highlight
    );
    click_chip_effect_action(
        harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::OtherEffect,
    );
    let before = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    let id = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .chips()
        .first()
        .unwrap()
        .id();
    click_chip_effect_action(harness, 0, FilterEffect::Table, filters::ChipControl::Edit);
    click_condition_action(harness, filters::REGEX_TOGGLE_LABEL);
    replace_condition_text(harness, "[");
    assert!(
        harness
            .get_by_label(filters::APPLY_CONDITION_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        before
    );
    assert_eq!(match_count(harness), "2 of 4");
    assert!(
        harness
            .query(By::new().label_contains("unclosed character class"))
            .is_some()
    );
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    rendering.snapshot_with_color_tolerance(snapshot);
    let harness = &mut rendering.inner;
    click_condition_action(harness, "Cancel");
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        before
    );
    click_chip_effect_action(harness, 0, FilterEffect::Map, filters::ChipControl::Edit);
    click_condition_action(harness, filters::REGEX_TOGGLE_LABEL);
    replace_condition_text(harness, "beta|gamma");
    click_condition_action(harness, filters::APPLY_CONDITION_LABEL);
    let filters = harness.state().shown_log().unwrap().filters();
    assert_eq!(filters.selected_chip(), Some(id));
    assert_eq!(filters.chips().first().unwrap().id(), id);
    assert_eq!(
        filters.to_stored_stack().chips().first().unwrap().effects,
        before.chips().first().unwrap().effects
    );
    assert_eq!(
        filters.chip(id).unwrap().pattern(),
        &FilterPattern::regex("beta|gamma")
    );
    assert_eq!(
        RenderedMessage::of(harness, "receiver: alpha beta gamma").color_of("beta"),
        highlight
    );
    assert_ne!(
        RenderedMessage::of(harness, "receiver: alpha beta gamma").color_of("alpha"),
        highlight
    );
    assert_eq!(harness.get_all_by_label("Regex").count(), 2);
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
}

#[test]
fn replacement_drafts_and_selection_remain_bound_to_each_logs_condition() {
    let second = BOOLEAN_FILTER_LOG.replace("receiver:", "sensor:");
    let mut state = viewer_state(
        Vec::new(),
        &[("first.log", BOOLEAN_FILTER_LOG), ("second.log", &second)],
    );
    let ids = state
        .logs
        .iter_with_ids()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    for id in &ids {
        let (stack, _) = state.logs.filter_stack_mut_by_id(*id).unwrap();
        stack.set_live_filter_text("alpha");
        stack.add_live_filter_as_chip().unwrap();
        stack.set_live_filter_text("gamma");
    }
    let mut harness = harness_from(state);
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::Edit,
    );
    click_condition_action(&mut harness, filters::REGEX_TOGGLE_LABEL);
    replace_condition_text(&mut harness, "[");
    harness.get_by_label("first.log").click();
    harness.run_steps(3);
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::Edit,
    );
    replace_condition_text(&mut harness, "beta");
    click_condition_action(&mut harness, filters::APPLY_CONDITION_LABEL);
    harness.get_by_label("second.log").click();
    harness.run_steps(3);
    let field = harness.get(
        By::new().predicate(|node| node.author_id() == Some(filters::CONDITION_EDIT_FIELD_ID)),
    );
    assert_eq!(field.accesskit_node().value().as_deref(), Some("["));
    assert!(
        harness
            .get_by_label(filters::APPLY_CONDITION_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    click_condition_action(&mut harness, "Cancel");
    assert!(
        harness
            .state()
            .logs
            .iter_with_ids()
            .all(|(_, log)| log.filters().live_filter_text() == "gamma")
    );
    let patterns = ids
        .into_iter()
        .map(|id| {
            harness
                .state()
                .logs
                .get_by_id(id)
                .unwrap()
                .filters()
                .chips()
                .first()
                .unwrap()
                .pattern()
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        patterns,
        vec![FilterPattern::plain("beta"), FilterPattern::plain("alpha")]
    );
}

#[test]
fn applied_condition_replacements_use_acknowledged_attachment_settings() {
    let mut state = viewer_state(Vec::new(), &[("saved.log", BOOLEAN_FILTER_LOG)]);
    let id = state.first_loaded_log();
    let (stack, _) = state.logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text("alpha");
    stack.add_live_filter_as_chip().unwrap();
    let saved = state.shown_log().unwrap().settings_snapshot();
    state.logs.save_attachment(
        id,
        LogAttachmentRef {
            recording: DatabaseRef {
                identity: "receiver".into(),
                group_name: "walk".into(),
            },
            id: LogAttachmentId::new_random(),
        },
        saved,
        &state.recordings.view(),
    );
    let mut harness = harness_from(state);
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::Edit,
    );
    replace_condition_text(&mut harness, "beta");
    click_condition_action(&mut harness, filters::APPLY_CONDITION_LABEL);
    let write = harness
        .state_mut()
        .logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    let persisted = write.settings().filters.clone();
    assert_eq!(
        persisted.chips().first().unwrap().condition,
        StoredLogFilterCondition::Message {
            text: "beta".into(),
            regex: false
        }
    );
    assert!(
        harness
            .state_mut()
            .logs
            .acknowledge_settings(write.complete(Ok(())))
    );
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
    let reopened = FilterStack::from_stored_stack(
        Arc::new(harness.state().shown_log().unwrap().parsed().clone()),
        &persisted,
    );
    assert_eq!(
        reopened.chips().first().unwrap().pattern(),
        &FilterPattern::plain("beta")
    );
}

#[test]
fn selecting_and_removing_conditions_updates_inline_explanations_independently_of_mode() {
    let mut state = viewer_state(Vec::new(), &[("conditions.log", BOOLEAN_FILTER_LOG)]);
    let id = state.first_loaded_log();
    let (stack, slots) = state.logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text("alpha");
    let first = stack.add_live_filter_as_chip().unwrap();
    stack.set_live_filter_regex(true);
    stack.set_live_filter_text("alpha");
    stack.add_live_filter_as_map_highlight(slots).unwrap();
    let mut harness = harness_from(state);
    harness.get_by_label("Text (all terms)");
    harness.get_by_label("Regex");
    let (stack, _) = harness.state_mut().logs.filter_stack_mut_by_id(id).unwrap();
    let replacement = PreparedFilterCondition::try_from(&LiveFilterDraft::Message {
        text: "alpha beta".into(),
        regex: false,
    })
    .unwrap();
    assert!(stack.replace_chip_condition(first, replacement));
    run_until_the_scans_land(&mut harness);
    harness
        .get(chip_effect_control(
            &harness,
            1,
            FilterEffect::Map,
            filters::ChipControl::Value,
        ))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness
            .get(chip_effect_control(
                &harness,
                1,
                FilterEffect::Map,
                filters::ChipControl::Value
            ))
            .accesskit_node()
            .toggled(),
        Some(Toggled::True)
    );
    let highlight =
        gt_ui_theme::LOG_LIVE_FILTER.resolve(harness.ctx.global_style().visuals.dark_mode);
    assert_ne!(
        RenderedMessage::of(&harness, "receiver: alpha beta gamma").color_of("beta"),
        highlight
    );
    harness
        .get(chip_effect_control(
            &harness,
            0,
            FilterEffect::Table,
            filters::ChipControl::Value,
        ))
        .click();
    harness.run_steps(3);
    assert_eq!(
        RenderedMessage::of(&harness, "receiver: alpha beta gamma").color_of("beta"),
        highlight
    );
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::Remove,
    );
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .selected_chip(),
        None
    );
    assert_ne!(
        RenderedMessage::of(&harness, "receiver: alpha beta gamma").color_of("beta"),
        highlight
    );
}

#[test]
fn removing_one_effect_preserves_the_shared_conditions_replacement_draft() {
    let mut harness = harness_of(Vec::new(), &[("conditions.log", BOOLEAN_FILTER_LOG)]);
    type_into_live_filter(&mut harness, "alpha");
    harness.get_by_label(filters::ADD_FILTER_LABEL).click();
    run_until_the_scans_land(&mut harness);
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::OtherEffect,
    );
    let id = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .chips()
        .first()
        .unwrap()
        .id();
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::Edit,
    );
    click_condition_action(&mut harness, filters::REGEX_TOGGLE_LABEL);
    replace_condition_text(&mut harness, "[");
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::Remove,
    );
    let field = harness.get(
        By::new().predicate(|node| node.author_id() == Some(filters::CONDITION_EDIT_FIELD_ID)),
    );
    assert_eq!(field.accesskit_node().value().as_deref(), Some("["));
    assert!(
        harness
            .get_by_label(filters::APPLY_CONDITION_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    click_condition_action(&mut harness, "Cancel");
    let chip = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .chip(id)
        .unwrap();
    assert!(!chip.has_effect(FilterEffect::Table));
    assert!(chip.has_effect(FilterEffect::Map));
    assert_eq!(chip.pattern(), &FilterPattern::plain("alpha"));
}
