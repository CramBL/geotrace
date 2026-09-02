//! The viewer driven on its own: the rows the table draws for one log, what a
//! row click asks of the map, and the footer's association controls.

use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone as _, Utc};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::PAPERCLIP as ICON_PAPERCLIP;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
use gt_log_view::{
    FilterChipMode, LayerColorSlot, LoadedLog, LoadedLogs, LogAttachmentRef, SessionLogAttachments,
};
use gt_pending_writes::WriteAccess;
use rstest::rstest;

use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use gt_test_utils::window_fit::{
    CRAMPED_VIEWPORT, NARROW_VIEWPORT, OVERSIZED_ROW_COUNT, SHORT_VIEWPORT,
};
use gt_test_utils::{
    AuditedWindow, By, ControlLabel, HarnessInteraction as _, TestHarness,
    WindowFitAssertions as _, snapshot_harness,
};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::{FileSource, Latitude, Longitude};
use gt_ui_types::{LoadedLogId, LogMatchColor, LogMatchGlyph, LogMatchHover};

use super::{
    AssociationWindowUnit, LOG_VIEWER_TITLE, LogViewerContext, LogViewerRequests, LogViewerWindow,
    filters, line_table, log_list,
};

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
/// selected row switches what the filter row shows.
const SECOND_LOG: &str = "\
2026-05-29 18:48:26 hal-powerd: battery low
2026-05-29 18:48:28 hal-powerd: battery critical
";

/// A third log, for the group of logs that take their positions from no
/// recording.
const THIRD_LOG: &str = "\
2026-05-29 18:48:29 kernel: usb 1-1 disconnect
";

/// The timestamp column of the log's first entry, as the table writes it: a
/// leading space where an interpolated entry carries its marker.
const FIRST_ENTRY_TIMESTAMP: &str = " 2026-05-29 18:48:25";

/// The message of the log's first entry, which its last boot repeats.
const FIRST_ENTRY_MESSAGE: &str = "navsyncd: starting";

/// The format the parse read the fixture log in, as the summary panel names
/// it.
const FIXTURE_LOG_FORMAT: &str = "ISO 8601";

/// The second row of the fixture log, whose line carries no timestamp of its
/// own.
const INTERPOLATED_ENTRY_TIMESTAMP: &str = "≈2026-05-29 18:48:26";

/// A log that was never loaded here, standing in for one the viewer is not
/// showing.
const UNLOADED_LOG: LoadedLogId = LoadedLogId::new(7);

/// The association window a freshly loaded log starts with, matching the app's
/// default.
const ASSOCIATION_WINDOW_SECS: i64 = 60;

/// Frames the cursor rests on a row before egui opens its hover text: the
/// harness clock ticks a quarter second per frame, past the tooltip delay.
const TOOLTIP_DELAY_FRAMES: usize = 3;

/// The window the viewer is driven in, wide enough for the footer's controls
/// to sit on one row.
const VIEWER_SIZE: egui::Vec2 = egui::vec2(760.0, 560.0);

/// A scroll to one of [`long_log`]'s entries leaves the head of the log off
/// screen: the log has more lines than the table draws at once.
const LONG_LOG_ENTRIES: usize = 200;

/// The first entry of [`long_log`] the map's clicked hexagon groups.
const CLICKED_ENTRY: usize = 120;

fn log_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 29, 18, 48, 25)
        .single()
        .unwrap_or_default()
}

struct ViewerState {
    viewer: LogViewerWindow,
    logs: LoadedLogs,
    recordings: LoadedFiles,
    /// What the loaded recordings hold in history, as a recording load lists
    /// it.
    attachments: SessionLogAttachments,
    map_center: Option<(f64, f64)>,
    log_hover: LogMatchHover,
    /// The hexagon the map was clicked on, which the viewer opens on.
    clicked_glyph: Option<LogMatchGlyph>,
    requests: LogViewerRequests,
    /// What the session may write, which is what grays the attachment
    /// controls.
    write_access: WriteAccess,
    /// Whether the recordings database is open, which is what grays the
    /// footer's "Load recording".
    history_available: bool,
}

impl ViewerState {
    fn shown_log(&self) -> Option<&LoadedLog> {
        self.viewer
            .selected_log()
            .and_then(|id| self.logs.get_by_id(id))
    }

