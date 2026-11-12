use egui::Popup;
use gt_log_view::{SourceSearchScope, SourceSearchStatus};

use super::*;
use crate::app::log_viewer::find::{self, LogFind};

impl ViewerState {
    fn active_find(&self) -> &LogFind {
        self.viewer.active_state().find.as_ref().unwrap()
    }
}

fn finish_find(harness: &mut Harness<'_, ViewerState>) {
    assert!(harness.step_until(|harness| matches!(
        harness.state().active_find().search.status(),
        SourceSearchStatus::ShowResults | SourceSearchStatus::ReportEmptyQuery
    )));
    harness.run_steps(3);
}

fn search_for(harness: &mut Harness<'_, ViewerState>, text: &str) {
    let id = harness.state().viewer.selected_log().unwrap();
    if harness
        .state()
        .viewer
        .active_state()
        .find
        .as_ref()
        .is_none_or(|find| !find.open)
    {
        harness.state_mut().viewer.set_find_open(id, true);
    }
    harness.run_steps(2);
    harness
        .ctx
        .memory_mut(|memory| memory.request_focus(egui::Id::new((find::FIND_FIELD_ID, id))));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    harness.run_steps(2);
    finish_find(harness);
}

#[rstest]
#[case::default(VIEWER_SIZE, "log_find")]
#[case::narrow(NARROW_VIEWPORT, "log_find_narrow")]
#[case::short(SHORT_VIEWPORT, "log_find_short")]
fn find_reveals_exact_hidden_hits_and_restores_filtered_reading(
    #[case] viewport: egui::Vec2,
    #[case] snapshot: &str,
) {
    let mut rendering = TestHarness::builder().size(viewport).ui_state(
        viewer_ui,
        viewer_state(
            vec![recording("walk.gtd", 55.0)],
            &[("source.log", FIND_SOURCE)],
        ),
    );
    let harness = &mut rendering.inner;
    harness.run_steps(3);
    add_filter(harness, "beta");
    add_map_highlight(harness, "alpha");
    let before = harness.state().viewer.active_state().table_reading_anchor;
    let filters = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    let matches = harness.state_mut().map_matches().clone();
    assert_eq!(matches.match_count(), 1);
    search_for(harness, "alpha");
    assert_eq!(
        harness.state().active_find().scope,
        SourceSearchScope::SearchWholeLog
    );
    assert_eq!(
        harness
            .state()
            .active_find()
            .search
            .current()
            .unwrap()
            .line_number(),
        1
    );
    harness.get_by_label("1 / 2");
    harness.get_by_label(source_navigation_ui::REVEAL_LABEL);
    assert_eq!(match_count(harness), "1 of 2");
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    let count = harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "1 / 2" => {
                Some((text, shape.clip_rect))
            }
            _ => None,
        })
        .unwrap();
    assert!(count.1.contains_rect(count.0.visual_bounding_rect()));
    rendering.snapshot_with_color_tolerance(snapshot);
    let harness = &mut rendering.inner;
    harness.get_by_label(find::NEXT_LABEL).click();
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .active_find()
            .search
            .current()
            .unwrap()
            .byte_range(),
        31..36
    );
    harness.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new((
            find::FIND_FIELD_ID,
            harness.state().viewer.selected_log().unwrap(),
        )))
    });
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(
        harness.state().active_find().search.current_index(),
        Some(0)
    );
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .active_find()
            .search
            .current()
            .unwrap()
            .line_number(),
        1
    );
    harness
        .get_by_label(source_navigation_ui::RETURN_LABEL)
        .click();
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .viewer
            .active_state()
            .source_reveal
            .is_none()
    );
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .table_reading_anchor
            .map(|anchor| anchor.source_line),
        before.map(|anchor| anchor.source_line)
    );
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        filters
    );
    assert_eq!(harness.state_mut().map_matches(), &matches);
}

#[test]
fn find_preserves_queries_occurrences_and_source_text_across_tabs_and_chronology() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("first.log", gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG),
            ("second.log", FIND_SOURCE),
        ],
    );
    search_for(&mut harness, "alpha");
    harness.get_by_label(find::NEXT_LABEL).click();
    harness.run_steps(3);
    let second = harness
        .state()
        .active_find()
        .search
        .current()
        .unwrap()
        .clone();
    select_log(&mut harness, "first.log");
    search_for(&mut harness, "Feb 29");
    harness.get_by_label(find::NEXT_LABEL).click();
    harness.run_steps(3);
    let first = harness
        .state()
        .active_find()
        .search
        .current()
        .unwrap()
        .clone();
    let state = harness.state_mut();
    let id = state.viewer.selected_log().unwrap();
    assert!(state.logs.get_mut_by_id(id).unwrap().resolve_years_as(
        YearResolution::PlaceLastEntryInYear(2024),
        &state.recordings.view()
    ));
    state
        .viewer
        .preserve_source_position_after_chronology_change(state.logs.get_by_id(id).unwrap(), id);
    harness.run_steps(3);
    assert_eq!(harness.state().active_find().search.current(), Some(&first));
    assert_eq!(harness.state().active_find().text, "Feb 29");
    harness.get_by_label("Feb 29 12:00:00 host service: later leap day");
    select_log(&mut harness, "second.log");
    assert_eq!(
        harness.state().active_find().search.current(),
        Some(&second)
    );
    assert_eq!(harness.state().active_find().text, "alpha");
    harness.get_by_label(find::CLOSE_LABEL).click();
    harness.run_steps(3);
    assert!(harness.state().viewer.open);
    assert!(!harness.state().active_find().open);
    table_focus::assert_table_focused(&harness);
}

