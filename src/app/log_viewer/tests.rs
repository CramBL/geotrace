//! The viewer driven on its own: the rows the table draws for one log, what a
//! row click asks of the map, and the footer's association controls.

use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone as _, Utc};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
use gt_log_view::{FilterChipMode, LayerColorSlot, LoadedLog, LoadedLogs};
use gt_test_utils::{By, HarnessInteraction as _, TestHarness, snapshot_harness};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::{FileSource, Latitude, Longitude};
use gt_ui_types::{HoveredLogGlyph, LogMatchColor, LogMatchHover};

use super::{AssociationWindowUnit, LogViewerContext, LogViewerWindow, filters};

/// One log holding every row kind the table draws: an entry timestamped from
/// its neighbours, a reboot separator, and a backwards timestamp step no clock
/// adjustment explains.
const LOG_WITH_EVERY_ROW_KIND: &str = "\
2026-05-29 18:48:25 navsyncd: starting
  at 0x0000c3f4 in gnss_task+0x54
2026-05-29 18:48:27 navsyncd: fix acquired
--- Device reboot ---
2026-05-29 18:48:30 navsyncd: starting
2026-05-29 18:44:00 navsyncd: telemetry queued
2026-05-29 18:48:40 navsyncd: fix acquired
";

/// A second log sharing none of the first one's messages: switching the
/// selector switches what the filter row shows.
const SECOND_LOG: &str = "\
2026-05-29 18:48:26 hal-powerd: battery low
2026-05-29 18:48:28 hal-powerd: battery critical
";

/// The timestamp column of the log's first entry, as the table writes it: a
/// leading space where an interpolated entry carries its marker.
const FIRST_ENTRY_TIMESTAMP: &str = " 2026-05-29 18:48:25";

/// The second row of the fixture log, whose line carries no timestamp of its
/// own.
const INTERPOLATED_ENTRY_TIMESTAMP: &str = "≈2026-05-29 18:48:26";

/// A log that was never loaded here, standing in for one the viewer is not
/// showing.
const UNLOADED_LOG: gt_ui_types::LoadedLogId = gt_ui_types::LoadedLogId::new(7);

/// The association window a freshly loaded log starts with, matching the app's
/// default.
const ASSOCIATION_WINDOW_SECS: i64 = 60;

/// The window the viewer is driven in, wide enough for the footer's controls
/// to sit on one row.
const VIEWER_SIZE: egui::Vec2 = egui::vec2(760.0, 560.0);

fn log_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 29, 18, 48, 25)
        .single()
        .unwrap_or_default()
}

struct ViewerState {
    viewer: LogViewerWindow,
    logs: LoadedLogs,
    recordings: LoadedFiles,
    map_center: Option<(f64, f64)>,
    log_hover: LogMatchHover,
}

impl ViewerState {
    fn shown_log(&self) -> Option<&LoadedLog> {
        self.logs.get(self.viewer.selected_log_index())
    }
}

/// A recording of ten minutes of walking from `first_lat_deg`, starting the
/// moment the fixture log does.
fn recording(filename: &str, first_lat_deg: f64) -> gt_types::LoadedFile {
    let points = gt_test_utils::nav_points_walking_from(
        log_start(),
        600,
        1,
        Latitude::new(first_lat_deg),
        Longitude::new(12.0),
    );
    gt_track_builder::build_loaded_file(
        filename.to_owned(),
        &points,
        &[],
        Vec::new(),
        Vec::new(),
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from(filename)),
        FileMeta::default(),
        Vec::new(),
    )
}

/// The viewer open on the fixture log, associated against `recordings` when any
/// of them unambiguously covers it.
fn harness_with(recordings: Vec<gt_types::LoadedFile>) -> Harness<'static, ViewerState> {
    harness_of(recordings, &[("navsyncd.log", LOG_WITH_EVERY_ROW_KIND)])
}

/// The viewer open on the last of `logs`, each of them named and parsed from
/// its own text.
fn harness_of(
    recordings: Vec<gt_types::LoadedFile>,
    logs: &[(&str, &str)],
) -> Harness<'static, ViewerState> {
    let mut harness = Harness::builder()
        .with_size(VIEWER_SIZE)
        .build_ui_state(viewer_ui, viewer_state(recordings, logs));
    harness.run_steps(3);
    harness
}