    /// The log that loaded first, which every single-log fixture is.
    fn first_loaded_log(&self) -> LoadedLogId {
        self.logs.first_id().expect("a fixture log is loaded")
    }
}

/// A log of `count` entries, one per second from [`log_start`].
fn long_log(count: usize) -> String {
    (0..count)
        .map(|second| {
            format!(
                "{} navsyncd: entry {second}\n",
                (log_start() + Duration::seconds(second as i64)).format(super::TIMESTAMP_FORMAT)
            )
        })
        .collect()
}

/// The timestamp column of entry `index` of [`long_log`], as the table writes
/// it: an anchored timestamp is prefixed with a space.
fn long_log_entry_timestamp(index: usize) -> String {
    format!(
        " {}",
        (log_start() + Duration::seconds(index as i64)).format(super::TIMESTAMP_FORMAT)
    )
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
    harness_from(viewer_state(recordings, logs))
}

fn harness_from(state: ViewerState) -> Harness<'static, ViewerState> {
    let mut harness = Harness::builder()
        .with_size(VIEWER_SIZE)
        .build_ui_state(viewer_ui, state);
    // Matches the app's context setup: the clickable rows depend on it.
    gt_ui_theme::install_app_style(&harness.ctx);
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
            write_access: state.write_access,
            recordings: state.recordings.view(),
            recording_names: &names,
            attachments: &state.attachments,
            map_center_request: &mut state.map_center,
            log_hover: &mut state.log_hover,
            clicked_glyph: &mut state.clicked_glyph,
            requests: &mut state.requests,
            history_available: state.history_available,
            dialog_open: false,
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
    let mut last_loaded = None;
    for (name, text) in logs {
        let parsed = gt_logfile::parse_log((*text).into(), log_start())
            .unwrap_or_else(|error| panic!("the fixture log parses: {error}"));
        let mut log = LoadedLog::new(
            Some((*name).to_owned()),
            parsed,
            Duration::seconds(ASSOCIATION_WINDOW_SECS),
        );
        let recordings = loaded_recordings.view();
        let unambiguous = log
            .rank_association_candidates(&recordings)
            .unambiguous_target();
        log.anchor_to_loaded_recording(unambiguous, &recordings);
        last_loaded = Some(loaded_logs.push(log).id());
    }

    let logs = loaded_logs;
    let mut viewer = LogViewerWindow::new();
    if let Some(id) = last_loaded {
        viewer.open_on_log(id);
    }

    ViewerState {
        viewer,
        logs,
        recordings: loaded_recordings,
        attachments: SessionLogAttachments::default(),
        map_center: None,
        log_hover: LogMatchHover::default(),
        clicked_glyph: None,
        requests: LogViewerRequests::default(),
        write_access: WriteAccess::Owner,
        history_available: true,
    }
}

/// Clicks the table row whose timestamp column reads `timestamp`.
fn click_line(harness: &mut Harness<ViewerState>, timestamp: &str) {
    let row = harness.get_by_label(timestamp).rect().center();
    harness.press_drag_release(row, egui::Vec2::ZERO, 1);
    harness.run_steps(2);
}

/// Parks the cursor on the table row whose timestamp column reads `timestamp`,
/// long enough for the hover texts of that row to open.
fn hover_line(harness: &mut Harness<ViewerState>, timestamp: &str) {
    let row = harness.get_by_label(timestamp).rect().center();
    harness.hover_at_and_settle(row, TOOLTIP_DELAY_FRAMES);
}

/// A recording in the history database, standing in for one this session's
/// log was stored with.
fn attachment_ref() -> gt_log_view::LogAttachmentRef {
    gt_log_view::LogAttachmentRef {
        recording: gt_store::DatabaseRef {
            identity: "nav-devkit-mk2".to_owned(),
            group_name: "2026-05-29T18-48-25".to_owned(),
        },
        id: gt_store::LogAttachmentId::new_random(),
    }
}

