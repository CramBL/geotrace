use gt_log_view::SourceSearchStatus;

use super::*;
use crate::app::log_viewer::{find, jump};

enum JumpInput<'a> {
    GoToLine(&'a str),
    GoToTime(&'a str),
}

impl JumpInput<'_> {
    fn edit(self, harness: &mut Harness<'_, ViewerState>) {
        let (mode, text) = match self {
            Self::GoToLine(text) => ("Go to line…", text),
            Self::GoToTime(text) => ("Go to time…", text),
        };
        harness.get_by_label("Go to…").click();
        harness.run_steps(2);
        harness.get_by_label(mode).click();
        harness.run_steps(3);
        let field = By::new().predicate(|node| node.author_id() == Some(jump::JUMP_FIELD_ID));
        harness.get(field.clone()).scroll_to_me();
        harness.run_steps(2);
        harness.get(field).click();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run_steps(2);
        harness
            .input_mut()
            .events
            .push(egui::Event::Text(text.to_owned()));
        harness.run_steps(2);
    }

    fn submit(self, harness: &mut Harness<'_, ViewerState>) {
        self.edit(harness);
        harness.get_by_label("Go").click();
        harness.run_steps(4);
    }
}

#[rstest]
#[case::default(VIEWER_SIZE, "log_jump_line")]
#[case::narrow(NARROW_VIEWPORT, "log_jump_line_narrow")]
#[case::short(SHORT_VIEWPORT, "log_jump_line_short")]
fn physical_jumps_reveal_exact_source_and_preserve_filters_and_map(
    #[case] viewport: egui::Vec2,
    #[case] snapshot: &str,
) {
    let mut rendering = TestHarness::builder().size(viewport).ui_state(
        viewer_ui,
        viewer_state(
            vec![recording("walk.gtd", 55.0)],
            &[("source.log", JUMP_SOURCE)],
        ),
    );
    let harness = &mut rendering.inner;
    harness.run_steps(3);
    assert!(harness.query_by_label("Go").is_none());
    add_filter(harness, "beta");
    add_map_highlight(harness, "alpha");
    let filters = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    let matches = harness.state_mut().map_matches().clone();
    let before = harness.state().viewer.active_state().table_reading_anchor;
    JumpInput::GoToLine("0").submit(harness);
    harness.get_by_label("Enter a physical line number greater than zero");
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .table_reading_anchor
            .map(|anchor| anchor.source_line),
        before.map(|anchor| anchor.source_line)
    );
    JumpInput::GoToLine("1").submit(harness);
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .source_reveal
            .unwrap()
            .entry_index(),
        Some(0)
    );
    harness.get_by_label("svc: alpha");
    assert_eq!(match_count(harness), "1 of 2");
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    let go = harness
        .output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Go" => {
                Some((text, shape.clip_rect))
            }
            _ => None,
        })
        .unwrap();
    let clip = go.1;
    let bounds = go.0.visual_bounding_rect();
    rendering.snapshot_with_color_tolerance(snapshot);
    assert!(
        clip.contains_rect(bounds),
        "Go bounds {bounds:?}, clip {clip:?}"
    );
    let harness = &mut rendering.inner;
    JumpInput::GoToLine("2").submit(harness);
    let state = harness.state_mut();
    let id = state.viewer.selected_log().unwrap();
    let rows = state
        .viewer
        .table_rows(state.logs.get_by_id(id).unwrap(), id);
    assert!((0..rows.len()).any(|index| rows.at(index)
        == Some(line_table::LineTableRow::RenderRawSource { line_number: 2 })));
    JumpInput::GoToLine("3").submit(harness);
    harness.get_by_label("--- Device reboot ---");
    let prior = harness.state().viewer.active_state().table_reading_anchor;
    JumpInput::GoToLine("999").submit(harness);
    harness.get_by_label("Source line unavailable");
    assert_eq!(
        harness.state().viewer.active_state().table_reading_anchor,
        prior
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
fn time_jumps_open_date_controls_and_preserve_find_through_chronology_changes() {
    let mut harness = harness_of(
        Vec::new(),
        &[("unresolved.log", gt_test_utils::UNRESOLVED_YEARS_LOG)],
    );
    add_filter(&mut harness, "fix acquired");
    harness.get_by_label(find::FIND_LABEL).click();
    harness.run_steps(3);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("gnss".into()));
    assert!(harness.step_until(|harness| {
        harness
            .state()
            .viewer
            .active_state()
            .find
            .as_ref()
            .is_some_and(|find| find.search.status() == &SourceSearchStatus::ShowResults)
    }));
    let original_hit = harness
        .state()
        .viewer
        .active_state()
        .find
        .as_ref()
        .unwrap()
        .search
        .current()
        .unwrap()
        .clone();
    harness.get_by_label("Go to…").click();
    harness.run_steps(2);
    harness.get_by_label("Go to time…").click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().viewer.active_state().inspector,
        Some(InspectorSection::ShowChronology)
    );
    assert!(harness.get_by_label("Go").accesskit_node().is_disabled());
    let state = harness.state_mut();
    let id = state.viewer.selected_log().unwrap();
    assert!(state.logs.get_mut_by_id(id).unwrap().resolve_years_as(
        YearResolution::PlaceLastEntryInYear(2026),
        &state.recordings.view()
    ));
    state
        .viewer
        .preserve_source_position_after_chronology_change(state.logs.get_by_id(id).unwrap(), id);
    harness.run_steps(3);
    JumpInput::GoToTime("2026-05-20 11:30").submit(&mut harness);
    harness.get_by_label("Enter a date, time and UTC or a numeric offset");
    JumpInput::GoToTime("2026-05-20 11:30 UTC").submit(&mut harness);
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .source_reveal
            .unwrap()
            .entry_index(),
        Some(1)
    );
    let state = harness.state_mut();
    assert!(state.logs.get_mut_by_id(id).unwrap().resolve_years_as(
        YearResolution::PlaceLastEntryInYear(2025),
        &state.recordings.view()
    ));
    state
        .viewer
        .preserve_source_position_after_chronology_change(state.logs.get_by_id(id).unwrap(), id);
    harness.run_steps(3);
    harness.get_by_label("Go").click();
    harness.run_steps(3);
    harness.get_by_label("No entry at or after this time");
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .find
            .as_ref()
            .unwrap()
            .search
            .current(),
        Some(&original_hit)
    );
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .find
            .as_ref()
            .unwrap()
            .text,
        "gnss"
    );
}

#[test]
fn jump_drafts_remain_per_log_and_escape_preserves_the_viewer() {
    let second = JUMP_SOURCE.replace("svc:", "other:");
    let mut harness = harness_of(
        Vec::new(),
        &[("first.log", JUMP_SOURCE), ("second.log", &second)],
    );
    select_log(&mut harness, "first.log");
    JumpInput::GoToLine("2").edit(&mut harness);
    select_log(&mut harness, "second.log");
    JumpInput::GoToTime("2026-05-29 20:48:25 +02:00").edit(&mut harness);
    select_log(&mut harness, "first.log");
    let field =
        harness.get(By::new().predicate(|node| node.author_id() == Some(jump::JUMP_FIELD_ID)));
    assert_eq!(field.accesskit_node().value().as_deref(), Some("2"));
    field.click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.query_by_label("Go").is_none());
    assert!(harness.state().viewer.open);
    select_log(&mut harness, "second.log");
    harness.get(
        By::new()
            .role(Role::TextInput)
            .value("2026-05-29 20:48:25 +02:00"),
    );
}

const JUMP_SOURCE: &str =
    "2026-05-29 18:48:25 svc: alpha\n \t\n--- Device reboot ---\n2026-05-29 18:48:26 svc: beta\n";