/// [`harness_of`] on a rendering harness, for the test that reads the pixels
/// of the rows back.
fn rendering_harness_with(
    recordings: Vec<gt_types::LoadedFile>,
) -> TestHarness<'static, ViewerState> {
    let mut harness = TestHarness::builder().size(VIEWER_SIZE).ui_state(
        viewer_ui,
        viewer_state(recordings, &[("navsyncd.log", LOG_WITH_EVERY_ROW_KIND)]),
    );
    harness.inner.run_steps(3);
    harness
}

/// One frame of the viewer, as the app draws it.
fn viewer_ui(ui: &mut egui::Ui, state: &mut ViewerState) {
    let names = RecordingNames::resolve(state.recordings.view(), "{filename}");
    state.viewer.show(
        ui.ctx(),
        &mut state.logs,
        LogViewerContext {
            recordings: state.recordings.view(),
            recording_names: &names,
            map_center_request: &mut state.map_center,
            log_hover: &mut state.log_hover,
        },
    );
}

/// The recordings and logs the viewer opens on, the viewer showing the log
/// that loaded last.
fn viewer_state(recordings: Vec<gt_types::LoadedFile>, logs: &[(&str, &str)]) -> ViewerState {
    let mut loaded_recordings = LoadedFiles::new();
    for file in recordings {
        loaded_recordings.push(file, FileHistory::None);
    }
    let mut loaded_logs = LoadedLogs::default();
    for (name, text) in logs {
        let parsed = gt_logfile::parse_log((*text).into(), log_start())
            .unwrap_or_else(|error| panic!("the fixture log parses: {error}"));
        let mut log = LoadedLog::new(
            Some((*name).to_owned()),
            parsed,
            Duration::seconds(ASSOCIATION_WINDOW_SECS),
        );
        let target = log
            .rank_association_candidates(&loaded_recordings.view())
            .unambiguous_target();
        log.associate_with(target, &loaded_recordings.view());
        loaded_logs.push(log);
    }

    let logs = loaded_logs;
    let mut viewer = LogViewerWindow::new();
    viewer.open_on_newly_loaded_log(&logs);

    ViewerState {
        viewer,
        logs,
        recordings: loaded_recordings,
        map_center: None,
        log_hover: LogMatchHover::default(),
    }
}

/// Clicks the table row whose timestamp column reads `timestamp`.
fn click_line(harness: &mut Harness<ViewerState>, timestamp: &str) {
    let row = harness.get_by_label(timestamp).rect().center();
    harness.press_drag_release(row, egui::Vec2::ZERO, 1);
    harness.run_steps(2);
}

/// Parks the cursor on the table row whose timestamp column reads `timestamp`.
fn hover_line(harness: &mut Harness<ViewerState>, timestamp: &str) {
    let row = harness.get_by_label(timestamp).rect().center();
    harness.hover_at_and_settle(row, 2);
}

/// The position the map draws its cross-highlight ring at.
fn ringed_position(harness: &Harness<ViewerState>) -> Option<gt_types::MercPoint> {
    harness.state().log_hover.row_position
}

#[test]
fn the_table_opens_each_boot_with_a_divider_and_marks_an_interpolated_timestamp() {
    let harness = harness_with(Vec::new());

    harness.get_by_label("Boot 1 · up 2s · 3 entries");
    harness.get_by_label("Boot 2 · up 10s · 3 entries");
    // The line between 18:48:25 and 18:48:27 carries no timestamp of its own.
    harness.get_by_label("≈2026-05-29 18:48:26");
}

/// The order-anomaly section is drawn only for a log the parse found one in,
/// naming the line the unexplained backwards step lands on.
#[test]
fn the_summary_panel_lists_the_line_an_unexplained_backwards_step_lands_on() {
    let mut harness = harness_with(Vec::new());
    assert!(
        harness.query_by_label("Order anomalies").is_none(),
        "the summary panel starts folded away"
    );

    let summary = harness
        .state()
        .shown_log()
        .map(LoadedLog::parse_summary_line)
        .unwrap_or_default();
    harness.get_by_label(summary.as_str()).click();
    harness.run_steps(3);

    harness.get_by_label("Order anomalies");
    harness.get_by_label("Line 6");
    harness.get_by_label("steps back 4m");
}