/// Stores the shown log with the recording [`attachment_ref`] identifies, as
/// the app notes it once the database has written it.
fn attach_the_shown_log(harness: &mut Harness<ViewerState>) {
    let shown = harness.state().first_loaded_log();
    let state = harness.state_mut();
    let recordings = state.recordings.view();
    if let Some(log) = state.logs.get_mut_by_id(shown) {
        log.record_attachment(attachment_ref(), Vec::new(), &recordings);
    }
    harness.run_steps(2);
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

    unfold_the_summary_panel(&mut harness);

    harness.get_by_label("Order anomalies");
    harness.get_by_label("Line 6");
    harness.get_by_label("steps back 4m");
}

/// A log no exporter wrote a summary block for has no service table to show:
/// the panel is the figures the parse derived.
#[test]
fn the_summary_panel_of_a_log_without_an_exporter_block_shows_the_derived_figures_alone() {
    let mut harness = harness_with(Vec::new());

    unfold_the_summary_panel(&mut harness);

    harness.get_by_label("Format");
    harness.get_by_label("Boots");
    assert!(
        harness.query_by_label("Service summary").is_none(),
        "nothing in this log states a service table"
    );
}

/// Clicks the parse summary, which unfolds the panel beneath it.
fn unfold_the_summary_panel(harness: &mut Harness<ViewerState>) {
    let summary = harness
        .state()
        .shown_log()
        .map(LoadedLog::parse_summary_line)
        .unwrap_or_default();
    harness.get_by_label(summary.as_str()).click();
    harness.run_steps(3);
}

#[test]
fn clicking_a_line_with_a_position_asks_the_map_to_centre_on_it() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| log.associated_entry_count() > 0),
        "the log is anchored to the one overlapping recording"
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

/// A drag across a line selects its text for the clipboard, and the row it
/// crossed reports no click.
#[test]
fn dragging_across_a_line_copies_its_text_and_leaves_the_map_where_it_was() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    let timestamp = harness.get_by_label(FIRST_ENTRY_TIMESTAMP).rect();
    let message = harness
        .topmost_matching(By::new().label(FIRST_ENTRY_MESSAGE))
        .rect();

    let from = timestamp.left_center() + egui::vec2(2.0, 0.0);
    let to = message.right_center() - egui::vec2(2.0, 0.0);
    harness.press_drag_release(from, to - from, 4);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();

    // The assertion is on the words the copy states: egui spaces one copied
    // galley from the next by how far apart they sat.
    let copied = copied_text(&harness);
    let words: Vec<&str> = copied.split_whitespace().collect();
    assert_eq!(words, ["2026-05-29", "18:48:25", "navsyncd:", "starting"]);
    assert_eq!(
        harness.state().map_center,
        None,
        "the drag selects text: the row reports no click"
    );
}

/// The figures the parse derived are quoted elsewhere: their values select
/// and copy.
#[test]
fn dragging_across_a_parse_figure_copies_its_value() {
    let mut harness = harness_with(Vec::new());
    unfold_the_summary_panel(&mut harness);
    let format = harness.get_by_label(FIXTURE_LOG_FORMAT).rect();

    let from = format.left_center();
    let to = format.right_center();
    harness.press_drag_release(from, to - from, 4);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.step();

    assert_eq!(copied_text(&harness).trim(), FIXTURE_LOG_FORMAT);
}

/// The text the viewer last put on the clipboard.
fn copied_text(harness: &Harness<ViewerState>) -> String {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            egui::OutputCommand::OpenUrl(_) | egui::OutputCommand::CopyImage(_) => None,
        })
        .expect("the drag selected text to copy")
}

#[test]
fn hovering_a_line_with_a_position_rings_it_on_the_map() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);

    hover_line(&mut harness, FIRST_ENTRY_TIMESTAMP);

    let position = harness
        .state()
        .shown_log()
        .and_then(|log| log.entry_placement(0))
        .map(|placement| {
            let (latitude, longitude) = placement.position;
            gt_types::mercator::normalize(latitude, longitude)
        });
    assert!(
        position.is_some(),
        "the one overlapping recording gave the line a position"
    );
    assert_eq!(ringed_position(&harness), position);
}

