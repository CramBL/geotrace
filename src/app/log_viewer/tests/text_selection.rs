use egui::epaint::TextShape;
use egui::os::OperatingSystem;
use egui::text::{CCursor, CharIndex};
use egui::{
    Event, Key, Layout, Modifiers, OutputCommand, PointerButton, Pos2, Shape, TouchDeviceId,
    TouchId, TouchPhase,
};

use super::table_filter_controls::SourceReadingPosition;
use super::*;
use crate::app::log_viewer::{find, keyboard_navigation};

#[rstest]
#[case::at_bottom(false)]
#[case::batched_below_bottom(true)]
fn releasing_at_the_viewport_end_excludes_clipped_rows(#[case] batched: bool) {
    let text = long_log(12);
    let mut harness = short_table_harness(&text);
    let viewport = SourceReadingPosition::of(&harness).viewport;
    let first = harness.get_by_label("navsyncd: entry 0").rect();
    let second = harness.get_by_label("navsyncd: entry 1").rect();
    let opening = harness.get_by_label("Boot 1 · up 11s · 12 entries").rect();
    let last = harness.get_by_label("navsyncd: entry 2").rect();
    assert_eq!(
        viewport.height(),
        4.0 * (second.center().y - first.center().y)
    );
    let start = opening.left_center();
    let mut end = Pos2::new(last.right(), viewport.bottom());
    if batched {
        end.y += last.height();
        drag_in_one_frame(&mut harness, start, end);
    } else {
        harness.press_drag_release(start, end - start, 4);
    }
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    let text = harness.copied_text().unwrap();
    assert_eq!(text.lines().count(), 4);
    assert!(text.ends_with("navsyncd: entry 2"));
}

fn short_table_harness(text: &str) -> Harness<'static, ViewerState> {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_size(VIEWER_SIZE)
        .build_ui_state(
            |ui, state: &mut ViewerState| {
                let id = state.first_loaded_log();
                let log = state.logs.get_by_id(id).unwrap();
                state.viewer.keyboard_access = super::super::KeyboardAccess::Permit;
                let size = egui::vec2(ui.available_width(), 4.0 * line_table::row_height(ui));
                ui.allocate_ui_with_layout(size, Layout::top_down(egui::Align::Min), |ui| {
                    state.viewer.line_table_ui(
                        ui,
                        log,
                        id,
                        &mut line_table::LineTableRequests {
                            map_center: &mut state.map_center,
                            hover: &mut state.log_hover,
                        },
                    );
                });
            },
            viewer_state(Vec::new(), &[("boundaries.log", text)]),
        );
    harness.run_steps(5);
    harness
}