#[test]
fn clicking_a_line_with_a_position_asks_the_map_to_centre_on_it() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| log.associated_entry_count() > 0),
        "the one overlapping recording is the log's association target"
    );

    click_line(&mut harness, FIRST_ENTRY_TIMESTAMP);

    let centred = harness.state().map_center;
    assert!(
        centred.is_some_and(|(lat, _)| (lat - 55.0).abs() < 0.1),
        "the map centres on the line's position, got {centred:?}"
    );
}

#[test]
fn clicking_a_line_with_no_fix_within_the_window_leaves_the_map_where_it_was() {
    let mut harness = harness_with(Vec::new());
    assert_eq!(
        harness
            .state()
            .shown_log()
            .map(LoadedLog::associated_entry_count),
        Some(0),
        "no recording is loaded, so no line has a position"
    );

    click_line(&mut harness, FIRST_ENTRY_TIMESTAMP);

    assert_eq!(harness.state().map_center, None);
}

#[test]
fn hovering_a_line_with_a_position_rings_it_on_the_map() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);

    hover_line(&mut harness, FIRST_ENTRY_TIMESTAMP);

    let position = harness
        .state()
        .shown_log()
        .and_then(|log| log.entry_position(0))
        .map(|(latitude, longitude)| gt_types::mercator::normalize(latitude, longitude));
    assert!(
        position.is_some(),
        "the one overlapping recording gave the line a position"
    );
    assert_eq!(ringed_position(&harness), position);
}

#[test]
fn hovering_a_line_with_no_fix_within_the_window_rings_nothing() {
    let mut harness = harness_with(Vec::new());

    hover_line(&mut harness, FIRST_ENTRY_TIMESTAMP);

    assert_eq!(ringed_position(&harness), None);
}

/// The hexagon the cursor is on over the map marks the rows of the lines it
/// stands for, and leaves the rest of the table as it was.
#[test]
fn a_hovered_hexagon_marks_its_own_lines_in_the_table() {
    let mut harness = rendering_harness_with(vec![recording("walk.gtd", 55.0)]);
    let pixels_per_point = harness.inner.ctx.pixels_per_point();
    let marked_row = harness.inner.get_by_label(FIRST_ENTRY_TIMESTAMP).rect();
    let other_row = harness
        .inner
        .get_by_label(INTERPOLATED_ENTRY_TIMESTAMP)
        .rect();
    let before = harness.inner.render().expect("the harness renders a frame");
    let shown_log = harness
        .state()
        .logs
        .get_with_id(0)
        .map(|(id, _)| id)
        .expect("the fixture log is loaded");

    harness.state_mut().log_hover.glyph = Some(HoveredLogGlyph {
        log: shown_log,
        color: LogMatchColor::LiveFilter,
        entry_indices: vec![0],
    });
    harness.inner.run_steps(2);

    let after = harness.inner.render().expect("the harness renders a frame");
    assert!(
        snapshot_harness::pixels_differ(&before, &after, marked_row, pixels_per_point),
        "the line the hexagon stands for takes a background"
    );
    assert!(
        !snapshot_harness::pixels_differ(&before, &after, other_row, pixels_per_point),
        "a line it does not stand for is left alone"
    );
}

/// A hexagon of a log the viewer is not showing marks nothing: stacks are per
/// log, and the viewer never switches logs on its own.
#[test]
fn a_hovered_hexagon_of_another_log_leaves_the_shown_one_alone() {
    let mut harness = rendering_harness_with(vec![recording("walk.gtd", 55.0)]);
    let pixels_per_point = harness.inner.ctx.pixels_per_point();
    let first_row = harness.inner.get_by_label(FIRST_ENTRY_TIMESTAMP).rect();
    let before = harness.inner.render().expect("the harness renders a frame");

    harness.state_mut().log_hover.glyph = Some(HoveredLogGlyph {
        log: UNLOADED_LOG,
        color: LogMatchColor::LiveFilter,
        entry_indices: vec![0],
    });
    harness.inner.run_steps(2);

    let after = harness.inner.render().expect("the harness renders a frame");
    assert!(!snapshot_harness::pixels_differ(
        &before,
        &after,
        first_row,
        pixels_per_point
    ));
    assert_eq!(
        harness.state().viewer.selected_log_index(),
        0,
        "the viewer stays on the log it was showing"
    );
}

