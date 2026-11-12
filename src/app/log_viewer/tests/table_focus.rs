use super::*;
use crate::app::log_viewer::{find, jump, keyboard_navigation};

pub(super) fn focus_table(harness: &mut Harness<'_, ViewerState>) {
    let id = harness.state().viewer.selected_log().unwrap();
    harness
        .state_mut()
        .viewer
        .log_state_mut(id)
        .table_focus_requested = true;
    harness.run_steps(2);
    assert_table_focused(harness);
}

pub(super) fn assert_table_focused(harness: &Harness<'_, ViewerState>) {
    let table = harness.get_by_label(keyboard_navigation::TABLE_LABEL);
    assert_eq!(table.accesskit_node().role(), Role::Table);
    assert!(table.accesskit_node().is_focused());
}

#[test]
fn tab_traversal_reaches_the_table_and_leaves_the_map_unchanged() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    harness.ctx.memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
            memory.surrender_focus(focused);
        }
    });
    let mut reached_table = false;
    for _ in 0..MAXIMUM_VIEWER_TAB_STEPS {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        if harness
            .get_by_label(keyboard_navigation::TABLE_LABEL)
            .accesskit_node()
            .is_focused()
        {
            reached_table = true;
            break;
        }
    }
    assert!(reached_table);
    assert_eq!(harness.state().map_center, None);
    harness.key_press(egui::Key::Tab);
    harness.run_steps(2);
    assert!(
        !harness
            .get_by_label(keyboard_navigation::TABLE_LABEL)
            .accesskit_node()
            .is_focused()
    );
}

#[test]
fn an_arrow_immediately_after_focus_keeps_the_table_focused() {
    let text = long_log(LONG_LOG_ENTRIES);
    let mut harness = harness_of(Vec::new(), &[("source.log", text.as_str())]);
    let scroll = TableScroll::of(&harness);
    harness
        .state_mut()
        .viewer
        .active_state_mut()
        .table_focus_requested = true;
    harness.step();
    press_scroll_key(&mut harness, egui::Key::ArrowDown);
    assert_table_focused(&harness);
    assert!(
        (scroll.offset_px(&harness) - scroll.row_height_px).abs() < SCROLL_READING_TOLERANCE_PX
    );
    assert_eq!(harness.state().map_center, None);
}

#[test]
fn home_and_end_reach_the_complete_displayed_plan_and_repeated_arrows_count_once_each() {
    let mut harness = keyboard_scroll_harness();
    let scroll = TableScroll::of(&harness);
    press_scroll_key(&mut harness, egui::Key::End);
    let end = scroll.offset_px(&harness);
    assert!(end > scroll.row_height_px);
    harness.get_by_label(long_log_entry_timestamp(LONG_LOG_ENTRIES - 1).as_str());
    press_scroll_key(&mut harness, egui::Key::Home);
    assert!(scroll.offset_px(&harness).abs() < SCROLL_READING_TOLERANCE_PX);
    for repeat in [false, true] {
        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run_steps(2);
    assert!(
        (scroll.offset_px(&harness) - 2.0 * scroll.row_height_px).abs()
            < SCROLL_READING_TOLERANCE_PX
    );
    assert_table_focused(&harness);
    press_scroll_key(&mut harness, egui::Key::End);
    assert!((scroll.offset_px(&harness) - end).abs() < SCROLL_READING_TOLERANCE_PX);
    press_scroll_key(&mut harness, egui::Key::PageUp);
    let page = end - scroll.offset_px(&harness);
    let visible_height = table_row_count(&harness) as f32 * scroll.row_height_px - end;
    assert!((page - visible_height).abs() < SCROLL_READING_TOLERANCE_PX);
}

#[test]
fn home_and_end_include_filtered_entries_and_temporary_raw_source() {
    let text = format!("{}raw final source\n", long_log(LONG_LOG_ENTRIES));
    let mut harness = harness_of(Vec::new(), &[("source.log", text.as_str())]);
    add_filter(&mut harness, "entry 1");
    let id = harness.state().viewer.selected_log().unwrap();
    let raw_line = u32::try_from(text.lines().count()).unwrap();
    {
        let state = harness.state_mut();
        let log = state.logs.get_by_id(id).unwrap();
        state.viewer.navigate_to_source(
            log,
            id,
            line_table::SourceRequest::NavigateToLine(raw_line),
        );
    }
    harness.run_steps(3);
    focus_table(&mut harness);
    let before = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    press_scroll_key(&mut harness, egui::Key::Home);
    let first = harness
        .state()
        .viewer
        .active_state()
        .table_reading_anchor
        .unwrap();
    assert!(first.source_line < raw_line);
    press_scroll_key(&mut harness, egui::Key::End);
    harness.get_by_label("raw final source");
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        before
    );
    assert_table_focused(&harness);
}

