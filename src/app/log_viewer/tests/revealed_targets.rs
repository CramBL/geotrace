use egui::Button;

use super::table_filter_controls::SourceReadingPosition;
use super::*;

#[rstest]
#[case::default(VIEWER_SIZE, "log_viewer_revealed_source")]
#[case::narrow(NARROW_VIEWPORT, "log_viewer_revealed_source_narrow")]
#[case::short(SHORT_VIEWPORT, "log_viewer_revealed_source_short")]
fn the_viewer_outlines_revealed_rows_and_restores_filtered_source_positions(
    #[case] viewport: egui::Vec2,
    #[case] snapshot: &str,
) {
    let text = super::long_log(LONG_LOG_ENTRIES);
    let mut rendering = TestHarness::builder().size(viewport).ui_state(
        super::viewer_ui,
        super::viewer_state(
            vec![super::recording("walk.gtd", 55.0)],
            &[("long.log", &text)],
        ),
    );
    rendering.inner.run_steps(3);
    let harness = &mut rendering.inner;
    let id = harness.state().first_loaded_log();
    let (stack, slots) = harness.state_mut().logs.filter_stack_mut_by_id(id).unwrap();
    stack.set_live_filter_text("entry");
    stack.add_live_filter_as_map_highlight(slots).unwrap();
    stack.set_live_filter_text("entry 1");
    stack.wait_for_queries();
    harness.run_steps(3);
    harness.state_mut().viewer.active_state_mut().scroll_to_row = Some(8);
    harness.run_steps(3);
    let prior = SourceReadingPosition::of(harness);
    let stored = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    let map = harness.state_mut().map_matches().clone();
    assert!(map.match_count() > 0);
    harness.state_mut().clicked_glyph = Some(LogMatchGlyph {
        log: id,
        color: LogMatchColor::LiveFilter,
        entry_indices: vec![0, 1],
    });
    harness.run_steps(3);
    let reason = "Line 1 is outside the table conditions\nMessage entry 1";
    let chip = harness.get_by_label(source_navigation_ui::REVEAL_LABEL);
    assert_eq!(chip.accesskit_node().description().as_deref(), Some(reason));
    let row = harness.get_by_label("navsyncd: entry 0").rect();
    let amber = gt_ui_theme::warning_amber(harness.ctx.global_style().visuals.dark_mode);
    assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(outline) if outline.stroke.color == amber && outline.rect.contains(row.center()))));
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    assert!(
        harness.ctx.content_rect().contains_rect(
            harness
                .get_by_label(source_navigation_ui::RETURN_LABEL)
                .rect()
        )
    );
    rendering.snapshot_with_color_tolerance(snapshot);
    let harness = &mut rendering.inner;
    harness
        .get_by_label(source_navigation_ui::REVEAL_LABEL)
        .click();
    harness.run_steps(2);
    harness.get_by_label(reason);
    harness
        .get_by_label(source_navigation_ui::REVEAL_LABEL)
        .click();
    harness.run_steps(2);
    harness
        .get_by_label(source_navigation_ui::RETURN_LABEL)
        .click();
    harness.run_steps(3);
    prior.assert_retained(harness);
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        stored
    );
    assert_eq!(harness.state_mut().map_matches(), &map);
    assert!(
        harness
            .query_by_label(source_navigation_ui::REVEAL_LABEL)
            .is_none()
    );
}

#[rstest]
#[case::anchor("Year of line 2", 1)]
#[case::conflict("Cannot resolve the year of log line 1", 0)]
fn chronology_source_buttons_reveal_exact_entries_without_absolute_years(
    #[case] label: &str,
    #[case] entry: usize,
) {
    let mut harness = super::harness_of(Vec::new(), &[("unresolved.log", UNRESOLVED_SOURCE)]);
    super::type_into_live_filter(&mut harness, "absent");
    super::open_date_inspector(&mut harness);
    if entry == 0 {
        super::click_inspector_control(&mut harness, "Date diagnostics and help");
    }
    super::click_inspector_control(&mut harness, label);
    assert!(
        !harness
            .state()
            .shown_log()
            .unwrap()
            .parsed()
            .chronology()
            .is_resolved()
    );
    assert_eq!(
        harness
            .state()
            .viewer
            .active_state()
            .source_reveal
            .unwrap()
            .entry_index(),
        Some(entry)
    );
    harness.get_by_label(source_navigation_ui::REVEAL_LABEL);
    assert_eq!(super::match_count(&harness), "0 of 2");
    assert_eq!(harness.state_mut().map_matches().match_count(), 0);
    harness
        .get_by_label(source_navigation_ui::RETURN_LABEL)
        .click();
    harness.run_steps(3);
    assert_eq!(super::table_row_count(&harness), 0);
}

#[test]
fn an_unavailable_source_button_preserves_reading_position_and_shows_brief_feedback() {
    let text = super::long_log(LONG_LOG_ENTRIES);
    let state = super::viewer_state(Vec::new(), &[("source.log", &text)]);
    let id = state.first_loaded_log();
    let mut harness = Harness::builder().with_size(VIEWER_SIZE).build_ui_state(
        move |ui, state: &mut ViewerState| {
            super::viewer_ui(ui, state);
            state.viewer.source_link_ui(
                ui,
                state.logs.get_by_id(id).unwrap(),
                id,
                line_table::SourceRequest::NavigateToLine(u32::MAX),
                Button::new("Unavailable source"),
            );
        },
        state,
    );
    gt_ui_theme::install_app_style(&harness.ctx);
    harness.run_steps(3);
    let prior = SourceReadingPosition::of(&harness);
    harness.get_by_label("Unavailable source").click_accesskit();
    harness.run_steps(3);
    harness.get_by_label(source_navigation_ui::UNAVAILABLE_SOURCE_LABEL);
    prior.assert_retained(&harness);
    harness.run_steps(16);
    assert!(
        harness
            .query_by_label(source_navigation_ui::UNAVAILABLE_SOURCE_LABEL)
            .is_none()
    );
}

#[test]
fn structural_source_links_mark_exact_rows_without_map_positions() {
    let mut harness = super::harness_of(Vec::new(), &[("source.log", SOURCE_LOG)]);
    let state = harness.state_mut();
    let id = state.first_loaded_log();
    assert!(state.viewer.navigate_to_source(
        state.logs.get_by_id(id).unwrap(),
        id,
        line_table::SourceRequest::NavigateToLine(2)
    ));
    harness.run_steps(3);
    let source = harness.get_by_label("--- Device reboot ---").rect();
    let amber = gt_ui_theme::warning_amber(harness.ctx.global_style().visuals.dark_mode);
    assert!(harness.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(outline) if outline.stroke.color == amber && outline.rect.contains(source.center()))));
    assert_eq!(
        harness
            .get_by_label(source_navigation_ui::REVEAL_LABEL)
            .accesskit_node()
            .description()
            .as_deref(),
        Some("Line 2 is hidden while structural source lines are off")
    );
    assert_eq!(super::match_count(&harness), "3 of 3");
    harness.hover_at(source.center());
    harness.step();
    assert_eq!(harness.state().log_hover.row_placement, None);
    harness
        .get_by_label(source_navigation_ui::RETURN_LABEL)
        .click();
    harness.run_steps(3);
    assert!(harness.query_by_label("--- Device reboot ---").is_none());
}

const UNRESOLVED_SOURCE: &str = "\
May 20 11:00:00 host svc: conflict
May 20 12:00:00 host svc: anchor
----------- Journal summary -----------
Logs begin at: Wed 20-May-2026 12:00:00 UTC
Logs end at: Wed 20-May-2026 12:00:10 UTC
";