/// A selectable label senses the pointer, and both hover texts of a line still
/// reach the reader over its text: the row's own, and the one an interpolated
/// timestamp carries.
#[rstest]
#[case::row(FIRST_ENTRY_TIMESTAMP, line_table::ASSOCIATED_ROW_HOVER)]
#[case::interpolated_timestamp(
    INTERPOLATED_ENTRY_TIMESTAMP,
    line_table::INTERPOLATED_TIMESTAMP_HOVER
)]
fn hovering_the_text_of_a_line_shows_its_hover_text(
    #[case] timestamp: &str,
    #[case] expected: &str,
) {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);

    hover_line(&mut harness, timestamp);

    harness.get_by_label(expected);
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
    let shown_log = harness.state().first_loaded_log();

    harness.state_mut().log_hover.glyph = Some(LogMatchGlyph {
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

    harness.state_mut().log_hover.glyph = Some(LogMatchGlyph {
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
        harness.state().viewer.selected_log(),
        Some(harness.state().first_loaded_log()),
        "the viewer stays on the log it was showing"
    );
}

/// A click on a hexagon of a log the viewer is not showing switches to that
/// log, opens the window, and scrolls the table to that hexagon's first line.
#[test]
fn clicking_a_hexagon_shows_its_log_at_the_first_line_it_groups() {
    let long = long_log(LONG_LOG_ENTRIES);
    let mut harness = harness_of(
        vec![recording("walk.gtd", 55.0)],
        &[
            ("navsyncd.log", long.as_str()),
            ("hal-powerd.log", SECOND_LOG),
        ],
    );
    let clicked_log = harness.state().first_loaded_log();
    harness.state_mut().viewer.open = false;
    harness.run_steps(2);

    harness.state_mut().clicked_glyph = Some(LogMatchGlyph {
        log: clicked_log,
        color: LogMatchColor::LiveFilter,
        entry_indices: vec![CLICKED_ENTRY, CLICKED_ENTRY + 1],
    });
    harness.run_steps(3);

    assert_eq!(harness.state().viewer.selected_log(), Some(clicked_log));
    assert!(harness.state().viewer.open, "the click opens the window");
    harness.get_by_label(long_log_entry_timestamp(CLICKED_ENTRY).as_str());
    assert!(
        harness.query_by_label(FIRST_ENTRY_TIMESTAMP).is_none(),
        "the table scrolled away from the head of the log"
    );
}

/// A hexagon of a log that unloaded before the viewer read the click leaves
/// the window closed and the selection where it was.
#[test]
fn a_click_on_a_hexagon_of_an_unloaded_log_leaves_the_window_closed() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    let shown = harness.state().first_loaded_log();
    harness.state_mut().viewer.open = false;
    harness.run_steps(2);

    harness.state_mut().clicked_glyph = Some(LogMatchGlyph {
        log: UNLOADED_LOG,
        color: LogMatchColor::LiveFilter,
        entry_indices: vec![0],
    });
    harness.run_steps(3);

    assert!(!harness.state().viewer.open);
    assert_eq!(harness.state().viewer.selected_log(), Some(shown));
}

/// The lines of the clicked hexagon stay marked once the cursor has left the
/// map, which is where the reader looks for them.
#[test]
fn the_lines_of_a_clicked_hexagon_stay_marked_in_the_table() {
    let mut harness = rendering_harness_with(vec![recording("walk.gtd", 55.0)]);
    let pixels_per_point = harness.inner.ctx.pixels_per_point();
    let marked_row = harness.inner.get_by_label(FIRST_ENTRY_TIMESTAMP).rect();
    let other_row = harness
        .inner
        .get_by_label(INTERPOLATED_ENTRY_TIMESTAMP)
        .rect();
    let before = harness.inner.render().expect("the harness renders a frame");
    let shown_log = harness.state().first_loaded_log();

    harness.state_mut().clicked_glyph = Some(LogMatchGlyph {
        log: shown_log,
        color: LogMatchColor::LiveFilter,
        entry_indices: vec![0],
    });
    harness.inner.run_steps(2);

    let after = harness.inner.render().expect("the harness renders a frame");
    assert!(
        snapshot_harness::pixels_differ(&before, &after, marked_row, pixels_per_point),
        "the clicked hexagon's line takes a background"
    );
    assert!(
        !snapshot_harness::pixels_differ(&before, &after, other_row, pixels_per_point),
        "a line the clicked hexagon does not group is left alone"
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

/// The manual path to the association dialog, which the app opens on the log
/// the footer names.
#[test]
fn the_footer_requests_the_association_dialog_for_the_shown_log() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    let shown = harness.state().first_loaded_log();

    harness.get_by_label(super::ATTACH_LABEL).click();
    harness.run_steps(2);

    assert_eq!(
        harness.state().requests.open_association_dialog,
        Some(shown)
    );
}

/// The attachment the viewer shows and takes off: the indicator names the
/// recording holding the log, and "Remove attachment" acts only on a log that
/// has one.
#[test]
fn an_attachment_is_shown_and_removable_only_while_the_log_has_one() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    assert!(
        harness
            .get_by_label(super::DETACH_LABEL)
            .accesskit_node()
            .is_disabled(),
        "a log stored nowhere has no attachment to remove"
    );
    assert!(harness.query_by_label(ICON_PAPERCLIP).is_none());

    let shown = harness.state().first_loaded_log();
    attach_the_shown_log(&mut harness);

    harness.get_by_label(ICON_PAPERCLIP);
    harness.get_by_label(super::DETACH_LABEL).click();
    harness.run_steps(2);

    assert_eq!(harness.state().requests.detach, Some(shown));
}

/// The recording an attached log is stored with is the recording it takes its
/// positions from: the footer offers "no recording" only once the attachment is
/// removed.
#[test]
fn the_footer_takes_no_recording_off_an_attached_log() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    attach_the_shown_log(&mut harness);

    harness.get(By::new().value(gt_ui_theme::EM_DASH)).click();
    harness.run_steps(2);
    let no_recording = harness.bottommost_matching(By::new().label(gt_ui_theme::EM_DASH));
    let position = no_recording.rect().center();

    assert!(no_recording.accesskit_node().is_disabled());
    harness.hover_at_and_settle(position, TOOLTIP_DELAY_FRAMES);
    harness.get_by_label_contains(super::NO_RECORDING_ATTACHED_HOVER);
    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| log.anchor_key().is_some()),
        "the log stays anchored to the recording it is stored with"
    );
}