/// The value field and the unit dropdown together name the window a log
/// associates under.
#[test]
fn the_footer_reads_the_association_window_in_the_unit_its_dropdown_shows() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    assert_eq!(
        harness
            .state()
            .shown_log()
            .map(LoadedLog::association_window),
        Some(Duration::seconds(ASSOCIATION_WINDOW_SECS))
    );

    harness
        .get(By::new().value(AssociationWindowUnit::Seconds.label()))
        .click();
    harness.run_steps(2);
    harness
        .get_by_label(AssociationWindowUnit::Minutes.label())
        .click();
    harness.run_steps(2);

    // Clicking the drag value selects its text for typing. egui keeps reporting
    // the field as the spin button while it is edited: the keys go to the
    // focused widget.
    harness
        .get(By::new().role(egui::accesskit::Role::SpinButton))
        .click();
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("2".to_owned()));
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(3);

    assert_eq!(
        harness
            .state()
            .shown_log()
            .map(LoadedLog::association_window),
        Some(Duration::minutes(2)),
        "the value is read as minutes, the unit the dropdown was switched to"
    );
}

#[test]
fn unloading_the_shown_log_leaves_the_viewer_on_its_empty_state() {
    let mut harness = harness_with(Vec::new());

    harness.get_by_label(super::ICON_X).click();
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    harness.get_by_label(super::EMPTY_STATE_HINT);
}

/// Runs until every scan the shown log's filters started has landed: they run
/// on worker threads, and the table draws the matches that have arrived.
fn run_until_the_scans_land(harness: &mut Harness<ViewerState>) {
    // The frame that reads the click or the keystroke is the one that starts
    // the scan: the wait below is only meaningful after it.
    harness.run_steps(1);
    let landed = harness.step_until(|harness| {
        harness
            .state()
            .shown_log()
            .is_some_and(|log| !log.filters().is_query_pending())
    });
    assert!(landed, "the filter scans landed");
    harness.run_steps(2);
}

/// Types `text` into the live filter, then runs the frames its scan needs to
/// land. The field is focused by its own id: the viewer renders further text
/// inputs of its own.
fn type_into_live_filter(harness: &mut Harness<ViewerState>, text: &str) {
    harness.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(filters::LIVE_FILTER_FIELD_ID));
    });
    harness.run_steps(2);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    run_until_the_scans_land(harness);
}

/// The count the filter row shows: the lines the table draws, of the log's
/// entries.
fn match_count(harness: &Harness<ViewerState>) -> String {
    let filters = harness
        .state()
        .shown_log()
        .map(LoadedLog::filters)
        .expect("a log is shown");
    format!(
        "{} of {}",
        filters.visible_entries().len(),
        filters.entry_count()
    )
}

fn live_filter_text(harness: &Harness<ViewerState>) -> String {
    harness
        .state()
        .shown_log()
        .map(|log| log.filters().live_filter_text().to_owned())
        .unwrap_or_default()
}