fn drag_in_one_frame(harness: &mut Harness<'_, ViewerState>, start: Pos2, end: Pos2) {
    harness.hover_at(start);
    harness.step();
    harness.input_mut().events.extend([
        Event::PointerButton {
            pos: start,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerMoved(end),
        Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    harness.step();
}

#[rstest]
#[case::released_in_the_same_frame(true)]
#[case::held_across_frames(false)]
fn a_scrollbar_drag_does_not_select_table_text(#[case] batched: bool) {
    let mut harness = short_table_harness(&long_log(36));
    harness.ctx.set_os(OperatingSystem::Nix);
    let scrollbar = harness.get(By::new().role(Role::ScrollBar)).rect();
    let start = Pos2::new(scrollbar.center().x, scrollbar.top() + 10.0);
    let end = start + egui::vec2(0.0, 40.0);
    if batched {
        drag_in_one_frame(&mut harness, start, end);
    } else {
        harness.press_drag_release(start, end - start, 4);
        let anchor = harness
            .state()
            .viewer
            .active_state()
            .table_reading_anchor
            .unwrap();
        assert!(anchor.source_line > 1);
    }
    assert!(settled_selection(&harness).is_none());
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert!(harness.copied_text().is_none());
}

#[test]
fn a_scrollbar_press_overrides_the_release_position_for_table_focus() {
    let mut harness = harness_of(Vec::new(), &[("scrollbar.log", &long_log(120))]);
    let label = "navsyncd: entry 0";
    let message = harness.get_by_label(label).rect();
    harness.press_drag_release(message.left_center(), egui::vec2(message.width(), 0.0), 4);
    table_focus::assert_table_focused(&harness);
    harness.key_press(Key::Tab);
    harness.run_steps(2);
    assert!(
        !harness
            .get_by_label(keyboard_navigation::TABLE_LABEL)
            .accesskit_node()
            .is_focused()
    );
    assert!(selection_painted(&harness, label));

    let scrollbar = harness.get(By::new().role(Role::ScrollBar)).rect();
    let start = egui::pos2(scrollbar.center().x, scrollbar.top() + 10.0);
    drag_in_one_frame(&mut harness, start, message.center());
    assert!(
        !harness
            .get_by_label(keyboard_navigation::TABLE_LABEL)
            .accesskit_node()
            .is_focused()
    );
    assert!(selection_painted(&harness, label));
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert!(harness.copied_text().is_none());
}

#[rstest]
#[case::before_the_ellipsis(false, 0.75)]
#[case::left_half_of_the_ellipsis(true, 0.25)]
#[case::right_half_of_the_ellipsis(true, 0.75)]
fn double_clicking_elided_text_selects_the_glyph_under_the_pointer(
    #[case] on_marker: bool,
    #[case] fraction: f32,
) {
    let message = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ".repeat(4);
    let text = format!("2026-05-29 18:48:25 {message}\n");
    let mut harness = harness_of(Vec::new(), &[("elided.log", &text)]);
    harness.ctx.set_os(OperatingSystem::Nix);
    let shape = harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| {
            let Shape::Text(shape) = &shape.shape else {
                return None;
            };
            (shape.galley.text() == message && shape.galley.elided).then_some(shape)
        })
        .unwrap();
    let marker = shape.galley.end().index - 1;
    let glyph = if on_marker { marker } else { marker - 1 };
    let pointer = glyph_point(shape, glyph, fraction);
    let start = if on_marker { marker.0 } else { 0 };
    let expected: String = message.chars().skip(start).collect();
    harness.double_click_at(pointer);
    assert!(selection_painted(&harness, &message));
    assert_eq!(settled_selection(&harness), Some(expected.as_str()));
    assert!(harness.copied_text().is_none());
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(expected.as_str()));
}

#[rstest]
#[case::left_half(0.25)]
#[case::right_half(0.75)]
fn a_unicode_word_after_punctuation_is_selected_from_either_half(#[case] fraction: f32) {
    let message = "svc: café.βeta gamma";
    let log = format!("2026-05-29 18:48:25 {message}\n");
    let mut harness = harness_of(Vec::new(), &[("words.log", &log)]);
    let shape = harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == message => Some(text),
            _ => None,
        })
        .unwrap();
    let beta = CharIndex(
        message
            .chars()
            .position(|character| character == 'β')
            .unwrap(),
    );
    let pointer = glyph_point(shape, beta, fraction);
    harness.double_click_at(pointer);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("βeta"));
}

fn glyph_point(shape: &TextShape, glyph: CharIndex, fraction: f32) -> Pos2 {
    let left = shape.galley.pos_from_cursor(CCursor::new(glyph));
    let right = shape.galley.pos_from_cursor(CCursor::new(glyph + 1));
    shape.pos
        + egui::vec2(
            egui::lerp(left.left()..=right.left(), fraction),
            left.center().y,
        )
}

#[test]
fn selecting_an_inspector_value_supersedes_the_table_selection() {
    let mut harness = harness_with(Vec::new());
    unfold_the_summary_panel(&mut harness);
    let message = harness
        .topmost_matching(By::new().label(FIRST_ENTRY_MESSAGE))
        .rect();
    harness.press_drag_release(message.left_center(), egui::vec2(message.width(), 0.0), 4);
    assert!(selection_painted(&harness, FIRST_ENTRY_MESSAGE));
    let format = harness.get_by_label(FIXTURE_LOG_FORMAT).rect();
    harness.press_drag_release(format.left_center(), egui::vec2(format.width(), 0.0), 4);
    assert!(!selection_painted(&harness, FIRST_ENTRY_MESSAGE));
    table_focus::focus_table(&mut harness);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(FIXTURE_LOG_FORMAT));
}

fn inspector_selection_harness() -> Harness<'static, ViewerState> {
    let mut harness = word_selection_harness();
    unfold_the_summary_panel(&mut harness);
    let message = harness.get_by_label("svc: alpha beta gamma").rect();
    harness.press_drag_release(message.left_center(), egui::vec2(message.width(), 0.0), 4);
    let format = harness.get_by_label(FIXTURE_LOG_FORMAT).rect();
    harness.press_drag_release(format.left_center(), egui::vec2(format.width(), 0.0), 4);
    table_focus::assert_table_focused(&harness);
    assert!(!selection_painted(&harness, "svc: alpha beta gamma"));
    assert!(selection_painted(&harness, FIXTURE_LOG_FORMAT));

    harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    harness.run_steps(2);
    assert!(
        !harness
            .get_by_label(keyboard_navigation::TABLE_LABEL)
            .accesskit_node()
            .is_focused()
    );
    harness
}

#[test]
fn select_all_copies_the_table_after_tabbing_from_the_inspector() {
    let mut harness = inspector_selection_harness();
    harness.key_press(Key::Tab);
    harness.run_steps(2);
    table_focus::assert_table_focused(&harness);
    assert!(!selection_painted(&harness, FIXTURE_LOG_FORMAT));
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(WORD_DOCUMENT));
}