/// Attaching a log to a recording and taking it back out both write to the
/// recording history: a read-only session offers neither, and says why.
#[test]
fn the_attachment_controls_are_grayed_in_a_read_only_session() {
    let mut harness = harness_with(vec![recording("walk.gtd", 55.0)]);
    attach_the_shown_log(&mut harness);
    harness.state_mut().write_access = WriteAccess::ReadOnly;
    harness.run_steps(2);

    let attach = harness.get_by_label(super::ATTACH_LABEL);
    assert!(attach.accesskit_node().is_disabled());
    let attach_center = attach.rect().center();
    assert!(
        harness
            .get_by_label(super::DETACH_LABEL)
            .accesskit_node()
            .is_disabled(),
        "a read-only session takes no attachment out of the recording history"
    );

    harness.hover_at_and_settle(attach_center, 3);

    harness.get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);
}

#[test]
fn unloading_the_shown_log_leaves_the_viewer_on_its_empty_state() {
    let mut harness = harness_with(Vec::new());

    harness.get_by_label(super::ICON_X).click();
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    harness.get_by_label(super::LOG_LOAD_HINT);
}

/// The viewer follows the log it was showing, wherever that log sits among the
/// loaded ones.
#[test]
fn unloading_another_log_leaves_the_viewer_on_the_one_it_shows() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("navsyncd.log", LOG_WITH_EVERY_ROW_KIND),
            ("hal-powerd.log", SECOND_LOG),
        ],
    );
    let shown = harness.state().viewer.selected_log();
    let other = harness.state().first_loaded_log();

    harness.state_mut().logs.remove_by_id(other);
    harness.run_steps(3);

    assert_eq!(harness.state().viewer.selected_log(), shown);
    assert_eq!(
        harness.state().shown_log().map(LoadedLog::name),
        Some("hal-powerd.log")
    );
}

#[test]
fn unloading_the_shown_log_leaves_the_viewer_on_the_log_that_loaded_first() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("navsyncd.log", LOG_WITH_EVERY_ROW_KIND),
            ("hal-powerd.log", SECOND_LOG),
        ],
    );

    // The second row is the log that loaded last, which the viewer opened on.
    harness
        .nth_matching(By::new().label(super::ICON_X), 1)
        .click();
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(
        harness.state().shown_log().map(LoadedLog::name),
        Some("navsyncd.log")
    );
}

