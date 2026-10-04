use std::sync::Arc;

use gt_logfile::LogLevelKind;

use super::*;
use crate::app::log_viewer::line_table::DiagnosticTarget;

#[test]
fn physical_line_numbers_align_before_timestamps_and_select_for_copying() {
    let text = format!("{}\n\n\n2026-05-29 18:48:40 navsyncd: last\n", long_log(8));
    let mut harness = harness_of(Vec::new(), &[("numbered.log", &text)]);
    let number = harness.bottommost_matching(By::new().label("1")).rect();
    let largest = harness.get_by_label("12").rect();
    let first_time = harness
        .get_by_label(long_log_entry_timestamp(0).as_str())
        .rect();
    let last_time = harness.get_by_label(" 2026-05-29 18:48:40").rect();
    assert!((number.right() - largest.right()).abs() < 0.1);
    assert!((first_time.left() - last_time.left()).abs() < 0.1);
    assert!(largest.right() < last_time.left());
    assert!(number.width() < largest.width());
    harness.press_drag_release(largest.left_center(), egui::vec2(largest.width(), 0.0), 4);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();
    assert_eq!(copied_text(&harness).trim(), "12");
}

#[rstest]
#[case::physical_line_number("1")]
#[case::timestamp("2026")]
fn message_filters_exclude_physical_numbers_and_timestamps(#[case] text: &str) {
    let mut harness = harness_of(
        Vec::new(),
        &[("messages.log", "2026-05-29 18:48:25 navsyncd: unique\n")],
    );
    type_into_live_filter(&mut harness, text);
    assert_eq!(match_count(&harness), "0 of 1");
}

#[test]
fn structural_source_rows_show_exact_text_and_boot_context_in_source_order() {
    let mut harness = harness_of(Vec::new(), &[("source.log", SOURCE_LOG)]);
    assert!(!harness.state().viewer.show_structural_lines);
    assert!(harness.query_by_label("--- Device reboot ---").is_none());
    set_display_option(&mut harness, "Show structural lines", true);
    assert!(harness.state().viewer.show_structural_lines);
    let separator = harness.get_by_label("--- Device reboot ---").rect();
    let second_boot = harness.get_by_label("Boot 2 · up 10s · 2 entries").rect();
    assert!((separator.center().y - second_boot.center().y).abs() < 0.1);
    let first = harness.get_by_label("navsyncd: first").rect();
    let second = harness.get_by_label("navsyncd: second").rect();
    let last = harness.get_by_label("navsyncd: last").rect();
    let summary = harness
        .get_by_label("----------- Journal summary -----------")
        .rect();
    let tail = harness.get_by_label("trailing source text").rect();
    assert!(first.top() < separator.top());
    assert!(separator.top() < second.top());
    assert!(last.top() < summary.top());
    assert!(summary.top() < tail.top());
    assert_eq!(
        harness
            .get_all_by_label("Boot 2 · up 10s · 2 entries")
            .count(),
        1
    );
    harness.get_by_label("Boot 1 · up 0s · 1 entry");
}

#[rstest]
#[case::message(FilterScope::Message)]
#[case::service(FilterScope::Service)]
#[case::hostname(FilterScope::Hostname)]
fn structural_source_rows_remain_visible_when_entry_conditions_hide_every_entry(
    #[case] scope: FilterScope,
) {
    let mut harness = harness_of(
        vec![recording("walk.gtd", 55.0)],
        &[("source.log", SOURCE_LOG)],
    );
    set_display_option(&mut harness, "Show structural lines", true);
    let shown = harness.state().first_loaded_log();
    let (stack, _) = harness
        .state_mut()
        .logs
        .filter_stack_mut_by_id(shown)
        .unwrap();
    stack.set_live_filter_scope(scope);
    stack.set_live_filter_text("no entry matches");
    stack.add_live_filter_as_chip();
    let second = stack.create_group();
    stack.set_group_operator(second, FilterGroupOperator::Any);
    stack.set_live_filter_text("no other entry matches");
    stack.wait_for_queries();
    harness.run_steps(2);
    assert_eq!(match_count(&harness), "0 of 3");
    harness.get_by_label("--- Device reboot ---");
    harness.get_by_label("Boot 2 · up 10s · 2 entries");
    harness.get_by_label("----------- Journal summary -----------");
    let tail = harness.get_by_label("trailing source text").rect();
    harness.hover_at(tail.center());
    harness.step();
    assert_eq!(harness.state().log_hover.row_placement, None);
    harness.click_at(tail.center());
    harness.step();
    assert_eq!(harness.state().map_center, None);
    assert_eq!(harness.state_mut().map_matches().match_count(), 0);
}