#[rstest]
#[case::inspector_copy_before_tab(false, &[FIXTURE_LOG_FORMAT])]
#[case::copies_before_and_after_tab(true, &[FIXTURE_LOG_FORMAT, WORD_DOCUMENT])]
fn batched_copy_follows_focus_from_inspector_to_table(
    #[case] copy_table: bool,
    #[case] expected: &[&str],
) {
    let mut harness = inspector_selection_harness();
    harness
        .input_mut()
        .events
        .extend([Event::Copy, key_press_event(Key::Tab, Modifiers::NONE)]);
    if copy_table {
        harness
            .input_mut()
            .events
            .extend([key_press_event(Key::A, Modifiers::COMMAND), Event::Copy]);
    }
    harness.step();
    let copies: Vec<_> = harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| {
            if let OutputCommand::CopyText(text) = command {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(copies, expected);
    harness.step();
    table_focus::assert_table_focused(&harness);
    assert!(!selection_painted(&harness, FIXTURE_LOG_FORMAT));
}

#[test]
fn keyboard_focus_moves_copy_from_the_table_to_find() {
    let mut harness = harness_with(Vec::new());
    let message = harness
        .topmost_matching(By::new().label(FIRST_ENTRY_MESSAGE))
        .rect();
    harness.press_drag_release(message.left_center(), egui::vec2(message.width(), 0.0), 4);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(Event::Text("navsyncd".to_owned()));
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    let field = harness.get(By::new().label(find::FIND_LABEL).role(Role::TextInput));
    assert!(field.accesskit_node().is_focused());
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("navsyncd"));
}

#[rstest]
#[case::move_before_release(true)]
#[case::move_on_release(false)]
fn a_short_drag_reports_primary_on_release(#[case] move_before_release: bool) {
    let mut harness = harness_of(
        vec![recording("walk.gtd", 55.0)],
        &[("short-drag.log", "2026-05-29 18:48:25 svc: abcd\n")],
    );
    harness.ctx.set_os(OperatingSystem::Nix);
    let message = harness.get_by_label("svc: abcd").rect();
    let midpoint = message.left() + message.width() * (5.5 / 9.0);
    let from = Pos2::new(midpoint - 2.0, message.center().y);
    let to = Pos2::new(midpoint + 2.0, message.center().y);
    harness.hover_at(from);
    harness.step();
    harness.drag_at(from);
    harness.step();
    if move_before_release {
        harness.hover_at(to);
        harness.step();
        assert!(selection_painted(&harness, "svc: abcd"));
    }
    assert!(settled_selection(&harness).is_none() && harness.copied_text().is_none());

    harness.input_mut().events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    assert_eq!(settled_selection(&harness), Some("a"));
    assert!(harness.copied_text().is_none());
    assert!(harness.state().map_center.is_none());
    harness.step();
    assert!(settled_selection(&harness).is_none());
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("a"));
}

#[rstest]
#[case::separate_select_all(false, Key::A, Modifiers::COMMAND, WORD_DOCUMENT)]
#[case::separate_shift_right(false, Key::ArrowRight, Modifiers::SHIFT, "s")]
#[case::batched_select_all(true, Key::A, Modifiers::COMMAND, WORD_DOCUMENT)]
#[case::batched_shift_right(true, Key::ArrowRight, Modifiers::SHIFT, "s")]
fn a_row_click_centres_the_map_and_preserves_caret_keyboard_focus(
    #[case] batched: bool,
    #[case] key: Key,
    #[case] modifiers: Modifiers,
    #[case] expected: &str,
) {
    let mut harness = harness_of(
        vec![recording("walk.gtd", 55.0)],
        &[("words.log", WORD_LOG)],
    );
    harness.ctx.set_os(OperatingSystem::Nix);
    let (message, _) = label_shape(&harness, "svc: alpha beta gamma").unwrap();
    let pos = glyph_point(message, CharIndex::ZERO, 0.25);
    harness.hover_at(pos);
    harness.step();
    for pressed in [true, false] {
        harness.input_mut().events.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        if pressed && !batched {
            harness.step();
            table_focus::assert_table_focused(&harness);
            assert!(harness.state().map_center.is_none());
        }
    }
    harness.step();
    table_focus::assert_table_focused(&harness);
    assert!(harness.state().map_center.is_some());
    assert!(settled_selection(&harness).is_none());
    harness
        .input_mut()
        .events
        .extend([key_press_event(key, modifiers), Event::Copy]);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(expected));
}