/// The recording in the history database the fixture attachments belong to.
fn stored_recording_ref() -> gt_store::DatabaseRef {
    gt_store::DatabaseRef {
        identity: "nav-devkit-mk2".to_owned(),
        group_name: "2026-05-29T18-48-25".to_owned(),
    }
}

/// The session sidecar of a recording the history database holds, which is
/// what lets a log anchored to that database entry resolve to it.
fn stored_in_history() -> FileHistory {
    FileHistory::recording(
        stored_recording_ref().identity,
        gt_store::RecordingMeta {
            time_range: None,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        },
        Some(stored_recording_ref()),
    )
}

/// The fixture log as the history database holds it with the fixture
/// recording.
fn stored_attachment() -> gt_store::LogAttachmentEntry {
    gt_store::LogAttachmentEntry {
        id: gt_store::LogAttachmentId::new_random(),
        attachment: gt_store::LogAttachment::new(
            "navsyncd.log".to_owned(),
            gt_store::LogContentHash::of_log_bytes(LOG_WITH_EVERY_ROW_KIND.as_bytes()),
            Vec::new(),
        ),
    }
}

/// The viewer over one recording of the history database, which holds
/// `attachment` as its one stored log, and over no loaded log.
fn viewer_state_over_a_stored_recording(attachment: gt_store::LogAttachmentEntry) -> ViewerState {
    let mut state = viewer_state(Vec::new(), &[]);
    state
        .recordings
        .push(recording("walk.gtd", 55.0), stored_in_history());
    state
        .attachments
        .set_attachments_of(stored_recording_ref(), vec![attachment]);
    state.viewer.open = true;
    state
}

/// Loads the fixture log into `state` holding `attachment`, as a log read back
/// out of the recording arrives.
fn load_the_stored_log(state: &mut ViewerState, attachment: &gt_store::LogAttachmentEntry) {
    let parsed = gt_logfile::parse_log(LOG_WITH_EVERY_ROW_KIND.into(), log_start())
        .unwrap_or_else(|error| panic!("the fixture log parses: {error}"));
    let mut log = LoadedLog::new(
        Some("navsyncd.log".to_owned()),
        parsed,
        Duration::seconds(ASSOCIATION_WINDOW_SECS),
    );
    log.restore_attachment(
        LogAttachmentRef {
            recording: stored_recording_ref(),
            id: attachment.id,
        },
        Vec::new(),
        &state.recordings.view(),
    );
    let id = state.logs.push(log).id();
    state.viewer.open_on_log(id);
}

/// Anchors the log named `log` to the recording at `index`, as choosing that
/// recording in the footer does.
fn anchor_log_to_recording(harness: &mut Harness<ViewerState>, log: &str, index: usize) {
    let state = harness.state_mut();
    let chosen: Option<LoadedLogId> = state
        .logs
        .iter_with_ids()
        .find(|(_, loaded)| loaded.name() == log)
        .map(|(id, _)| id);
    let recordings = state.recordings.view();
    let recording = recordings.get(index).map(|entry| entry.id());
    if let Some(loaded) = chosen.and_then(|id| state.logs.get_mut_by_id(id)) {
        loaded.anchor_to_loaded_recording(recording, &recordings);
    }
    harness.run_steps(2);
}

/// Every group of the list: its heading, and the name each of its rows shows.
fn listed_rows(state: &ViewerState) -> Vec<(String, Vec<String>)> {
    let names = RecordingNames::resolve(state.recordings.view(), "{filename}");
    log_list::group_logs_by_recording(
        &state.logs,
        state.recordings.view(),
        &names,
        &state.attachments,
    )
    .into_iter()
    .map(|group| {
        let rows = group
            .rows
            .iter()
            .map(|row| match row {
                log_list::LogRow::Loaded(loaded) => loaded.name.clone(),
                log_list::LogRow::Available(available) => available.name.clone(),
            })
            .collect();
        (group.heading.text(), rows)
    })
    .collect()
}