#[rstest]
#[case::anomaly("Line 6", ORDER_ANOMALY_ENTRY_TIMESTAMP, 4)]
#[case::boot("Boot 2", "Boot 2 · up 10s · 3 entries", 3)]
fn hidden_diagnostics_reveal_exact_entries_without_changing_filters_or_map_matches(
    #[case] button: &str,
    #[case] target: &str,
    #[case] expected_entry: usize,
) {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    type_into_live_filter(&mut harness, "0x0000");
    unfold_the_summary_panel(&mut harness);
    let stack = harness.state().shown_log().unwrap().filters();
    let visible = stack.visible_entries().clone();
    let stored = stack.to_stored_stack();
    let map = harness.state_mut().map_matches().clone();
    let node = harness.get_by_label(button);
    assert!(!node.accesskit_node().is_disabled());
    node.click();
    harness.run_steps(2);
    harness.get_by_label(target);
    let state = harness.state();
    let reveal = state.viewer.diagnostic_reveal.unwrap();
    assert_eq!(reveal.entry_index, expected_entry);
    let stack = state.shown_log().unwrap().filters();
    assert_eq!(*stack.visible_entries(), visible);
    assert_eq!(stack.to_stored_stack(), stored);
    assert_eq!(match_count(&harness), "1 of 6");
    assert_eq!(harness.state_mut().map_matches().layers(), map.layers());
    assert_eq!(
        harness.state_mut().map_matches().match_count(),
        map.match_count()
    );
}

#[test]
fn another_diagnostic_replaces_the_reveal_and_a_visible_target_clears_it() {
    let mut harness = harness_with(Vec::new());
    type_into_live_filter(&mut harness, "0x0000");
    unfold_the_summary_panel(&mut harness);
    harness.get_by_label("Line 6").click();
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .viewer
            .diagnostic_reveal
            .unwrap()
            .entry_index,
        4
    );
    harness.get_by_label("Boot 2").click();
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .viewer
            .diagnostic_reveal
            .unwrap()
            .entry_index,
        3
    );
    assert!(
        harness
            .query_by_label(ORDER_ANOMALY_ENTRY_TIMESTAMP)
            .is_none()
    );
    harness.get_by_label("Boot 1").click();
    harness.run_steps(2);
    assert_eq!(harness.state().viewer.diagnostic_reveal, None);
}

#[test]
fn idle_frames_and_scan_landing_preserve_reveals_and_cached_rows() {
    let mut state = viewer_state(Vec::new(), &[("nav.log", LOG_WITH_EVERY_ROW_KIND)]);
    let shown = state.first_loaded_log();
    let (stack, _) = state.logs.filter_stack_mut_by_id(shown).unwrap();
    stack.set_live_filter_text("0x0000");
    stack.wait_for_queries();
    stack.set_live_filter_text("starting");
    let log = state.logs.get_by_id(shown).unwrap();
    state
        .viewer
        .navigate_to_diagnostic(log, shown, DiagnosticTarget::Entry(4));
    let reveal = state.viewer.diagnostic_reveal;
    let rows = state.viewer.table_rows(log, shown);
    for _ in 0..3 {
        assert!(Arc::ptr_eq(&rows, &state.viewer.table_rows(log, shown)));
    }
    let (stack, _) = state.logs.filter_stack_mut_by_id(shown).unwrap();
    let revision = stack.semantic_revision();
    stack.set_live_filter_text("starting");
    assert_eq!(stack.semantic_revision(), revision);
    stack.wait_for_queries();
    let log = state.logs.get_by_id(shown).unwrap();
    let landed = state.viewer.table_rows(log, shown);
    assert!(!Arc::ptr_eq(&rows, &landed));
    assert_eq!(state.viewer.diagnostic_reveal, reveal);
    assert!(landed.row_of_exact_entry(4).is_some());
    assert!(Arc::ptr_eq(&landed, &state.viewer.table_rows(log, shown)));
}