#[derive(Clone, Copy, Debug)]
enum LocalEditor {
    EditCondition,
    Find,
    InspectDates,
    Jump,
    ReplaceCondition,
}

#[rstest]
fn closing_local_controls_restores_table_focus_and_reading(
    #[values(
        LocalEditor::EditCondition,
        LocalEditor::Find,
        LocalEditor::InspectDates,
        LocalEditor::Jump,
        LocalEditor::ReplaceCondition
    )]
    editor: LocalEditor,
) {
    let mut harness = keyboard_scroll_harness();
    if matches!(editor, LocalEditor::ReplaceCondition) {
        add_filter(&mut harness, "entry");
        focus_table(&mut harness);
    }
    press_scroll_key(&mut harness, egui::Key::PageDown);
    let anchor = harness
        .state()
        .viewer
        .active_state()
        .table_reading_anchor
        .unwrap();
    let close = match editor {
        LocalEditor::ReplaceCondition => {
            click_chip_effect_action(
                &mut harness,
                0,
                FilterEffect::Table,
                filters::ChipControl::Edit,
            );
            "Cancel"
        }
        LocalEditor::EditCondition => {
            open_live_editor(&mut harness);
            "Cancel"
        }
        LocalEditor::Find => {
            harness.get_by_label(find::FIND_LABEL).click();
            find::CLOSE_LABEL
        }
        LocalEditor::InspectDates => {
            harness.get_by_label("Dates").click();
            "Dates"
        }
        LocalEditor::Jump => {
            harness.get_by_label("Go to…").click();
            harness.run_steps(2);
            harness.get_by_label("Go to line…").click();
            "Close source jump"
        }
    };
    harness.run_steps(3);
    harness.get_by_label(close).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_label(close).click();
    harness.run_steps(4);
    assert_table_focused(&harness);
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .table_reading_anchor
            .unwrap()
            .source_line,
        anchor.source_line
    );
    assert_eq!(harness.state().map_center, None);
}

#[rstest]
fn editor_navigation_keys_preserve_the_table_reading_location(
    #[values(
        LocalEditor::EditCondition,
        LocalEditor::Find,
        LocalEditor::InspectDates,
        LocalEditor::Jump,
        LocalEditor::ReplaceCondition
    )]
    editor: LocalEditor,
) {
    let text: String = (0..LONG_LOG_ENTRIES)
        .map(|index| format!("May 29 18:48:25 host navsyncd: entry {index}\n"))
        .collect();
    let mut harness = harness_of(Vec::new(), &[("yearless.log", text.as_str())]);
    focus_table(&mut harness);
    let field = match editor {
        LocalEditor::ReplaceCondition => {
            add_filter(&mut harness, "entry");
            click_chip_effect_action(
                &mut harness,
                0,
                FilterEffect::Table,
                filters::ChipControl::Edit,
            );
            By::new().predicate(|node| node.author_id() == Some(filters::CONDITION_EDIT_FIELD_ID))
        }
        LocalEditor::EditCondition => {
            open_live_editor(&mut harness);
            By::new().role(Role::TextInput)
        }
        LocalEditor::Find => {
            harness.get_by_label(find::FIND_LABEL).click();
            By::new().role(Role::TextInput)
        }
        LocalEditor::InspectDates => {
            open_date_inspector(&mut harness);
            select_date_policy(&mut harness, unresolved_years::SET_YEAR_LABEL);
            By::new()
                .predicate(|node| node.author_id() == Some(unresolved_years::LAST_ENTRY_YEAR_ID))
        }
        LocalEditor::Jump => {
            harness.get_by_label("Go to…").click();
            harness.run_steps(2);
            harness.get_by_label("Go to line…").click();
            By::new().predicate(|node| node.author_id() == Some(jump::JUMP_FIELD_ID))
        }
    };
    harness.run_steps(3);
    harness.get(field.clone()).scroll_to_me();
    harness.run_steps(2);
    harness.get(field).click();
    harness.run_steps(2);
    let before = harness.state().viewer.active_state().table_reading_anchor;
    for key in [
        egui::Key::ArrowDown,
        egui::Key::PageDown,
        egui::Key::End,
        egui::Key::Home,
    ] {
        press_scroll_key(&mut harness, key);
        assert_eq!(
            harness.state().viewer.active_state().table_reading_anchor,
            before
        );
    }
}