/// Two recordings with a log each, and a third log that takes its positions
/// from neither.
#[test]
fn the_list_groups_every_log_under_the_recording_it_is_anchored_to() {
    let mut harness = harness_of(
        vec![recording("walk.gtd", 55.0), recording("drive.gtd", 60.0)],
        &[
            ("navsyncd.log", LOG_WITH_EVERY_ROW_KIND),
            ("hal-powerd.log", SECOND_LOG),
            ("kernel.log", THIRD_LOG),
        ],
    );

    anchor_log_to_recording(&mut harness, "navsyncd.log", 0);
    anchor_log_to_recording(&mut harness, "hal-powerd.log", 1);

    assert_eq!(
        listed_rows(harness.state()),
        [
            ("walk.gtd".to_owned(), vec!["navsyncd.log".to_owned()]),
            ("drive.gtd".to_owned(), vec!["hal-powerd.log".to_owned()]),
            (
                log_list::NOT_ANCHORED_HEADING.to_owned(),
                vec!["kernel.log".to_owned()]
            ),
        ]
    );
    harness.get_by_label("walk.gtd");
    harness.get_by_label("drive.gtd");
    harness.get_by_label(log_list::NOT_ANCHORED_HEADING);
}

/// One log alone stands on its own row: there is nothing to group it against.
#[test]
fn a_session_holding_one_log_lists_it_without_a_heading() {
    let harness = harness_with(vec![recording("walk.gtd", 55.0)]);

    harness.get_by_label("navsyncd.log");
    assert!(harness.query_by_label("walk.gtd").is_none());
}

/// Each row draws the toggle of its own log, whichever log the viewer shows.
#[test]
fn the_map_toggle_of_a_row_leaves_the_selection_where_it_is() {
    let mut harness = harness_of(
        Vec::new(),
        &[
            ("navsyncd.log", LOG_WITH_EVERY_ROW_KIND),
            ("hal-powerd.log", SECOND_LOG),
        ],
    );
    let shown = harness.state().viewer.selected_log();
    let first = harness.state().first_loaded_log();
    assert_ne!(shown, Some(first), "the viewer shows the log loaded last");

    harness.nth_matching(By::new().label(ICON_EYE), 0).click();
    harness.run_steps(2);

    assert_eq!(
        harness
            .state()
            .logs
            .get_by_id(first)
            .map(LoadedLog::is_visible),
        Some(false),
        "the toggle of the first row switched off the log of that row"
    );
    assert_eq!(harness.state().viewer.selected_log(), shown);
}

/// What the unload button says it leaves behind: an attached log stays stored
/// with its recording.
#[rstest]
#[case::attached(
    ShownLogStorage::StoredWithTheRecording,
    "Unload this log. It stays attached to walk.gtd."
)]
#[case::stored_nowhere(ShownLogStorage::StoredNowhere, log_list::UNLOAD_HOVER)]
fn the_unload_hover_says_whether_the_log_stays_attached(
    #[case] storage: ShownLogStorage,
    #[case] expected: &str,
) {
    let mut harness = match storage {
        ShownLogStorage::StoredWithTheRecording => {
            let attachment = stored_attachment();
            let mut state = viewer_state_over_a_stored_recording(attachment.clone());
            load_the_stored_log(&mut state, &attachment);
            harness_from(state)
        }
        ShownLogStorage::StoredNowhere => harness_with(vec![recording("walk.gtd", 55.0)]),
    };
    let unload = harness.get_by_label(super::ICON_X).rect().center();

    harness.hover_at_and_settle(unload, TOOLTIP_DELAY_FRAMES);

    harness.get_by_label_contains(expected);
}

/// Whether the shown log is stored with the recording it is anchored to.
#[derive(Debug, Clone, Copy)]
enum ShownLogStorage {
    StoredWithTheRecording,
    StoredNowhere,
}

/// A stored log that is not loaded is listed under its recording, and the row
/// requests that the app read it back.
#[test]
fn an_attachment_that_is_not_loaded_is_listed_with_a_load_button() {
    let attachment = stored_attachment();
    let mut harness = harness_from(viewer_state_over_a_stored_recording(attachment.clone()));
    assert_eq!(
        listed_rows(harness.state()),
        [("walk.gtd".to_owned(), vec!["navsyncd.log".to_owned()])]
    );

    harness
        .get_by_label(log_list::LOAD_ATTACHMENT_LABEL)
        .click();
    harness.run_steps(2);

    let requested = harness
        .state()
        .requests
        .load_attachment
        .as_ref()
        .expect("the row requested that the app read the attachment back");
    assert_eq!(requested.attachment.id, attachment.id);
    assert_eq!(requested.attachment.recording, stored_recording_ref());
    assert_eq!(requested.name, "navsyncd.log");
}