#[test]
fn a_word_selection_can_be_cut_and_extended_by_shift_click() {
    let mut harness = word_selection_harness();
    let message = harness.get_by_label("svc: alpha beta gamma").rect();
    let beta = Pos2::new(
        message.left() + message.width() * (13.0 / 21.0),
        message.center().y,
    );
    harness.double_click_at(beta);
    harness.input_mut().events.push(Event::Cut);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("beta"));
    harness.get_by_label("svc: alpha beta gamma");

    let end = message.right_center();
    harness.hover_at(end);
    harness.step();
    for pressed in [true, false] {
        harness.input_mut().events.push(Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::SHIFT,
        });
    }
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("beta gamma"));
}

#[rstest]
#[case::left_from_a_forward_drag(false, Key::ArrowLeft, Modifiers::SHIFT, "bet")]
#[case::right_from_a_reverse_drag(true, Key::ArrowRight, Modifiers::SHIFT, "eta")]
#[case::next_word(false, Key::ArrowRight, Modifiers::SHIFT | Modifiers::CTRL, "beta gamma")]
fn shift_arrow_changes_the_drag_endpoint(
    #[case] reverse: bool,
    #[case] key: Key,
    #[case] modifiers: Modifiers,
    #[case] expected: &str,
) {
    let mut harness = word_selection_harness();
    harness.ctx.set_os(OperatingSystem::Nix);
    let message = harness.get_by_label("svc: alpha beta gamma").rect();
    let start = egui::pos2(
        message.left() + message.width() * (11.0 / 21.0),
        message.center().y,
    );
    let end = egui::pos2(
        message.left() + message.width() * (15.0 / 21.0),
        message.center().y,
    );
    let (start, end) = if reverse { (end, start) } else { (start, end) };
    drag_in_one_frame(&mut harness, start, end);
    assert_eq!(settled_selection(&harness), Some("beta"));
    harness
        .input_mut()
        .events
        .push(key_press_event(key, modifiers));
    harness.step();
    assert_eq!(settled_selection(&harness), Some(expected));
    assert!(harness.copied_text().is_none());
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(expected));
}

#[test]
fn shift_arrow_crosses_row_and_field_boundaries_then_reverses_past_the_anchor() {
    let mut harness = word_selection_harness();
    let end = harness
        .get_by_label("svc: alpha beta gamma")
        .rect()
        .right_center();
    harness.click_at(end);
    harness.step();
    table_focus::assert_table_focused(&harness);
    for (key, presses, expected) in [(Key::ArrowRight, 3, "\n2 "), (Key::ArrowLeft, 4, "a")] {
        harness
            .input_mut()
            .events
            .extend((0..presses).map(|_| key_press_event(key, Modifiers::SHIFT)));
        harness.input_mut().events.push(Event::Copy);
        harness.step();
        assert_eq!(harness.copied_text().as_deref(), Some(expected));
    }
}

#[test]
fn shift_down_scrolls_to_the_next_matching_row() {
    let mut harness = short_table_harness(&long_log(12));
    let id = harness.state().first_loaded_log();
    let (filters, _) = harness.state_mut().logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_regex(true);
    filters.set_live_filter_text(r"\bentry (?:[012456789]|1[01])$");
    filters.add_live_filter_as_chip().unwrap();
    filters.wait_for_queries();
    harness.run_steps(3);
    let message = harness.get_by_label("navsyncd: entry 2").rect();
    harness.press_drag_release(message.left_center(), egui::vec2(message.width(), 0.0), 4);
    harness
        .input_mut()
        .events
        .push(key_press_event(Key::ArrowDown, Modifiers::SHIFT));
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    let next_timestamp = long_log_entry_timestamp(4);
    let expected = format!(
        "navsyncd: entry 2\n5  {} navsyncd: entry 4",
        next_timestamp.trim_start()
    );
    assert_eq!(harness.copied_text().as_deref(), Some(expected.as_str()));
    harness.run_steps(2);
    let next = harness.get_by_label("navsyncd: entry 4").rect();
    assert!(
        SourceReadingPosition::of(&harness)
            .viewport
            .contains_rect(next)
    );
}