#[derive(Clone, Copy, Debug)]
enum SemanticEdit {
    Add,
    Clear,
    CreateGroup,
    Enabled,
    Layer,
    Level,
    Membership,
    Operator,
    Refine,
    Regex,
    Remove,
    RemoveGroup,
    Scope,
    SelectGroup,
    Text,
}

#[rstest]
#[case::text(SemanticEdit::Text)]
#[case::regex(SemanticEdit::Regex)]
#[case::scope(SemanticEdit::Scope)]
#[case::level(SemanticEdit::Level)]
#[case::clear(SemanticEdit::Clear)]
#[case::add(SemanticEdit::Add)]
#[case::remove(SemanticEdit::Remove)]
#[case::enabled(SemanticEdit::Enabled)]
#[case::layer(SemanticEdit::Layer)]
#[case::refine(SemanticEdit::Refine)]
#[case::operator(SemanticEdit::Operator)]
#[case::membership(SemanticEdit::Membership)]
#[case::create_group(SemanticEdit::CreateGroup)]
#[case::remove_group(SemanticEdit::RemoveGroup)]
#[case::select_group(SemanticEdit::SelectGroup)]
fn filter_semantic_edits_clear_reveals_before_scan_landing(#[case] edit: SemanticEdit) {
    let mut state = viewer_state(Vec::new(), &[("nav.log", LOG_WITH_EVERY_ROW_KIND)]);
    let shown = state.first_loaded_log();
    let (stack, slots) = state.logs.filter_stack_mut_by_id(shown).unwrap();
    stack.set_live_filter_text("starting");
    let chip = stack.add_live_filter_as_chip().unwrap();
    let other_group = stack.create_group();
    let first_group = stack.groups().first().unwrap().id();
    stack.select_group(first_group);
    stack.set_live_filter_text("fix");
    if matches!(edit, SemanticEdit::Refine) {
        stack.add_chip_effect(chip, FilterEffect::Map, slots);
        stack.remove_chip_effect(chip, FilterEffect::Table, slots);
    }
    if matches!(edit, SemanticEdit::Level) {
        stack.set_live_filter_scope(FilterScope::Level);
    }
    stack.wait_for_queries();
    let log = state.logs.get_by_id(shown).unwrap();
    state
        .viewer
        .navigate_to_diagnostic(log, shown, DiagnosticTarget::Entry(4));
    assert!(state.viewer.diagnostic_reveal.is_some());
    let (stack, slots) = state.logs.filter_stack_mut_by_id(shown).unwrap();
    match edit {
        SemanticEdit::Text => stack.set_live_filter_text("starting"),
        SemanticEdit::Regex => stack.set_live_filter_regex(true),
        SemanticEdit::Scope => stack.set_live_filter_scope(FilterScope::Service),
        SemanticEdit::Level => stack.set_live_filter_level(LogLevelKind::Error),
        SemanticEdit::Clear => stack.clear_live_filter(),
        SemanticEdit::Add => {
            stack.add_live_filter_as_chip();
        }
        SemanticEdit::Remove => stack.remove_chip(chip, slots),
        SemanticEdit::Enabled => stack.set_chip_effect_enabled(chip, FilterEffect::Table, false),
        SemanticEdit::Layer => stack.add_chip_effect(chip, FilterEffect::Map, slots),
        SemanticEdit::Refine => stack.add_chip_effect(chip, FilterEffect::Table, slots),
        SemanticEdit::Operator => stack.set_group_operator(first_group, FilterGroupOperator::Any),
        SemanticEdit::Membership => stack.move_chip_to_group(chip, other_group),
        SemanticEdit::CreateGroup => {
            stack.create_group();
        }
        SemanticEdit::RemoveGroup => stack.remove_group(first_group),
        SemanticEdit::SelectGroup => stack.select_group(other_group),
    }
    let pending = stack.is_query_pending();
    let log = state.logs.get_by_id(shown).unwrap();
    state.viewer.table_rows(log, shown);
    assert_eq!(state.viewer.diagnostic_reveal, None);
    assert_eq!(state.viewer.scroll_to_row, None);
    assert_eq!(log.filters().is_query_pending(), pending);
}