/// Every chip of the shown log: its text, its mode, and the palette colour it
/// draws in.
fn chips(harness: &Harness<ViewerState>) -> Vec<(String, FilterChipMode, Option<usize>)> {
    harness
        .state()
        .shown_log()
        .map(|log| {
            log.filters()
                .chips()
                .iter()
                .map(|chip| {
                    (
                        chip.pattern().text.clone(),
                        chip.mode(),
                        chip.layer_slot().map(LayerColorSlot::index),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn add_filter(harness: &mut Harness<ViewerState>, text: &str) {
    type_into_live_filter(harness, text);
    harness.get_by_label(filters::ADD_FILTER_LABEL).click();
    run_until_the_scans_land(harness);
}

/// Clicks the mode toggle of the chip at `index`, which carries the glyph of
/// the mode that chip is in.
fn switch_chip_mode(harness: &mut Harness<ViewerState>, index: usize) {
    let modes: Vec<FilterChipMode> = chips(harness).iter().map(|(_, mode, _)| *mode).collect();
    let mode = modes.get(index).copied().expect("the chip is in the stack");
    let glyph = match mode {
        FilterChipMode::Layer => ICON_PLUS_CIRCLE,
        FilterChipMode::Refine => ICON_FUNNEL,
    };
    let among_the_same_mode = modes.iter().take(index).filter(|&&m| m == mode).count();
    harness
        .nth_matching(By::new().label(glyph), among_the_same_mode)
        .click();
    run_until_the_scans_land(harness);
}

/// Clicks the ✕ of the chip at `index`. The selector row's unload button
/// carries the same glyph and comes before every chip.
fn remove_chip(harness: &mut Harness<ViewerState>, index: usize) {
    harness
        .nth_matching(By::new().label(super::ICON_X), index + 1)
        .click();
    run_until_the_scans_land(harness);
}

/// Shows the log named `name`, through the selector the user would use.
fn select_log(harness: &mut Harness<ViewerState>, name: &str) {
    let shown = harness
        .state()
        .shown_log()
        .map(LoadedLog::name)
        .unwrap_or_default()
        .to_owned();
    harness.get(By::new().value(shown.as_str())).click();
    harness.run_steps(2);
    // The popup's row is the lower of the two: the selector above it shows the
    // name it is already on.
    harness.bottommost_matching(By::new().label(name)).click();
    run_until_the_scans_land(harness);
}

#[test]
fn typing_a_filter_leaves_the_table_showing_the_lines_it_matches() {
    let mut harness = harness_with(Vec::new());
    assert_eq!(match_count(&harness), "6 of 6");

    type_into_live_filter(&mut harness, "fix");

    assert_eq!(match_count(&harness), "2 of 6");
    harness.get_by_label("2 of 6");
    assert!(
        harness.query_by_label(FIRST_ENTRY_TIMESTAMP).is_none(),
        "a line the filter misses leaves the table"
    );
    harness.get_by_label("Boot 1 · up 2s · 3 entries");
    harness.get_by_label("Boot 2 · up 10s · 3 entries");
}

/// A boot session the filter leaves nothing of takes its divider with it.
#[test]
fn a_boot_the_filter_empties_loses_its_divider() {
    let mut harness = harness_with(Vec::new());

    type_into_live_filter(&mut harness, "telemetry");

    assert_eq!(match_count(&harness), "1 of 6");
    harness.get_by_label("Boot 2 · up 10s · 3 entries");
    assert!(
        harness
            .query_by_label("Boot 1 · up 2s · 3 entries")
            .is_none(),
        "the first boot has no line the filter matched"
    );
}

/// The terms of a plain filter match in any order. A regex matches the message
/// as one pattern.
#[test]
fn the_regex_toggle_switches_what_the_field_means() {
    let mut harness = harness_with(Vec::new());
    type_into_live_filter(&mut harness, "acquired fix");
    assert_eq!(match_count(&harness), "2 of 6");

    harness.get_by_label(filters::REGEX_TOGGLE_LABEL).click();
    run_until_the_scans_land(&mut harness);

    assert_eq!(
        match_count(&harness),
        "0 of 6",
        "no message holds the terms in the order the regex demands"
    );
    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| log.filters().live_filter_is_regex()),
        "the toggle belongs to the field, and stays on for the next filter"
    );
}

#[test]
fn an_invalid_regex_is_reported_under_the_field_and_leaves_the_table_whole() {
    let mut harness = harness_with(Vec::new());
    harness.get_by_label(filters::REGEX_TOGGLE_LABEL).click();
    harness.run_steps(2);

    type_into_live_filter(&mut harness, "navsyncd(");

    harness.get_by_label_contains("unclosed group");
    assert_eq!(match_count(&harness), "6 of 6");
    harness.get_by_label(FIRST_ENTRY_TIMESTAMP);
    assert!(
        harness
            .get_by_label(filters::ADD_FILTER_LABEL)
            .accesskit_node()
            .is_disabled(),
        "there is nothing to add while the pattern does not compile"
    );
}

#[test]
fn adding_a_filter_grays_out_while_the_field_is_empty() {
    let mut harness = harness_with(Vec::new());
    assert!(
        harness
            .get_by_label(filters::ADD_FILTER_LABEL)
            .accesskit_node()
            .is_disabled()
    );

    type_into_live_filter(&mut harness, "fix");

    assert!(
        !harness
            .get_by_label(filters::ADD_FILTER_LABEL)
            .accesskit_node()
            .is_disabled(),
        "a written filter can become a chip"
    );
}

#[test]
fn adding_a_filter_turns_it_into_a_chip_and_empties_the_field() {
    let mut harness = harness_with(Vec::new());

    add_filter(&mut harness, "fix");

    assert_eq!(
        chips(&harness),
        [("fix".to_owned(), FilterChipMode::Layer, Some(0))]
    );
    assert_eq!(live_filter_text(&harness), "");
    harness.get_by_label("fix");
}

/// The compare-phenomena-spatially mode: a layer chip colours its lines and
/// leaves the table whole, a refine chip narrows it.
#[test]
fn a_layer_chip_leaves_the_table_whole_and_a_refine_chip_narrows_it() {
    let mut harness = harness_with(Vec::new());
    add_filter(&mut harness, "fix");
    assert_eq!(match_count(&harness), "6 of 6");

    switch_chip_mode(&mut harness, 0);

    assert_eq!(match_count(&harness), "2 of 6");
    assert_eq!(
        chips(&harness),
        [("fix".to_owned(), FilterChipMode::Refine, None)],
        "a refine chip hands back the colour it drew in"
    );

    switch_chip_mode(&mut harness, 0);

    assert_eq!(match_count(&harness), "6 of 6");
    assert_eq!(
        chips(&harness),
        [("fix".to_owned(), FilterChipMode::Layer, Some(0))]
    );
}

#[test]
fn unticking_a_chip_takes_it_out_of_the_table_and_ticking_it_puts_it_back() {
    let mut harness = harness_with(Vec::new());
    add_filter(&mut harness, "fix");
    switch_chip_mode(&mut harness, 0);
    assert_eq!(match_count(&harness), "2 of 6");

    harness.get(By::new().role(Role::CheckBox)).click();
    run_until_the_scans_land(&mut harness);
    assert_eq!(match_count(&harness), "6 of 6");

    harness.get(By::new().role(Role::CheckBox)).click();
    run_until_the_scans_land(&mut harness);
    assert_eq!(
        match_count(&harness),
        "2 of 6",
        "a chip ticked again filters as it did before"
    );
}

#[test]
fn removing_a_chip_frees_the_colour_it_drew_in() {
    let mut harness = harness_with(Vec::new());
    add_filter(&mut harness, "fix");
    add_filter(&mut harness, "starting");

    remove_chip(&mut harness, 0);
    add_filter(&mut harness, "telemetry");

    assert_eq!(
        chips(&harness),
        [
            ("starting".to_owned(), FilterChipMode::Layer, Some(1)),
            ("telemetry".to_owned(), FilterChipMode::Layer, Some(0)),
        ],
        "the freed colour is the lowest one free again"
    );
}

/// Each log keeps the filter it was written for: the field switches with the
/// selector.
#[test]
fn the_live_filter_belongs_to_the_log_it_was_written_for() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("navsyncd.log", LOG_WITH_EVERY_ROW_KIND),
            ("hal-powerd.log", SECOND_LOG),
        ],
    );
    assert_eq!(
        harness.state().viewer.selected_log_index(),
        1,
        "the viewer opens on the log that loaded last"
    );

    type_into_live_filter(&mut harness, "battery low");
    assert_eq!(match_count(&harness), "1 of 2");

    select_log(&mut harness, "navsyncd.log");

    assert_eq!(match_count(&harness), "6 of 6");
    assert_eq!(
        live_filter_text(&harness),
        "",
        "the log that was filtered is the one holding the filter"
    );

    select_log(&mut harness, "hal-powerd.log");

    assert_eq!(match_count(&harness), "1 of 2");
    assert_eq!(live_filter_text(&harness), "battery low");
}
