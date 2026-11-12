use egui::{Modal, Popup, TextEdit, Window};
use gt_log_view::SourceSearchStatus;

use super::*;
use crate::app::log_viewer::find;

#[rstest]
#[case::physical(egui::Modifiers::CTRL)]
#[case::logical_command(egui::Modifiers { command: true, ..egui::Modifiers::CTRL })]
fn control_tab_cycles_session_order_reveals_overflow_and_restores_log_contexts(
    #[case] control: egui::Modifiers,
) {
    let text = long_log(LONG_LOG_ENTRIES);
    let logs: Vec<_> = (0..20)
        .map(|index| {
            (
                format!("source-{index}.log"),
                text.replace("navsyncd", &format!("service{index}")),
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
    add_filter(harness, "entry 17");
    harness.get_by_label(find::FIND_LABEL).click();
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("entry 10".into()));
    assert!(harness.step_until(|harness| {
        harness
            .state()
            .viewer
            .active_state()
            .find
            .as_ref()
            .is_some_and(|find| find.search.status() == &SourceSearchStatus::ShowResults)
    }));
    let last = *ids.last().unwrap();
    {
        let state = harness.state_mut();
        state.viewer.navigate_to_source(
            state.logs.get_by_id(last).unwrap(),
            last,
            line_table::SourceRequest::ShowSurroundingLines(50),
        );
    }
    harness.run_steps(3);
    table_focus::focus_table(harness);
    let last_anchor = harness.state().viewer.active_state().table_reading_anchor;
    let last_reveal = harness.state().viewer.active_state().source_reveal;
    let last_find = harness
        .state()
        .viewer
        .active_state()
        .find
        .as_ref()
        .unwrap()
        .search
        .current()
        .cloned();
    harness
        .state_mut()
        .viewer
        .restored_logs
        .note_log_loaded_with_a_recording(ids[0]);
    harness.hover_at(POINTER_OFF_EVERY_WINDOW);
    harness.key_press_modifiers(control, egui::Key::Tab);
    harness.run_steps(4);
    assert_eq!(harness.state().viewer.selected_log(), Some(ids[0]));
    assert_eq!(harness.state().viewer.restored_logs.count(), 0);
    table_focus::assert_table_focused(harness);
    let first_tab = harness
        .get(By::new().role(Role::Tab).label("source-0.log"))
        .rect();
    let window = harness.window_rect(LOG_VIEWER_TITLE).unwrap();
    assert!(window.contains_rect(first_tab));
    let scroll = TableScroll::of(harness);
    press_scroll_key(harness, egui::Key::ArrowDown);
    assert!((scroll.offset_px(harness) - scroll.row_height_px).abs() < SCROLL_READING_TOLERANCE_PX);
    harness.key_press_modifiers(control | egui::Modifiers::SHIFT, egui::Key::Tab);
    harness.run_steps(4);
    assert_eq!(harness.state().viewer.selected_log(), Some(last));
    table_focus::assert_table_focused(harness);
    let state = harness.state().viewer.active_state();
    assert_eq!(state.table_reading_anchor, last_anchor);
    assert_eq!(state.source_reveal, last_reveal);
    assert_eq!(state.find.as_ref().unwrap().text, "entry 10");
    assert_eq!(
        state.find.as_ref().unwrap().search.current(),
        last_find.as_ref()
    );
    assert_eq!(
        harness.state().shown_log().unwrap().filters().chips().len(),
        1
    );
    let last_tab = harness
        .get(By::new().role(Role::Tab).label("source-19.log"))
        .rect();
    assert!(window.contains_rect(last_tab));
    if !control.command {
        rendering.snapshot_with_color_tolerance("log_keyboard_tab_overflow");
    }
    let harness = &mut rendering.inner;
    for id in [ids[0], ids[18]] {
        harness
            .state_mut()
            .viewer
            .restored_logs
            .note_log_loaded_with_a_recording(id);
    }
    for modifiers in [
        control,
        control | egui::Modifiers::SHIFT,
        control | egui::Modifiers::SHIFT,
    ] {
        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
    }
    harness.run_steps(3);
    assert_eq!(harness.state().viewer.selected_log(), Some(ids[18]));
    assert_eq!(harness.state().viewer.restored_logs.count(), 0);
    table_focus::assert_table_focused(harness);
}

#[test]
fn control_tab_preserves_an_unapplied_year_and_condition_draft() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("first.log", gt_test_utils::UNRESOLVED_YEARS_LOG),
            ("second.log", gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG),
        ],
    );
    let selected = harness.state().viewer.selected_log();
    open_date_inspector(&mut harness);
    select_date_policy(&mut harness, unresolved_years::SET_YEAR_LABEL);
    chronology_policies::edit_last_entry_year(&mut harness, 2032);
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.run_steps(3);
    type_into_live_filter(&mut harness, "unfinished condition");
    harness.key_press_modifiers(
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        egui::Key::Tab,
    );
    harness.run_steps(3);
    assert_eq!(harness.state().viewer.selected_log(), selected);
    assert_eq!(
        harness
            .get(
                By::new().predicate(
                    |node| node.author_id() == Some(unresolved_years::LAST_ENTRY_YEAR_ID)
                )
            )
            .accesskit_node()
            .numeric_value(),
        Some(2032.0)
    );
    assert!(harness.state().requests.resolve_years.is_none());
    table_focus::assert_table_focused(&harness);
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.run_steps(3);
    assert_eq!(live_filter_text(&harness), "unfinished condition");
}