#[test]
fn hovering_the_table_without_focus_leaves_navigation_keys_available() {
    let mut harness = keyboard_scroll_harness();
    let scroll = TableScroll::of(&harness);
    harness.ctx.memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
            memory.surrender_focus(focused);
        }
    });
    let table = harness
        .get_by_label(keyboard_navigation::TABLE_LABEL)
        .rect();
    harness.hover_at(table.center());
    harness.run_steps(2);
    press_scroll_key(&mut harness, egui::Key::PageDown);
    assert!(scroll.offset_px(&harness).abs() < SCROLL_READING_TOLERANCE_PX);
}

#[test]
fn an_open_menu_blocks_table_navigation_with_a_retained_table_focus() {
    let mut harness = keyboard_scroll_harness();
    let scroll = TableScroll::of(&harness);
    harness.get_by_label("Go to…").click();
    harness.run_steps(2);
    let id = harness.state().viewer.selected_log().unwrap();
    harness.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::unique((keyboard_navigation::TABLE_FOCUS_ID, id)))
    });
    harness.run_steps(2);
    for key in [egui::Key::ArrowDown, egui::Key::PageDown, egui::Key::End] {
        press_scroll_key(&mut harness, key);
        assert!(scroll.offset_px(&harness).abs() < SCROLL_READING_TOLERANCE_PX);
    }
}

#[test]
fn association_input_navigation_preserves_table_position() {
    let text = long_log(LONG_LOG_ENTRIES);
    let mut harness = harness_of(
        vec![recording("walk.gtd", 55.0)],
        &[("source.log", text.as_str())],
    );
    focus_table(&mut harness);
    harness.get(By::new().role(Role::SpinButton)).click();
    harness.run_steps(2);
    let before = harness.state().viewer.active_state().table_reading_anchor;
    for key in [egui::Key::ArrowDown, egui::Key::PageDown, egui::Key::End] {
        press_scroll_key(&mut harness, key);
        assert_eq!(
            harness.state().viewer.active_state().table_reading_anchor,
            before
        );
    }
}

#[test]
fn the_focused_table_has_a_visible_outline() {
    let mut rendering = rendering_harness_with(Vec::new());
    focus_table(&mut rendering.inner);
    let table = rendering
        .inner
        .get_by_label(keyboard_navigation::TABLE_LABEL)
        .rect();
    let stroke = rendering.inner.ctx.global_style().visuals.selection.stroke;
    assert!(
        rendering
            .inner
            .output()
            .shapes
            .iter()
            .any(|shape| match &shape.shape {
                egui::Shape::Rect(rect) => rect.stroke == stroke && table.contains_rect(rect.rect),
                _ => false,
            })
    );
    rendering.snapshot_with_color_tolerance("log_table_keyboard_focus");
}

const MAXIMUM_VIEWER_TAB_STEPS: usize = 40;