#[test]
fn vertical_selection_keeps_its_rendered_column_across_filtered_offscreen_rows() {
    let log: String = (0..14)
        .map(|index| {
            let message = match index + 1 {
                9 => "abcdef nine",
                10 => "abcdef ten",
                11 => "x",
                13 => "abcdef thirteen",
                14 => "abcdef fourteen",
                _ => "omit this line",
            };
            format!(
                "{} {message}\n",
                long_log_entry_timestamp(index).trim_start()
            )
        })
        .collect();
    let mut harness = short_table_harness(&log);
    let id = harness.state().first_loaded_log();
    let (filters, _) = harness.state_mut().logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_regex(true);
    filters.set_live_filter_text("^(?:abcdef|x$)");
    filters.add_live_filter_as_chip().unwrap();
    filters.wait_for_queries();
    harness.run_steps(3);

    let (first, _) = label_shape(&harness, "abcdef nine").unwrap();
    let (second, _) = label_shape(&harness, "abcdef ten").unwrap();
    let from = glyph_point(first, CharIndex::ZERO, 0.0);
    let to = glyph_point(first, CharIndex(3), 0.0);
    assert_eq!(to.x, glyph_point(second, CharIndex(3), 0.0).x);
    assert!(harness.query_by_label("abcdef fourteen").is_none());
    drag_in_one_frame(&mut harness, from, to);
    let through_second = "abcdef nine\n10  2026-05-29 18:48:34 abc";
    let through_last = "abcdef nine\n10  2026-05-29 18:48:34 abcdef ten\n11  2026-05-29 18:48:35 x\n13  2026-05-29 18:48:37 abcdef thirteen\n14  2026-05-29 18:48:38 abc";

    harness.input_mut().events.extend([
        key_press_event(Key::ArrowDown, Modifiers::SHIFT),
        Event::Copy,
    ]);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(through_second));

    harness.set_size(egui::vec2(VIEWER_SIZE.x - 180.0, VIEWER_SIZE.y));
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .extend((0..3).map(|_| key_press_event(Key::ArrowDown, Modifiers::SHIFT)));
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(through_last));
    assert!(harness.query_by_label("abcdef fourteen").is_none());
    assert!(harness.state().map_center.is_none());

    harness.run_steps(2);
    let (last, viewport) = label_shape(&harness, "abcdef fourteen").unwrap();
    assert!(viewport.contains_rect(last.visual_bounding_rect()));
    let reverse_to_middle =
        "abcdef nine\n10  2026-05-29 18:48:34 abcdef ten\n11  2026-05-29 18:48:35 x";
    for (steps, expected) in [(2, reverse_to_middle), (2, "abc")] {
        harness
            .input_mut()
            .events
            .extend((0..steps).map(|_| key_press_event(Key::ArrowUp, Modifiers::SHIFT)));
        harness.input_mut().events.push(Event::Copy);
        harness.step();
        assert_eq!(harness.copied_text().as_deref(), Some(expected));
    }
}

#[rstest]
#[case::start([Key::ArrowUp, Key::ArrowUp, Key::ArrowDown], "1  2026-05-29 18:48:25 ")]
#[case::end([Key::ArrowDown, Key::ArrowDown, Key::ArrowUp], "abcdef")]
fn vertical_selection_reverses_from_the_document_boundary(
    #[case] keys: [Key; 3],
    #[case] expected: &str,
) {
    let mut harness = harness_of(
        Vec::new(),
        &[(
            "boundary.log",
            "2026-05-29 18:48:25 abcdef\n2026-05-29 18:48:26 abcdef\n",
        )],
    );
    let mut messages = harness
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == "abcdef" => Some(text),
            _ => None,
        });
    let first = messages.next().unwrap();
    let second = messages.next().unwrap();
    let start = glyph_point(first, CharIndex::ZERO, 0.0);
    let end = glyph_point(first, CharIndex(3), 0.0);
    assert_eq!(end.x, glyph_point(second, CharIndex(3), 0.0).x);
    drag_in_one_frame(&mut harness, start, end);

    for key in keys {
        harness
            .input_mut()
            .events
            .push(key_press_event(key, Modifiers::SHIFT));
        harness.step();
    }
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(expected));
}

fn label_shape<'a>(
    harness: &'a Harness<ViewerState>,
    label: &str,
) -> Option<(&'a TextShape, egui::Rect)> {
    harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            Shape::Text(text) if text.galley.text() == label => Some((text, shape.clip_rect)),
            _ => None,
        })
}

#[test]
fn triple_click_selects_a_row_and_select_all_includes_other_displayed_rows() {
    let mut harness = word_selection_harness();
    let pos = harness
        .get_by_label("svc: alpha beta gamma")
        .rect()
        .center();
    harness.hover_at(pos);
    harness.step();
    for pressed in [true, false, true, false, true, false] {
        harness.input_mut().events.push(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(
        harness.copied_text().as_deref(),
        Some("1  2026-05-29 18:48:25 svc: alpha beta gamma"),
    );

    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(WORD_DOCUMENT));
}

#[test]
fn copy_before_select_all_uses_the_previous_range() {
    let mut harness = word_selection_harness();
    harness.ctx.set_os(OperatingSystem::Nix);
    let message = harness.get_by_label("svc: alpha beta gamma").rect();
    let beta = Pos2::new(
        message.left() + message.width() * (13.0 / 21.0),
        message.center().y,
    );
    harness.double_click_at(beta);
    harness.input_mut().events.extend([
        Event::Copy,
        Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        },
    ]);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("beta"));
    assert_eq!(settled_selection(&harness), Some(WORD_DOCUMENT));
}