#[test]
fn find_scope_tracks_filter_scan_landings_and_query_replacement() {
    let mut harness = harness_of(Vec::new(), &[("source.log", FIND_SOURCE)]);
    search_for(&mut harness, "alpha");
    harness
        .get(By::new().role(Role::ComboBox).value("Whole log"))
        .click();
    harness.run_steps(2);
    harness.get_by_label("Filtered results").click();
    harness.run_steps(2);
    finish_find(&mut harness);
    assert_eq!(harness.state().active_find().search.occurrences().len(), 2);
    add_filter(&mut harness, "beta");
    finish_find(&mut harness);
    harness.get_by_label("0 / 0");
    assert!(
        harness
            .get_by_label(find::NEXT_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    search_for(&mut harness, "2026-05-29");
    assert_eq!(harness.state().active_find().search.occurrences().len(), 1);
    assert_eq!(
        harness
            .state()
            .active_find()
            .search
            .current()
            .unwrap()
            .line_number(),
        4
    );
    search_for(&mut harness, "absent");
    assert_eq!(harness.state().active_find().search.occurrences().len(), 0);
    let id = harness.state().viewer.selected_log().unwrap();
    harness
        .ctx
        .memory_mut(|memory| memory.request_focus(egui::Id::new((find::FIND_FIELD_ID, id))));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.key_press(egui::Key::Backspace);
    harness.run_steps(3);
    assert_eq!(
        harness.state().active_find().search.status(),
        &SourceSearchStatus::ReportEmptyQuery
    );
}

#[test]
fn find_shortcuts_open_from_viewer_controls_and_escape_closes_only_find() {
    let mut harness = harness_of(Vec::new(), &[("source.log", FIND_SOURCE)]);
    type_into_live_filter(&mut harness, "alpha");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    harness.run_steps(3);
    assert!(harness.state().active_find().open);
    harness.get_by_label(find::CLOSE_LABEL).click();
    harness.run_steps(3);
    harness.ctx.memory_mut(|memory| {
        let focused = memory.focused().unwrap();
        memory.surrender_focus(focused);
    });
    table_focus::focus_table(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    harness.run_steps(3);
    assert!(harness.state().active_find().open);
    harness
        .get(By::new().role(Role::ComboBox).value("Whole log"))
        .click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.state().active_find().open);
    assert!(harness.state().viewer.open);
    assert!(!Popup::is_any_open(&harness.ctx));
    let id = harness.state().viewer.selected_log().unwrap();
    harness
        .ctx
        .memory_mut(|memory| memory.request_focus(egui::Id::new((find::FIND_FIELD_ID, id))));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(!harness.state().active_find().open);
    assert!(harness.state().viewer.open);
}

#[test]
fn find_navigates_raw_and_structural_source_without_entry_timestamps() {
    let mut harness = harness_of(Vec::new(), &[("source.log", FIND_SOURCE)]);
    search_for(&mut harness, "\t");
    assert_eq!(
        harness
            .state()
            .active_find()
            .search
            .current()
            .unwrap()
            .line_number(),
        2
    );
    let state = harness.state_mut();
    let id = state.viewer.selected_log().unwrap();
    let rows = state
        .viewer
        .table_rows(state.logs.get_by_id(id).unwrap(), id);
    assert!((0..rows.len()).any(|index| rows.at(index)
        == Some(line_table::LineTableRow::RenderRawSource { line_number: 2 })));
    harness
        .get_by_label(source_navigation_ui::RETURN_LABEL)
        .click();
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .viewer
            .active_state()
            .source_reveal
            .is_none()
    );
    harness.get_by_label(find::NEXT_LABEL).click();
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .viewer
            .active_state()
            .source_reveal
            .is_some()
    );
    search_for(&mut harness, "Device reboot");
    assert_eq!(
        harness
            .state()
            .active_find()
            .search
            .current()
            .unwrap()
            .line_number(),
        3
    );
    assert_eq!(harness.get_all_by_label("--- Device reboot ---").count(), 2);
    assert_eq!(match_count(&harness), "2 of 2");
}

#[test]
fn find_background_preserves_selected_condition_foreground() {
    let mut rendering = rendering_harness_of(Vec::new(), &[("source.log", FIND_SOURCE)]);
    let harness = &mut rendering.inner;
    add_filter(harness, "alpha");
    search_for(harness, "alpha");
    let colors = harness.ctx.global_style().visuals.clone();
    let texts = harness
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "svc: alpha alpha" => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    let text = texts.first().unwrap();
    let formats = text
        .galley
        .job
        .sections
        .iter()
        .filter(|section| {
            text.galley
                .job
                .text
                .get(section.byte_range.start.0..section.byte_range.end.0)
                == Some("alpha")
        })
        .map(|section| &section.format)
        .collect::<Vec<_>>();
    assert_eq!(formats.len(), 2);
    assert_eq!(
        formats.first().unwrap().color,
        gt_ui_theme::LOG_LIVE_FILTER.resolve(colors.dark_mode)
    );
    assert_eq!(
        formats.first().unwrap().background,
        colors.selection.bg_fill
    );
    assert_eq!(
        formats.last().unwrap().background,
        egui::Color32::TRANSPARENT
    );
}

const FIND_SOURCE: &str = "2026-05-29 18:48:25 svc: alpha alpha\n \t\n--- Device reboot ---\n2026-05-29 18:48:26 svc: beta\n";
