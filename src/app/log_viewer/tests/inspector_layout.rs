use super::table_filter_controls::SourceReadingPosition;
use super::*;

#[derive(Debug)]
struct ReadingRegions {
    table: egui::Rect,
    inspector: egui::Rect,
    controls: egui::Rect,
}

impl ReadingRegions {
    fn of(harness: &Harness<ViewerState>) -> Self {
        let clip_of = |label: &str| {
            harness
                .output()
                .shapes
                .iter()
                .find_map(|shape| {
                    let egui::Shape::Text(text) = &shape.shape else {
                        return None;
                    };
                    (text.galley.job.text == label
                        || (label == "Format" && text.galley.job.text == "Automatic"))
                        .then_some(shape.clip_rect)
                })
                .unwrap_or_else(|| panic!("painted {label}"))
        };
        Self {
            table: SourceReadingPosition::of(harness).viewport,
            inspector: clip_of("Format"),
            controls: clip_of("Notice 0"),
        }
    }

    fn assert_default_reservation(&self) {
        let height = self.table.bottom() - self.controls.top();
        assert!(self.inspector.height() <= height / 3.0 + 2.0, "{self:?}");
        assert!(self.table.height() >= height / 2.0 - 2.0, "{self:?}");
        assert!(self.inspector.bottom() < self.table.top());
    }
}

#[rstest]
#[case::default(
    VIEWER_SIZE,
    "log_viewer_inspector_default",
    InspectorSection::ShowSummary
)]
#[case::narrow(
    NARROW_VIEWPORT,
    "log_viewer_inspector_narrow",
    InspectorSection::ShowSummary
)]
#[case::short(
    SHORT_VIEWPORT,
    "log_viewer_inspector_short",
    InspectorSection::ShowSummary
)]
#[case::unresolved_narrow(
    NARROW_VIEWPORT,
    "log_viewer_unresolved_inspector_narrow",
    InspectorSection::ShowChronology
)]
#[case::unresolved_short(
    SHORT_VIEWPORT,
    "log_viewer_unresolved_inspector_short",
    InspectorSection::ShowChronology
)]
fn the_inspector_reserves_table_space_with_overflowing_controls(
    #[case] viewport: egui::Vec2,
    #[case] snapshot: &str,
    #[case] section: InspectorSection,
) {
    let text = match section {
        InspectorSection::ShowSummary => long_log(LONG_LOG_ENTRIES),
        InspectorSection::ShowChronology => format!(
            "{}{}",
            gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG,
            long_log(LONG_LOG_ENTRIES).replace("2026-05-29", "May 29")
        ),
    };
    let mut state = viewer_state(Vec::new(), &[("navsyncd.log", &text)]);
    state.viewer.notices = (0..40).map(|index| format!("Notice {index}")).collect();
    state.viewer.active_state_mut().inspector = Some(section);
    let id = state.first_loaded_log();
    let (stack, _) = state.logs.filter_stack_mut_by_id(id).unwrap();
    for _ in 0..8 {
        let group = stack.create_group();
        stack.select_group(group);
        stack.set_live_filter_text("entry");
        stack.add_live_filter_as_chip();
    }
    stack.wait_for_queries();
    let mut rendering = TestHarness::builder()
        .size(viewport)
        .ui_state(viewer_ui, state);
    rendering.inner.run_steps(6);
    let regions = ReadingRegions::of(&rendering.inner);
    regions.assert_default_reservation();
    rendering
        .inner
        .assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    rendering.inner.assert_control_is_reachable(
        AuditedWindow::titled(LOG_VIEWER_TITLE),
        ControlLabel("Positions from"),
    );
    rendering.snapshot_with_color_tolerance(snapshot);
    if section == InspectorSection::ShowChronology {
        click_inspector_control(&mut rendering.inner, unresolved_years::DETAILS_LABEL);
        assert_eq!(
            SourceReadingPosition::of(&rendering.inner).viewport,
            regions.table
        );
        select_date_policy(&mut rendering.inner, unresolved_years::SET_YEAR_LABEL);
        click_inspector_control(&mut rendering.inner, unresolved_years::APPLY_YEAR_LABEL);
        assert_eq!(
            rendering
                .inner
                .state()
                .requests
                .resolve_years
                .as_ref()
                .map(|request| request.log),
            Some(id)
        );
    }
    let disclosure = match section {
        InspectorSection::ShowSummary => super::super::SUMMARY_DISCLOSURE_ID,
        InspectorSection::ShowChronology => super::super::inspector::DATE_STATUS_ID,
    };
    rendering
        .inner
        .get(By::new().predicate(|node| node.author_id() == Some(disclosure)))
        .click();
    rendering.inner.run_steps(4);
    let table = SourceReadingPosition::of(&rendering.inner).viewport;
    assert!(table.height() >= (regions.table.bottom() - regions.controls.top()) / 2.0 - 2.0);
}