/// The viewer over the fixture log stored with a recording the session has not
/// loaded, as opening that log from the history window leaves it.
fn harness_over_a_log_whose_recording_is_not_loaded() -> Harness<'static, ViewerState> {
    let mut state = viewer_state(Vec::new(), &[]);
    load_the_stored_log(&mut state, &stored_attachment());
    harness_from(state)
}

/// The heading the list gives the group of a recording that is not loaded, and
/// the row the footer states it on.
const NOT_LOADED_RECORDING: &str = "nav-devkit-mk2 (not loaded)";

/// A log whose recording is not loaded is listed under that recording, and the
/// footer states the anchor and offers to open the recording from history.
#[test]
fn the_footer_offers_the_recording_of_a_log_that_is_not_loaded() {
    let mut harness = harness_over_a_log_whose_recording_is_not_loaded();
    assert_eq!(
        listed_rows(harness.state()),
        [(
            NOT_LOADED_RECORDING.to_owned(),
            vec!["navsyncd.log".to_owned()]
        )]
    );
    harness.get_by_label(format!("Anchored to {NOT_LOADED_RECORDING}").as_str());

    harness.get_by_label(super::LOAD_RECORDING_LABEL).click();
    harness.run_steps(2);

    assert_eq!(
        harness.state().requests.open_recording,
        Some(stored_recording_ref())
    );
}

/// Opening the recording reads it from the recordings database: with no
/// database open there is nothing to read it from, and the button says so.
#[test]
fn loading_the_recording_is_grayed_while_no_recordings_database_is_open() {
    let mut harness = harness_over_a_log_whose_recording_is_not_loaded();
    harness.state_mut().history_available = false;
    harness.run_steps(2);

    let load = harness.get_by_label(super::LOAD_RECORDING_LABEL);
    assert!(load.accesskit_node().is_disabled());
    let position = load.rect().center();
    harness.hover_at_and_settle(position, TOOLTIP_DELAY_FRAMES);

    harness.get_by_label_contains(super::LOAD_RECORDING_NO_DATABASE_HOVER);
    assert_eq!(harness.state().requests.open_recording, None);
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

/// Shows the log named `name`, by clicking its row in the list.
fn select_log(harness: &mut Harness<ViewerState>, name: &str) {
    harness.get_by_label(name).click();
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
        harness.state().shown_log().map(LoadedLog::name),
        Some("hal-powerd.log"),
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

/// A log whose lines are longer and more numerous than any audit viewport
/// shows: an unbroken message stresses the width, the row count the height.
fn oversized_log_text() -> String {
    let unbroken = gt_test_utils::oversized_text('e');
    (0..OVERSIZED_ROW_COUNT)
        .map(|index| format!("2026-05-29 18:48:25 navsyncd[{index}]: {unbroken}\n"))
        .collect()
}

/// The log viewer keeps its footer controls reachable at any viewport, and an
/// unbroken log line scrolls inside the table instead of stretching the window
/// past the screen edge.
#[rstest::rstest]
fn log_viewer_window_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let mut state = viewer_state(Vec::new(), &[("navsyncd.log", &oversized_log_text())]);
    // Every part of the window at once: the notices, the parse summary the
    // header unfolds, and the table.
    state
        .viewer
        .report_warning(gt_test_utils::oversized_text('w'));
    state.viewer.summary_expanded = true;
    let mut harness = Harness::builder()
        .with_size(viewport)
        .build_ui_state(viewer_ui, state);
    gt_ui_theme::install_app_style(&harness.ctx);
    harness.run_steps(8);

    harness.assert_window_fits_the_viewport(AuditedWindow::titled(LOG_VIEWER_TITLE));
    harness.assert_control_is_reachable(
        AuditedWindow::titled(LOG_VIEWER_TITLE),
        ControlLabel("Associated with"),
    );
}
