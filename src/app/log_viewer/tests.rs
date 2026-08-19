//! The viewer driven on its own: the rows the table draws for one log, what a
//! row click asks of the map, and the footer's association controls.

use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone as _, Utc};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
use gt_log_view::{LoadedLog, LoadedLogs};
use gt_test_utils::{By, HarnessInteraction as _};
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::{FileSource, Latitude, Longitude};

use super::{AssociationWindowUnit, LogViewerContext, LogViewerWindow};

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

/// The timestamp column of the log's first entry, as the table writes it: a
/// leading space where an interpolated entry carries its marker.
const FIRST_ENTRY_TIMESTAMP: &str = " 2026-05-29 18:48:25";

/// The association window a freshly loaded log starts with, matching the app's
/// default.
const ASSOCIATION_WINDOW_SECS: i64 = 60;

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
    let mut loaded_recordings = LoadedFiles::new();
    for file in recordings {
        loaded_recordings.push(file, FileHistory::None);
    }
    let parsed = gt_logfile::parse_log(LOG_WITH_EVERY_ROW_KIND.into(), log_start())
        .unwrap_or_else(|error| panic!("the fixture log parses: {error}"));
    let mut log = LoadedLog::new(
        Some("navsyncd.log".to_owned()),
        parsed,
        Duration::seconds(ASSOCIATION_WINDOW_SECS),
    );
    let target = log
        .rank_association_candidates(&loaded_recordings.view())
        .unambiguous_target();
    log.associate_with(target, &loaded_recordings.view());

    let mut logs = LoadedLogs::default();
    logs.push(log);
    let mut viewer = LogViewerWindow::new();
    viewer.open_on_newly_loaded_log(&logs);

    let mut harness = Harness::builder()
        .with_size(egui::vec2(760.0, 560.0))
        .build_ui_state(
            |ui, state: &mut ViewerState| {
                let names = RecordingNames::resolve(state.recordings.view(), "{filename}");
                state.viewer.show(
                    ui.ctx(),
                    &mut state.logs,
                    LogViewerContext {
                        recordings: state.recordings.view(),
                        recording_names: &names,
                        map_center_request: &mut state.map_center,
                    },
                );
            },
            ViewerState {
                viewer,
                logs,
                recordings: loaded_recordings,
                map_center: None,
            },
        );
    harness.run_steps(3);
    harness
}

/// Clicks the table row whose timestamp column reads `timestamp`.
fn click_line(harness: &mut Harness<ViewerState>, timestamp: &str) {
    let row = harness.get_by_label(timestamp).rect().center();
    harness.press_drag_release(row, egui::Vec2::ZERO, 1);
    harness.run_steps(2);
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