#[test]
fn copy_between_batched_clicks_copies_the_word_and_allows_the_row_click() {
    let mut harness = harness_of(
        vec![recording("walk.gtd", 55.0)],
        &[("words.log", WORD_LOG)],
    );
    let first = harness.get_by_label("svc: alpha beta gamma").rect();
    let second = harness.get_by_label("svc: another row").rect();
    let beta = Pos2::new(
        first.left() + first.width() * (13.0 / 21.0),
        first.center().y,
    );
    let point = second.center();
    harness.ctx.set_os(OperatingSystem::Nix);
    harness.hover_at(point);
    harness.step();
    for (index, pos) in [beta, beta, point].into_iter().enumerate() {
        if index == 2 {
            harness.input_mut().events.push(Event::Copy);
        }
        for pressed in [true, false] {
            harness.input_mut().events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
    }
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("beta"));
    assert!(settled_selection(&harness).is_none());
    assert!(harness.state().map_center.is_some());
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert!(harness.copied_text().is_none());
}

#[rstest]
#[case::before_tab(Modifiers::NONE, true)]
#[case::after_tab(Modifiers::NONE, false)]
#[case::before_shift_tab(Modifiers::SHIFT, true)]
#[case::after_shift_tab(Modifiers::SHIFT, false)]
fn a_later_row_click_keeps_the_copy_ownership_at_tab(
    #[case] navigation: Modifiers,
    #[case] copy_first: bool,
) {
    let mut harness = word_selection_harness();
    let first = harness.get_by_label("svc: alpha beta gamma").rect();
    let beta = Pos2::new(
        first.left() + first.width() * (13.0 / 21.0),
        first.center().y,
    );
    harness.double_click_at(beta);
    table_focus::assert_table_focused(&harness);
    let point = harness.get_by_label("svc: another row").rect().center();
    harness.hover_at(point);
    harness.step();
    let tab = key_press_event(Key::Tab, navigation);
    let commands = if copy_first {
        [Event::Copy, tab]
    } else {
        [tab, Event::Copy]
    };
    harness.input_mut().events.extend(commands);
    for pressed in [true, false] {
        harness.input_mut().events.push(Event::PointerButton {
            pos: point,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.step();
    assert_eq!(
        harness.copied_text().as_deref(),
        copy_first.then_some("beta")
    );
}

#[test]
fn tab_into_the_table_restores_copy_for_its_retained_selection() {
    let mut harness = word_selection_harness();
    let first = harness.get_by_label("svc: alpha beta gamma").rect();
    let beta = Pos2::new(
        first.left() + first.width() * (13.0 / 21.0),
        first.center().y,
    );
    harness.double_click_at(beta);
    table_focus::assert_table_focused(&harness);
    harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
    harness.run_steps(2);
    let table = harness.get_by_label(keyboard_navigation::TABLE_LABEL);
    assert!(!table.accesskit_node().is_focused());

    harness
        .input_mut()
        .events
        .extend([key_press_event(Key::Tab, Modifiers::NONE), Event::Copy]);
    harness.step();
    table_focus::assert_table_focused(&harness);
    assert_eq!(harness.copied_text().as_deref(), Some("beta"));
}

fn key_press_event(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn dragging_on_the_second_press_extends_by_whole_words() {
    let mut harness = word_selection_harness();
    let message = harness.get_by_label("svc: alpha beta gamma").rect();
    let beta = Pos2::new(
        message.left() + message.width() * (13.0 / 21.0),
        message.center().y,
    );
    let gamma = Pos2::new(
        message.left() + message.width() * (18.5 / 21.0),
        message.center().y,
    );
    harness.click_at(beta);
    harness.press_drag_release(beta, gamma - beta, 4);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("beta gamma"));
}

fn word_selection_harness() -> Harness<'static, ViewerState> {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_size(VIEWER_SIZE)
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            viewer_ui,
            viewer_state(Vec::new(), &[("words.log", WORD_LOG)]),
        );
    harness.run_steps(3);
    harness
}

const WORD_LOG: &str =
    "2026-05-29 18:48:25 svc: alpha beta gamma\n2026-05-29 18:48:26 svc: another row\n";
const WORD_DOCUMENT: &str = "Boot 1 · up 1s · 2 entries\n1  2026-05-29 18:48:25 svc: alpha beta gamma\n2  2026-05-29 18:48:26 svc: another row";

fn settled_selection<'a>(harness: &'a Harness<'_, ViewerState>) -> Option<&'a str> {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| {
            if let OutputCommand::TextSelectionSettled(text) = command {
                Some(text.as_str())
            } else {
                None
            }
        })
}

fn selection_painted(harness: &Harness<ViewerState>, label: &str) -> bool {
    let fill = harness.ctx.global_style().visuals.selection.bg_fill;
    harness.output().shapes.iter().any(|shape| {
        let Shape::Text(text) = &shape.shape else {
            return false;
        };
        text.galley.text() == label
            && text.galley.rows.iter().any(|row| {
                row.visuals
                    .mesh
                    .vertices
                    .iter()
                    .any(|vertex| vertex.color == fill)
            })
    })
}