#[test]
fn switching_or_reloading_logs_clears_diagnostic_reveals() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("first.log", SECOND_LOG),
            ("second.log", LOG_WITH_EVERY_ROW_KIND),
        ],
    );
    type_into_live_filter(&mut harness, "0x0000");
    unfold_the_summary_panel(&mut harness);
    harness.get_by_label("Line 6").click();
    harness.run_steps(2);
    assert!(harness.state().viewer.diagnostic_reveal.is_some());
    harness.get_by_label("first.log").click();
    harness.run_steps(2);
    assert_eq!(harness.state().viewer.diagnostic_reveal, None);
    harness.get_by_label("second.log").click();
    harness.run_steps(2);
    assert_eq!(harness.state().viewer.diagnostic_reveal, None);
    harness.get_by_label("Line 6").click();
    harness.run_steps(2);
    let old = harness.state().viewer.selected.unwrap();
    let state = harness.state_mut();
    state.logs.remove_by_id(old);
    let parsed = gt_logfile::parse_log(LOG_WITH_EVERY_ROW_KIND.into(), log_start()).unwrap();
    let replacement = state
        .logs
        .push(LoadedLog::new(
            Some("second.log".to_owned()),
            parsed,
            Duration::seconds(60),
        ))
        .id();
    assert_ne!(replacement, old);
    harness.run_steps(2);
    assert_eq!(harness.state().viewer.diagnostic_reveal, None);
}

const SOURCE_LOG: &str = "\
2026-05-29 18:48:25 navsyncd: first
--- Device reboot ---
2026-05-29 18:48:30 navsyncd: second
2026-05-29 18:48:40 navsyncd: last
----------- Journal summary -----------
trailing source text
";

#[test]
fn structural_source_table_snapshot() {
    let mut harness = rendering_harness_of(Vec::new(), &[("source.log", SOURCE_LOG)]);
    set_display_option(&mut harness.inner, "Show structural lines", true);
    harness.snapshot("log_viewer_source_lines");
}

#[rstest]
#[case::anomaly(false)]
#[case::boot(true)]
fn diagnostic_navigation_scrolls_to_the_hidden_target_in_a_long_table(
    #[case] boot: bool,
    #[values(false, true)] structural: bool,
) {
    let text: String = (0..200)
        .map(|index| {
            let separator = if boot && matches!(index, 120 | 140) {
                "--- Device reboot ---\n"
            } else {
                ""
            };
            let hidden = if boot {
                (120..140).contains(&index)
            } else {
                index == 120
            };
            let second = if !boot && hidden { 60 } else { index };
            format!(
                "{separator}{} navsyncd: {} {index}\n",
                (log_start() + Duration::seconds(second)).format(super::super::TIMESTAMP_FORMAT),
                if hidden { "hidden" } else { "keep" },
            )
        })
        .collect();
    let mut harness = harness_of(Vec::new(), &[("long.log", &text)]);
    if structural {
        set_display_option(&mut harness, "Show structural lines", true);
    }
    type_into_live_filter(&mut harness, "keep");
    unfold_the_summary_panel(&mut harness);
    let button = if boot { "Boot 2" } else { "Line 121" };
    harness.get_by_label(button).click();
    harness.run_steps(2);
    let target = if boot {
        "Boot 2 · up 19s · 20 entries"
    } else {
        "navsyncd: hidden 120"
    };
    harness.get_by_label(target);
    assert!(harness.query_by_label("navsyncd: keep 119").is_none());
    assert_eq!(
        harness
            .state()
            .viewer
            .diagnostic_reveal
            .unwrap()
            .entry_index,
        120
    );
    assert_eq!(
        match_count(&harness),
        if boot { "180 of 200" } else { "199 of 200" }
    );
}

