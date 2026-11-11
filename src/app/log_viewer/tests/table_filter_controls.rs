use gt_log_view::TableFilterApplication;

use super::*;

#[test]
fn empty_results_offer_pause_and_clear_with_structural_and_revealed_rows() {
    let mut harness = harness_of(Vec::new(), &[("source.log", SOURCE_LOG)]);
    super::set_display_option(&mut harness, "Show structural source lines", true);
    type_into_live_filter(&mut harness, "no matching entry");
    harness.get_by_label(filters::NO_MATCHES_LABEL);
    harness.get_by_label("--- Device reboot ---");
    let shown = harness.state().first_loaded_log();
    let state = harness.state_mut();
    state.viewer.navigate_to_diagnostic(
        state.logs.get_by_id(shown).unwrap(),
        shown,
        line_table::DiagnosticTarget::Entry(0),
    );
    harness.run_steps(2);
    harness.get_by_label("navsyncd: first");
    harness.get_by_label(filters::NO_MATCHES_LABEL);
    harness
        .get_by_label(filters::PAUSE_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    assert_eq!(match_count(&harness), "3 of 3");
    harness.get_by_label(filters::TABLE_FILTERS_PAUSED_LABEL);
    assert!(harness.query_by_label(filters::NO_MATCHES_LABEL).is_none());
    harness
        .get_by_label(filters::RESUME_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    assert_eq!(match_count(&harness), "0 of 3");
    harness
        .get_by_label(filters::CLEAR_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    assert_eq!(match_count(&harness), "3 of 3");
    assert!(live_filter_text(&harness).is_empty());
    assert!(harness.state().viewer.diagnostic_reveal.is_none());
}

#[test]
fn pausing_and_clearing_one_log_preserves_other_logs_and_map_drafts() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("first.log", BOOLEAN_FILTER_LOG),
            ("second.log", SECOND_LOG),
        ],
    );
    select_log(&mut harness, "first.log");
    add_filter(&mut harness, "alpha");
    add_map_highlight(&mut harness, "beta");
    type_into_map_draft(&mut harness, "gamma");
    let stored = harness
        .state()
        .shown_log()
        .unwrap()
        .filters()
        .to_stored_stack();
    harness
        .get_by_label(filters::PAUSE_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    assert_eq!(match_count(&harness), "4 of 4");
    assert_eq!(
        harness
            .state()
            .shown_log()
            .unwrap()
            .filters()
            .to_stored_stack(),
        stored
    );
    select_log(&mut harness, "second.log");
    harness.get_by_label(filters::PAUSE_TABLE_FILTERS_LABEL);
    assert_eq!(match_count(&harness), "2 of 2");
    select_log(&mut harness, "first.log");
    harness.get_by_label(filters::TABLE_FILTERS_PAUSED_LABEL);
    harness
        .get_by_label(filters::CLEAR_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    let filters = harness.state().shown_log().unwrap().filters();
    assert_eq!(filters.live_filter_text(), "gamma");
    assert_eq!(filters.live_filter_effect(), FilterEffect::Map);
    assert_eq!(filters.chips().len(), 1);
    assert!(
        filters
            .chips()
            .first()
            .unwrap()
            .has_effect(FilterEffect::Map)
    );
    assert!(
        !filters
            .chips()
            .first()
            .unwrap()
            .has_effect(FilterEffect::Table)
    );
    assert_eq!(filters.table_application(), TableFilterApplication::Paused);
    select_log(&mut harness, "second.log");
    assert_eq!(match_count(&harness), "2 of 2");
}

#[test]
fn editing_a_paused_table_keeps_it_paused_until_resume() {
    let mut harness = harness_of(Vec::new(), &[("conditions.log", BOOLEAN_FILTER_LOG)]);
    add_filter(&mut harness, "alpha");
    harness
        .get_by_label(filters::PAUSE_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    type_into_live_filter(&mut harness, "beta");
    assert_eq!(match_count(&harness), "4 of 4");
    harness.get_by_label(filters::TABLE_FILTERS_PAUSED_LABEL);
    harness
        .get_by_label(filters::RESUME_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    assert_eq!(match_count(&harness), "1 of 4");
    assert_eq!(live_filter_text(&harness), "beta");
    harness
        .get_by_label(filters::CLEAR_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    assert_eq!(match_count(&harness), "4 of 4");
    assert!(live_filter_text(&harness).is_empty());
}

#[test]
fn invalid_table_input_is_distinct_from_finished_empty_results() {
    let mut harness = harness_of(Vec::new(), &[("conditions.log", BOOLEAN_FILTER_LOG)]);
    add_filter(&mut harness, "missing");
    open_live_editor(&mut harness);
    harness.get_by_label(filters::REGEX_TOGGLE_LABEL).click();
    harness.run_steps(2);
    type_into_live_filter(&mut harness, "[");
    harness.get_by_label("Fix the invalid table condition");
    assert!(harness.query_by_label(filters::NO_MATCHES_LABEL).is_none());
}

struct SourceReadingPosition {
    entry_index: usize,
    offset_px: f32,
    viewport: egui::Rect,
}

impl SourceReadingPosition {
    fn of(harness: &Harness<ViewerState>) -> Self {
        harness
            .output()
            .shapes
            .iter()
            .find_map(|shape| {
                let egui::Shape::Text(text) = &shape.shape else {
                    return None;
                };
                let index = text
                    .galley
                    .job
                    .text
                    .strip_prefix("navsyncd: entry ")?
                    .parse()
                    .ok()?;
                let bounds = text.visual_bounding_rect();
                bounds.intersects(shape.clip_rect).then_some(Self {
                    entry_index: index,
                    offset_px: bounds.top() - shape.clip_rect.top(),
                    viewport: shape.clip_rect,
                })
            })
            .expect("a source message intersects the table viewport")
    }

    fn assert_retained(&self, harness: &Harness<ViewerState>) {
        let shown = Self::of(harness);
        assert_eq!(shown.entry_index, self.entry_index);
        assert!(
            (shown.offset_px - self.offset_px).abs() < SCROLL_READING_TOLERANCE_PX,
            "source offset {} -> {}",
            self.offset_px,
            shown.offset_px
        );
    }
}

#[rstest]
#[case::pause_resume(filters::PAUSE_TABLE_FILTERS_LABEL)]
#[case::clear(filters::CLEAR_TABLE_FILTERS_LABEL)]
fn changing_table_application_preserves_the_source_reading_position(
    #[case] action: &str,
    #[values(false, true)] near_end: bool,
) {
    let text = long_log(LONG_LOG_ENTRIES);
    let mut harness = harness_of(Vec::new(), &[("long.log", &text)]);
    type_into_live_filter(&mut harness, "entry 1");
    harness.state_mut().viewer.scroll_to_row = Some(if near_end {
        table_row_count(&harness)
    } else {
        5
    });
    harness.run_steps(2);
    let table = SourceReadingPosition::of(&harness).viewport.center();
    harness.scroll_wheel_at(
        table,
        if near_end {
            READING_WHEEL_STEP_PX
        } else {
            -READING_WHEEL_STEP_PX
        },
        WHEEL_SETTLE_FRAMES,
    );
    let position = SourceReadingPosition::of(&harness);
    harness.get_by_label(action).click();
    harness.run_steps(2);
    position.assert_retained(&harness);
    if action == filters::PAUSE_TABLE_FILTERS_LABEL {
        harness
            .get_by_label(filters::RESUME_TABLE_FILTERS_LABEL)
            .click();
        harness.run_steps(2);
        position.assert_retained(&harness);
    }
}

#[rstest]
#[case::boot_divider(0)]
#[case::day_divider(1)]
fn pausing_an_unchanged_table_preserves_the_entry_below_a_divider(#[case] entry_index: usize) {
    let mut text = long_log(LONG_LOG_ENTRIES);
    if entry_index == 1 {
        text = text.replacen("2026-05-29 18:48:25", "2026-05-28 23:59:59", 1);
    }
    let mut harness = harness_of(Vec::new(), &[("dividers.log", &text)]);
    let row = line_table::LineTableRows::of(harness.state().shown_log().unwrap())
        .row_of_exact_entry(entry_index)
        .unwrap();
    harness.state_mut().viewer.scroll_to_row = Some(row);
    harness.run_steps(2);
    let position = SourceReadingPosition::of(&harness);
    assert_eq!(position.entry_index, entry_index);
    harness
        .get_by_label(filters::PAUSE_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    position.assert_retained(&harness);
    harness
        .get_by_label(filters::RESUME_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    position.assert_retained(&harness);
}

#[test]
fn pause_and_resume_preserve_a_midnight_reboot_entry_after_a_hidden_first_entry() {
    let start = Utc
        .with_ymd_and_hms(2026, 5, 29, 23, 59, 58)
        .single()
        .unwrap();
    let text: String = (0..126)
        .map(|index| {
            let separator = if index == 1 {
                "--- Device reboot ---\n"
            } else {
                ""
            };
            let message = if index == 1 {
                "hidden".to_owned()
            } else {
                format!("entry {index}")
            };
            format!(
                "{separator}{} navsyncd: {message}\n",
                (start + Duration::seconds(i64::from(index) * 2))
                    .format(super::super::TIMESTAMP_FORMAT)
            )
        })
        .collect();
    let mut harness = harness_of(Vec::new(), &[("midnight.log", &text)]);
    type_into_live_filter(&mut harness, "entry");
    assert!(!harness.state().viewer.show_structural_lines);
    let rows = line_table::LineTableRows::of(harness.state().shown_log().unwrap());
    assert_eq!(rows.len(), 128);
    harness.state_mut().viewer.scroll_to_row = rows.row_of_exact_entry(2);
    harness.run_steps(2);
    let position = SourceReadingPosition::of(&harness);
    assert_eq!(position.entry_index, 2);
    harness
        .get_by_label(filters::PAUSE_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    position.assert_retained(&harness);
    harness
        .get_by_label(filters::RESUME_TABLE_FILTERS_LABEL)
        .click();
    harness.run_steps(2);
    position.assert_retained(&harness);
}

#[rstest]
#[case::empty(false)]
#[case::paused(true)]
fn table_filter_recovery_snapshot(#[case] paused: bool) {
    let mut harness = rendering_harness_of(Vec::new(), &[("conditions.log", BOOLEAN_FILTER_LOG)]);
    type_into_live_filter(&mut harness.inner, "missing");
    if paused {
        harness
            .inner
            .get_by_label(filters::PAUSE_TABLE_FILTERS_LABEL)
            .click();
        harness.inner.run_steps(2);
    }
    harness.snapshot(if paused {
        "log_viewer_filters_paused"
    } else {
        "log_viewer_filters_empty"
    });
}

const READING_WHEEL_STEP_PX: f32 = 7.0;