#[rstest]
#[case::forward(false)]
#[case::backward(true)]
fn dragging_across_filtered_services_copies_partial_endpoints(#[case] backward: bool) {
    let mut harness = harness_of(Vec::new(), &[("service.log", SERVICE_LOG)]);
    let id = harness.state().first_loaded_log();
    let (filters, _) = harness.state_mut().logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_scope(FilterScope::Service);
    for service in ["alpha", "beta"] {
        filters.set_live_filter_text(service);
        filters.add_live_filter_as_chip().unwrap();
    }
    filters.set_group_operator(filters.selected_group(), FilterGroupOperator::Any);
    filters.wait_for_queries();
    harness.run_steps(3);
    assert_eq!(match_count(&harness), "3 of 5");

    let first = harness.get_by_label("alpha: première étape").rect();
    let last = harness.get_by_label("alpha: dernière étape").rect();
    let start = first.left_center() + egui::vec2(first.width() * (7.0 / 21.0), 0.0);
    let end = last.left_center() + egui::vec2(last.width() * (15.0 / 21.0), 0.0);
    let (from, to) = if backward { (end, start) } else { (start, end) };
    harness.press_drag_release(from, to - from, 5);
    harness.run_steps(2);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(
        harness.copied_text().as_deref(),
        Some(
            "première étape\n3  2026-05-29 18:48:27 beta: middle\n5  2026-05-29 18:48:29 alpha: dernière"
        )
    );
    assert_eq!(match_count(&harness), "3 of 5");
}

#[test]
fn mouse_drag_selects_log_text_after_a_touch() {
    let mut harness = harness_of(Vec::new(), &[("service.log", SERVICE_LOG)]);
    let message = harness.get_by_label("beta: middle").rect();
    let touch = message.center();
    let touch_event = |phase| Event::Touch {
        device_id: TouchDeviceId(0),
        id: TouchId(0),
        phase,
        pos: touch,
        force: None,
    };
    harness.hover_at(touch);
    harness.step();
    harness.drag_at(touch);
    harness
        .input_mut()
        .events
        .push(touch_event(TouchPhase::Start));
    harness.step();
    harness.drop_at(touch);
    harness
        .input_mut()
        .events
        .push(touch_event(TouchPhase::End));
    harness.step();
    assert!(
        harness
            .ctx
            .input(|input| input.has_touch_screen() && !input.any_touches())
    );

    harness.press_drag_release(message.left_center(), egui::vec2(message.width(), 0.0), 5);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some("beta: middle"));
}

#[test]
fn dragging_below_the_final_filtered_row_selects_to_the_end() {
    let mut harness = harness_of(Vec::new(), &[("service.log", SERVICE_LOG)]);
    type_into_live_filter(&mut harness, "alpha");
    let viewport = harness
        .get_by_label(keyboard_navigation::TABLE_LABEL)
        .rect();
    let first = harness.get_by_label("alpha: première étape").rect();
    let last = harness.get_by_label("alpha: dernière étape").rect();
    let blank_y = (last.bottom() + viewport.bottom()) / 2.0;
    let blank_start = Pos2::new(first.left(), blank_y);
    let blank_end = Pos2::new(last.right(), blank_y);
    assert!(last.bottom() < blank_y && viewport.contains(blank_end));

    harness.press_drag_release(blank_start, blank_end - blank_start, 5);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert!(harness.copied_text().is_none());

    let from = first.left_center();
    harness.press_drag_release(from, blank_start - from, 5);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(
        harness.copied_text().as_deref(),
        Some("alpha: première étape\n5  2026-05-29 18:48:29 alpha: dernière étape")
    );
}

#[test]
fn dragging_across_revealed_source_copies_a_blank_source_line() {
    let text = "2026-05-29 18:48:25 svc: first\n\n2026-05-29 18:48:27 svc: target\n";
    let mut harness = harness_of(Vec::new(), &[("source.log", text)]);
    type_into_live_filter(&mut harness, "target");
    harness.get_by_label("svc: target").click_secondary();
    harness.run_steps(2);
    harness.get_by_label("Show surrounding lines").click();
    harness.run_steps(3);

    let first = harness.get_by_label("svc: first").rect();
    let last = harness.get_by_label("svc: target").rect();
    let from = first.center();
    let to = last.right_center();
    harness.press_drag_release(from, to - from, 5);
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(
        harness.copied_text().as_deref(),
        Some("first\n2 \n3  2026-05-29 18:48:27 svc: target")
    );
}