#[test]
fn unresolved_date_controls_open_in_the_inspector_and_disclose_full_diagnostics() {
    let mut rendering = rendering_harness_of(
        Vec::new(),
        &[("journal.log", gt_test_utils::UNRESOLVED_YEARS_LOG)],
    );
    let harness = &mut rendering.inner;
    assert!(
        harness
            .query_by_label(unresolved_years::APPLY_YEAR_LABEL)
            .is_none()
    );
    open_date_inspector(harness);
    harness.get_by_label(unresolved_years::APPLY_YEAR_LABEL);
    harness.get_by_label("May 20 11:00:00 host navsyncd: clock moved backwards");
    assert!(harness.query_by_label("Written as").is_none());
    click_inspector_control(harness, unresolved_years::DETAILS_LABEL);
    harness.get_by_label("Written as");
    let id = harness.state().first_loaded_log();
    assert_eq!(
        harness.state().viewer.active_state().inspector,
        Some(InspectorSection::ShowChronology)
    );
    unfold_the_summary_panel(harness);
    assert_eq!(
        harness.state().viewer.active_state().inspector,
        Some(InspectorSection::ShowSummary)
    );
    assert_eq!(
        harness
            .query_all(By::new().role(Role::Splitter).label("Resize log details"))
            .count(),
        1
    );
    harness
        .state_mut()
        .viewer
        .toggle_inspector(id, InspectorSection::ShowChronology);
    harness.run_steps(3);
    rendering.snapshot_with_color_tolerance("log_viewer_chronology_inspector");
}

#[test]
fn resizing_the_inspector_overrides_the_default_and_restores_each_logs_height() {
    let text = long_log(LONG_LOG_ENTRIES);
    let second = text.replace("entry", "other");
    let mut harness = harness_of(Vec::new(), &[("first.log", &text), ("second.log", &second)]);
    unfold_the_summary_panel(&mut harness);
    let first_height = harness.get_by_label("Resize log details").rect().top();
    let divider = harness.get_by_label("Resize log details").rect().center();
    harness.press_drag_release(divider, egui::vec2(0.0, 65.0), 10);
    harness.run_steps(3);
    let resized = harness.get_by_label("Resize log details").rect().top();
    assert!(
        (resized - first_height - 65.0).abs() < 3.0,
        "{first_height} -> {resized}"
    );
    assert!(
        harness
            .state()
            .viewer
            .active_state()
            .inspector_height
            .is_some()
    );
    harness.get_by_label("first.log").click();
    harness.run_steps(3);
    unfold_the_summary_panel(&mut harness);
    assert!(
        harness
            .state()
            .viewer
            .active_state()
            .inspector_height
            .is_none()
    );
    harness.get_by_label("second.log").click();
    harness.run_steps(3);
    assert!((harness.get_by_label("Resize log details").rect().top() - resized).abs() < 1.0);
}