#[test]
fn source_backed_boot_dividers_fit_a_narrow_viewer_and_keep_navigation_on_one_row() {
    let mut state = viewer_state(Vec::new(), &[("source.log", SOURCE_LOG)]);
    state.viewer.show_structural_lines = true;
    state.viewer.summary_expanded = true;
    let id = state.first_loaded_log();
    let (stack, _) = state.logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text("first");
    stack.wait_for_queries();
    let mut harness = Harness::builder()
        .with_size(NARROW_VIEWPORT)
        .build_ui_state(viewer_ui, state);
    gt_ui_theme::install_app_style(&harness.ctx);
    harness.run_steps(8);
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    harness.get_by_label("Boot 2").click();
    harness.run_steps(2);
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    let divider = harness.get_by_label("Boot 2 · up 10s · 2 entries").rect();
    let source = harness.get_by_label("--- Device reboot ---").rect();
    let entry = harness.get_by_label("navsyncd: second").rect();
    let window = harness
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::new(Some(LOG_VIEWER_TITLE))))
        .unwrap();
    assert!(divider.right() <= window.right() - 7.0);
    assert!(source.right() <= divider.left());
    assert!((source.center().y - divider.center().y).abs() < 0.1);
    let row_height = harness.ctx.global_style().spacing.interact_size.y;
    assert!(source.height() <= row_height);
    assert!(divider.height() <= row_height);
    assert!((entry.top() - divider.top() - row_height).abs() < 0.1);
    assert_eq!(
        harness
            .state()
            .viewer
            .diagnostic_reveal
            .unwrap()
            .entry_index,
        1
    );
    harness.hover_and_settle(
        By::new().label("Boot 2 · up 10s · 2 entries"),
        TOOLTIP_DELAY_FRAMES,
    );
    assert!(
        harness
            .get_all_by_label("Boot 2 · up 10s · 2 entries")
            .count()
            >= 2
    );
}

#[rstest]
#[case::entry(DiagnosticTarget::Entry(usize::MAX))]
#[case::boot(DiagnosticTarget::BootSession(usize::MAX))]
fn invalid_diagnostic_targets_preserve_active_navigation(#[case] target: DiagnosticTarget) {
    let mut state = viewer_state(Vec::new(), &[("source.log", SOURCE_LOG)]);
    let id = state.first_loaded_log();
    let (stack, _) = state.logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text("first");
    stack.wait_for_queries();
    let log = state.logs.get_by_id(id).unwrap();
    state
        .viewer
        .navigate_to_diagnostic(log, id, DiagnosticTarget::BootSession(1));
    let reveal = state.viewer.diagnostic_reveal;
    let scroll = state.viewer.scroll_to_row;
    let rows = state.viewer.table_rows(log, id);
    state.viewer.navigate_to_diagnostic(log, id, target);
    assert_eq!(state.viewer.diagnostic_reveal, reveal);
    assert_eq!(state.viewer.scroll_to_row, scroll);
    assert!(Arc::ptr_eq(&rows, &state.viewer.table_rows(log, id)));
}
