use super::*;

impl ViewerState {
    fn apply_chronology_request(&mut self) {
        let request = self.requests.resolve_years.take().unwrap();
        let mut log = self.logs.get_mut_by_id(request.log).unwrap();
        assert!(log.resolve_years_as(request.year_resolution, &self.recordings.view()));
        self.viewer
            .preserve_source_position_after_chronology_change(&log, request.log);
        self.viewer
            .record_chronology_apply_result(request.log, true);
    }
}

fn edit_last_entry_year(harness: &mut Harness<ViewerState>, year: i32) {
    let field =
        By::new().predicate(|node| node.author_id() == Some(unresolved_years::LAST_ENTRY_YEAR_ID));
    harness.get(field.clone()).scroll_to_me();
    harness.run_steps(3);
    harness.get(field).click();
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(year.to_string()));
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
}

#[rstest]
#[case::default(VIEWER_SIZE, "log_viewer_manual_chronology")]
#[case::narrow(NARROW_VIEWPORT, "log_viewer_manual_chronology_narrow")]
#[case::short(SHORT_VIEWPORT, "log_viewer_manual_chronology_short")]
fn chronology_policies_allow_repeated_correction_and_reset_to_unresolved_dates(
    #[case] viewport: egui::Vec2,
    #[case] snapshot: &str,
) {
    let mut rendering = TestHarness::builder().size(viewport).ui_state(
        viewer_ui,
        viewer_state(
            Vec::new(),
            &[("journal.log", gt_test_utils::UNRESOLVED_YEARS_LOG)],
        ),
    );
    let harness = &mut rendering.inner;
    harness.run_steps(3);
    let stored = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    open_date_inspector(harness);
    select_date_policy(harness, unresolved_years::SET_YEAR_LABEL);
    edit_last_entry_year(harness, 2024);
    click_inspector_control(harness, unresolved_years::APPLY_YEAR_LABEL);
    let apply = harness
        .get_by_label(unresolved_years::APPLY_YEAR_LABEL)
        .rect();
    assert!(
        apply
            .y_range()
            .contains(harness.get_by_label("Ignored").rect().center().y)
    );
    harness.state_mut().apply_chronology_request();
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .parsed()
            .year_resolution(),
        YearResolution::PlaceLastEntryInYear(2024)
    );
    let first_range = harness.state().shown_log().unwrap().entry_time_range();
    assert!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .parsed()
            .chronology()
            .is_resolved()
    );
    harness.get_by_label("Dates set");
    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    rendering.snapshot_with_color_tolerance(snapshot);
    let harness = &mut rendering.inner;
    edit_last_entry_year(harness, 2026);
    click_inspector_control(harness, unresolved_years::APPLY_YEAR_LABEL);
    harness.state_mut().apply_chronology_request();
    harness.run_steps(3);
    assert_ne!(
        harness.state().shown_log().unwrap().entry_time_range(),
        first_range
    );
    click_inspector_control(harness, unresolved_years::RESET_LABEL);
    harness.state_mut().apply_chronology_request();
    harness.run_steps(3);
    let log = harness.state().shown_log().unwrap();
    assert_eq!(log.parsed().year_resolution(), YearResolution::Infer);
    assert!(!log.parsed().chronology().is_resolved());
    assert_eq!(log.filters().to_stored_stack(), stored);
    assert_eq!(
        harness.state().viewer.active_state().inspector,
        Some(InspectorSection::ShowChronology)
    );
}

#[test]
fn an_unresolved_manual_choice_keeps_correction_and_conflict_details_available() {
    let mut harness = harness_of(
        Vec::new(),
        &[("leap.log", gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG)],
    );
    open_date_inspector(&mut harness);
    select_date_policy(&mut harness, unresolved_years::SET_YEAR_LABEL);
    edit_last_entry_year(&mut harness, 2025);
    click_inspector_control(&mut harness, unresolved_years::APPLY_YEAR_LABEL);
    harness.state_mut().apply_chronology_request();
    harness.run_steps(3);
    assert!(
        !harness
            .state()
            .shown_log()
            .unwrap()
            .parsed()
            .chronology()
            .is_resolved()
    );
    harness.get_by_label("The date does not exist in 2025");
    harness.get_by_label(unresolved_years::DETAILS_LABEL);
    edit_last_entry_year(&mut harness, 2024);
    click_inspector_control(&mut harness, unresolved_years::APPLY_YEAR_LABEL);
    harness.state_mut().apply_chronology_request();
    assert!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .parsed()
            .chronology()
            .is_resolved()
    );
}

#[rstest]
#[case::space("2026-05-20 12:00:00")]
#[case::iso_t("2026-05-20T12:00:00Z")]
fn explicit_dates_disable_inapplicable_choices_and_allow_existing_policy_reset(
    #[case] timestamp: &str,
    #[values(false, true)] existing_policy: bool,
) {
    let text = format!(
        "{timestamp} kernel: sample\n----------- Journal summary -----------\nLogs begin at: Wed 20-May-2026 12:00:00 UTC\nLogs end at: Wed 20-May-2026 12:00:10 UTC\n"
    );
    let mut state = viewer_state(Vec::new(), &[("explicit.log", &text)]);
    let id = state.first_loaded_log();
    let range = state.shown_log().unwrap().entry_time_range();
    if existing_policy {
        assert!(state.logs.get_mut_by_id(id).unwrap().resolve_years_as(
            YearResolution::PlaceLastEntryInYear(2024),
            &state.recordings.view()
        ));
    }
    let mut harness = harness_from(state);
    open_date_inspector(&mut harness);
    harness.get_by_label("Inapplicable");
    let policy = By::new().predicate(|node| node.author_id() == Some(unresolved_years::POLICY_ID));
    harness.get(policy.clone()).click();
    harness.run_steps(3);
    for label in [
        unresolved_years::SET_YEAR_LABEL,
        unresolved_years::IGNORE_SUMMARY_BOUNDS_LABEL,
    ] {
        assert!(harness.get_by_label(label).accesskit_node().is_disabled());
    }
    let disabled = harness
        .get_by_label(unresolved_years::SET_YEAR_LABEL)
        .rect()
        .center();
    harness.hover_at_and_settle(disabled, TOOLTIP_DELAY_FRAMES);
    harness.get_by_label(unresolved_years::EXPLICIT_YEARS_HOVER);
    harness.get(policy).click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Dates set").is_none());
    if existing_policy {
        assert_eq!(
            harness
                .get(By::new().predicate(
                    |node| node.author_id() == Some(unresolved_years::LAST_ENTRY_YEAR_ID)
                ))
                .accesskit_node()
                .label()
                .as_deref(),
            Some("Year policy")
        );
        assert!(
            harness
                .get(By::new().predicate(
                    |node| node.author_id() == Some(unresolved_years::LAST_ENTRY_YEAR_ID)
                ))
                .accesskit_node()
                .is_disabled()
        );
        click_inspector_control(&mut harness, unresolved_years::RESET_LABEL);
        harness.state_mut().apply_chronology_request();
        harness.run_steps(3);
    }
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .parsed()
            .year_resolution(),
        YearResolution::Infer
    );
    assert_eq!(
        harness.state().shown_log().unwrap().entry_time_range(),
        range
    );
    assert!(harness.query_by_label("Dates set").is_none());
}