#[test]
fn shift_down_uses_the_revealed_source_rows_glyph_origin() {
    let mut harness = harness_of(Vec::new(), &[("source.log", SOURCE_LOG)]);
    type_into_live_filter(&mut harness, "last");
    harness.get_by_label("navsyncd: last").click_secondary();
    harness.run_steps(2);
    harness.get_by_label("Show surrounding lines").click();
    harness.run_steps(3);
    let header = "----------- Journal summary -----------";
    let raw = "trailing source text";
    let (entry, _) = label_shape(&harness, "navsyncd: last").unwrap();
    let (source, _) = label_shape(&harness, header).unwrap();
    let start = glyph_point(entry, CharIndex::ZERO, 0.0);
    let end = glyph_point(entry, CharIndex(3), 0.0);
    assert!(source.pos.x < entry.pos.x);
    let index = source
        .galley
        .cursor_from_pos(egui::vec2(
            end.x - source.pos.x,
            source.galley.rect.center().y,
        ))
        .index;
    let prefix: String = header.chars().take(index.0).collect();
    assert!(!prefix.is_empty() && prefix != header);
    let through_source = format!("navsyncd: last\n5 {prefix}");
    let through_target = format!("navsyncd: last\n5 {header}\n6 {raw}");
    drag_in_one_frame(&mut harness, start, end);
    for (key, expected) in [
        (Key::ArrowDown, through_source.as_str()),
        (Key::ArrowDown, through_target.as_str()),
        (Key::ArrowUp, through_source.as_str()),
    ] {
        harness
            .input_mut()
            .events
            .extend([key_press_event(key, Modifiers::SHIFT), Event::Copy]);
        harness.step();
        assert_eq!(harness.copied_text().as_deref(), Some(expected));
    }
}

#[test]
fn dragging_past_the_viewport_scrolls_and_copies_rows_above_it() {
    let text = long_log(LONG_LOG_ENTRIES);
    let mut harness = harness_of(Vec::new(), &[("long.log", &text)]);
    let viewport = SourceReadingPosition::of(&harness).viewport;
    let first = harness.get_by_label("navsyncd: entry 0").rect();
    let from = first.left_center();
    let below = Pos2::new(viewport.right() - 24.0, viewport.bottom() + 60.0);
    harness.hover_at(from);
    harness.step();
    harness.drag_at(from);
    harness.step();
    harness.hover_at(below);
    harness.run_steps(90);
    harness.drop_at(below);
    harness.step();

    assert!(harness.query_by_label("navsyncd: entry 0").is_none());
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    let text = harness.copied_text().unwrap();
    assert_eq!(text.lines().next(), Some("navsyncd: entry 0"));
    for index in 1..20 {
        let line_number = index + 1;
        let timestamp = long_log_entry_timestamp(index);
        assert_eq!(
            text.lines().nth(index),
            Some(format!("{line_number} {timestamp} navsyncd: entry {index}").as_str())
        );
    }
}

#[test]
fn selecting_an_elided_line_to_its_end_copies_the_full_message() {
    let message = format!("svc: {}done", "résultat ".repeat(150));
    let text = format!("2026-05-29 18:48:25 {message}\n");
    let mut harness = harness_of(Vec::new(), &[("long-line.log", &text)]);
    let message_rect = harness.get_by_label(message.as_str()).rect();
    assert!(harness.output().shapes.iter().any(|shape| {
        matches!(&shape.shape, Shape::Text(text) if text.galley.elided && text.galley.text() == message)
    }));
    harness.press_drag_release(
        message_rect.left_center(),
        egui::vec2(message_rect.width(), 0.0),
        5,
    );
    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert_eq!(harness.copied_text().as_deref(), Some(message.as_str()));
}

#[test]
fn applying_a_new_table_filter_clears_the_previous_selection() {
    let mut harness = harness_of(Vec::new(), &[("service.log", SERVICE_LOG)]);
    let from = harness
        .get_by_label("alpha: première étape")
        .rect()
        .left_center();
    let to = harness
        .get_by_label("alpha: dernière étape")
        .rect()
        .right_center();
    harness.press_drag_release(from, to - from, 5);
    table_focus::assert_table_focused(&harness);

    let id = harness.state().first_loaded_log();
    let (filters, _) = harness.state_mut().logs.filter_stack_mut_by_id(id).unwrap();
    filters.set_live_filter_text("middle");
    filters.wait_for_queries();
    harness.run_steps(3);
    assert_eq!(match_count(&harness), "1 of 5");
    table_focus::assert_table_focused(&harness);

    harness.input_mut().events.push(Event::Copy);
    harness.step();
    assert!(harness.copied_text().is_none());
}

const SERVICE_LOG: &str = "\
2026-05-29 18:48:25 alpha: première étape
2026-05-29 18:48:26 gamma: skipped one
2026-05-29 18:48:27 beta: middle
2026-05-29 18:48:28 gamma: skipped two
2026-05-29 18:48:29 alpha: dernière étape
";