#[test]
fn popup_escape_and_control_tab_remain_local() {
    let mut harness = harness_of(
        Vec::new(),
        &[("first.log", SOURCE_LOG), ("second.log", SECOND_LOG)],
    );
    table_focus::focus_table(&mut harness);
    let selected = harness.state().viewer.selected_log();
    harness.get_by_label("Go to…").click();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.run_steps(2);
    assert_eq!(harness.state().viewer.selected_log(), selected);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.state().viewer.open);
    assert!(!Popup::is_any_open(&harness.ctx));
}

#[derive(Clone, Copy)]
enum ExternalSurface {
    FocusWindow,
    OpenModal,
    ShowWindow,
}

struct ExternalWindowFixture {
    viewer: ViewerState,
    external: Option<ExternalSurface>,
    text: String,
}

#[rstest]
fn external_focus_and_modal_input_preserve_the_viewer(
    #[values(
        ExternalSurface::OpenModal,
        ExternalSurface::ShowWindow,
        ExternalSurface::FocusWindow
    )]
    surface: ExternalSurface,
) {
    let text = long_log(LONG_LOG_ENTRIES);
    let second = text.replace("navsyncd", "other-service");
    let state = ExternalWindowFixture {
        viewer: viewer_state(
            Vec::new(),
            &[
                ("first.log", text.as_str()),
                ("second.log", second.as_str()),
            ],
        ),
        external: None,
        text: String::new(),
    };
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_size(VIEWER_SIZE)
        .build_ui_state(
            |ui, state: &mut ExternalWindowFixture| {
                viewer_ui(ui, &mut state.viewer);
                let field = |ui: &mut egui::Ui| {
                    let response = ui.add(
                        TextEdit::singleline(&mut state.text)
                            .id(egui::Id::unique(OTHER_FIELD))
                            .hint_text(OTHER_FIELD),
                    );
                    if matches!(state.external, Some(ExternalSurface::FocusWindow)) {
                        response.request_focus();
                    }
                };
                match state.external {
                    Some(ExternalSurface::OpenModal) => {
                        Modal::new(egui::Id::unique("other_modal")).show(ui.ctx(), field);
                    }
                    Some(ExternalSurface::ShowWindow | ExternalSurface::FocusWindow) => {
                        Window::new("Other window").show(ui.ctx(), field);
                    }
                    None => {}
                }
            },
            state,
        );
    harness.run_steps(3);
    harness
        .state_mut()
        .viewer
        .viewer
        .active_state_mut()
        .table_focus_requested = true;
    harness.run_steps(2);
    harness.state_mut().external = Some(surface);
    harness.step();
    if matches!(surface, ExternalSurface::FocusWindow) {
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        assert!(harness.state().viewer.viewer.open);
    } else {
        harness.run_steps(2);
    }
    if matches!(surface, ExternalSurface::ShowWindow) {
        let before = harness
            .state()
            .viewer
            .viewer
            .active_state()
            .table_reading_anchor;
        harness.hover_at(POINTER_OFF_EVERY_WINDOW);
        harness.key_press(egui::Key::ArrowDown);
        harness.run_steps(3);
        assert_ne!(
            harness
                .state()
                .viewer
                .viewer
                .active_state()
                .table_reading_anchor,
            before
        );
    }
    harness
        .ctx
        .memory_mut(|memory| memory.request_focus(egui::Id::unique(OTHER_FIELD)));
    harness.run_steps(2);
    let selected = harness.state().viewer.viewer.selected_log();
    let anchor = harness
        .state()
        .viewer
        .viewer
        .active_state()
        .table_reading_anchor;
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    harness.key_press(egui::Key::End);
    harness.run_steps(3);
    assert_eq!(harness.state().viewer.viewer.selected_log(), selected);
    assert_eq!(
        harness
            .state()
            .viewer
            .viewer
            .active_state()
            .table_reading_anchor,
        anchor
    );
    assert!(harness.state().viewer.viewer.active_state().find.is_none());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.state().viewer.viewer.open);
}

#[test]
fn filter_escape_returns_focus_and_ignores_a_stale_dialog_target() {
    let mut harness = keyboard_scroll_harness();
    type_into_live_filter(&mut harness, "entry");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    table_focus::assert_table_focused(&harness);
    assert!(harness.state().viewer.open);
    assert!(!harness.state().viewer.active_state().filter_editor_open);
    add_filter(&mut harness, "entry");
    let committed = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    click_chip_effect_action(
        &mut harness,
        0,
        FilterEffect::Table,
        filters::ChipControl::Edit,
    );
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    table_focus::assert_table_focused(&harness);
    assert!(
        harness
            .state()
            .viewer
            .active_state()
            .condition_edit
            .is_none()
    );
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        committed
    );
    let loaded = harness.state().viewer.selected_log().unwrap();
    harness.state_mut().viewer.open = false;
    {
        let state = harness.state_mut();
        state.viewer.return_table_focus(loaded, &state.logs);
    }
    assert!(!harness.state().viewer.open);
    assert!(!harness.state().viewer.active_state().table_focus_requested);
    {
        let state = harness.state_mut();
        state.viewer.open = true;
        state.viewer.return_table_focus(UNLOADED_LOG, &state.logs);
    }
    assert_eq!(harness.state().viewer.selected_log(), Some(loaded));
    assert!(!harness.state().viewer.active_state().table_focus_requested);
    {
        let state = harness.state_mut();
        state.logs.remove_by_id(loaded);
        state.viewer.return_table_focus(loaded, &state.logs);
    }
    assert!(!harness.state().viewer.active_state().table_focus_requested);
}

const OTHER_FIELD: &str = "Other input";
