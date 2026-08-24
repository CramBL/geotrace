use egui::TextEdit;
use egui_phosphor::regular::ARROW_LINE_UP_LEFT as ICON_ARROW_LINE_UP_LEFT;
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use egui_phosphor::regular::ARTICLE as ICON_ARTICLE;
use egui_phosphor::regular::CHECK as ICON_CHECK;
use egui_phosphor::regular::COPY as ICON_COPY;
use egui_phosphor::regular::DOTS_SIX as ICON_DOTS_SIX;
use egui_phosphor::regular::FRAME_CORNERS as ICON_FRAME_CORNERS;
use egui_phosphor::regular::GEAR as ICON_GEAR;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use egui_phosphor::regular::PUSH_PIN as ICON_PUSH_PIN;
use egui_phosphor::regular::TERMINAL_WINDOW as ICON_TERMINAL_WINDOW;
use egui_phosphor::regular::X as ICON_X;
use std::collections::BTreeMap;
use std::ops::Range;
use std::panic;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc;
use std::thread;
use std::{
    sync::{Arc, Barrier},
    time::{Duration as StdDuration, Instant},
};

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use geotrace_sdk::{Channel, ChannelUnit, DateTime, Duration, Unit, Utc};
use gt_instance_lock::{
    DataDirectoryLock, DataDirectoryOwnership, InstanceState, InstanceStatus,
    MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES, SharedDataDirectoryLock,
};
use gt_jam::wire::HexObservation;
use gt_jam_store::schema;
use gt_log_view::LoadedLog;
use gt_pending_writes::{PendingWrites, WriteAccess, WriteKind};
use gt_store::{HistoryDatabase as _, InterruptedDelete, JamStore, Recordings};
use gt_test_utils::day_archive::{self, GroupPath};
use gt_test_utils::{
    By, DEMO_BYTES, GOLD_BYTES, HarnessInteraction as _, SyntheticGtdSpec, SyntheticLogSpec,
    SyntheticLogTimestamps, TestHarness, synthetic_gtd_bytes, synthetic_journald_log,
    synthetic_log_start,
};
use gt_types::{FileIdx, LoadWarning, TrackIdx, TrackRef};
use gt_ui_theme::MIDDLE_DOT;
use strum::IntoEnumIterator as _;

use super::App;
use super::archive_recovery::{
    self, ARCHIVE_IN_USE_BUTTON_LABEL, ArchiveUnavailable, InspectedArchives,
    InterruptedDeleteFinding, LEAVE_UNRECOVERED_BUTTON_LABEL, RECOVER_BUTTON_LABEL,
    UnavailableArchives,
};
use super::backfill_ui::DOWNLOAD_HISTORY_LABEL;
use super::environment_storage::EnvironmentArchive;
use super::environment_storage_ui::{
    AUTO_PRUNE_LABEL as ENVIRONMENT_AUTO_PRUNE_LABEL, DELETE_ALL_LABEL, DeleteBlocker,
    PRUNE_BUTTON_LABEL,
};
use super::history_open::CLEAR_LOCK_BUTTON_LABEL;
use super::instance_wait::{
    DATA_DIRECTORY_HELD_TITLE, DATA_DIRECTORY_RETRY_INTERVAL, LOCK_FILE_UNUSABLE_TITLE,
    START_READ_ONLY_BUTTON_LABEL, TAKE_OVER_BUTTON_LABEL, TAKE_OVER_CONFIRMATION_TITLE,
    TAKE_OVER_WARNING, TakenOverInstance,
};
use super::log_viewer;
use super::query;
use super::read_only_session::{READ_ONLY_MARKER_LABEL, READ_ONLY_RECORDING_HISTORY_HOVER};
use super::settings_ui::{self, SettingsPage};
use super::storage::{DatabasesPending, OPENING_DATABASES, OpenStorage, StorageOpen};
use super::storage_controls::AUTO_STORE_LABEL;
use crate::termination_signal::TERMINATION_SIGNAL_FLAG;

/// In-memory [`egui::DroppedFile`] for drag-drop tests. `bytes` drops carry a
/// relative path holding the display name, matching how web drops expose only
/// the file name, `path` drops behave like native drops from disk.
#[derive(Debug)]
struct TestDroppedFile {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

impl TestDroppedFile {
    fn bytes(bytes: impl Into<Vec<u8>>, name: &str) -> Self {
        Self {
            path: PathBuf::from(name),
            bytes: Some(bytes.into()),
        }
    }

    fn path(path: PathBuf) -> Self {
        Self { path, bytes: None }
    }
}

impl egui::DroppedFile for TestDroppedFile {
    fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        match &self.bytes {
            Some(bytes) => Ok(bytes.clone()),
            None => std::fs::read(&self.path).map_err(|e| e.to_string()),
        }
    }
}

/// App constructor for snapshot harnesses, persisting settings at the harness's
/// temp config path. `fading` is supplied by the harness (off by default) so
/// snapshots don't capture mid-animation hover fades.
fn build_app(cc: &eframe::CreationContext<'_>, config_path: &std::path::Path, fading: bool) -> App {
    build_app_with_write_access(cc, config_path, fading, WriteAccess::Owner)
}

/// [`build_app`] for a session with the given write access, which is what
/// decides whether the settings are persisted at all.
fn build_app_with_write_access(
    cc: &eframe::CreationContext<'_>,
    config_path: &std::path::Path,
    fading: bool,
    write_access: WriteAccess,
) -> App {
    App::new_with_config(
        cc,
        &[],
        Some(config_path.to_path_buf()),
        super::StartupOptions {
            fading_enabled: fading,
            offline: true,
            storage: crate::app::Storage::Disabled,
            app_version: super::TEST_APP_VERSION,
            pending_writes: PendingWrites::new(write_access),
            instance_lock: SharedDataDirectoryLock::marking_nothing(),
        },
    )
}

/// Fixes every date the settings window seeds from today, or its snapshots
/// would redate every day.
fn pin_settings_dates(app: &mut App) {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 2).unwrap_or_default();
    app.interference_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
    app.geomagnetic_index_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
    app.tec_map_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
    app.solar_flare_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
    app.environment_storage_ui =
        crate::app::environment_storage_ui::EnvironmentStorageUi::with_today(today);
}

/// App constructor for the functional (non-snapshot) tests that don't touch a
/// config file. Fading stays off so frame counts are deterministic.
fn transient_app(cc: &mut eframe::CreationContext<'_>) -> App {
    transient_app_with_paths(cc, &[])
}

/// [`transient_app`] started with the files a command line named.
fn transient_app_with_paths(cc: &eframe::CreationContext<'_>, paths: &[PathBuf]) -> App {
    transient_app_with_the_instance_lock(
        cc,
        paths,
        SharedDataDirectoryLock::marking_nothing(),
        PendingWrites::default(),
    )
}

/// [`transient_app_with_paths`] on the data directory `instance_lock` was
/// taken on, which is what decides whether the run opens anything, with
/// `pending_writes` deciding whether it writes to it at all.
fn transient_app_with_the_instance_lock(
    cc: &eframe::CreationContext<'_>,
    paths: &[PathBuf],
    instance_lock: SharedDataDirectoryLock,
    pending_writes: PendingWrites,
) -> App {
    App::new_with_config(
        cc,
        paths,
        None,
        super::StartupOptions {
            fading_enabled: false,
            offline: true,
            storage: crate::app::Storage::Disabled,
            app_version: super::TEST_APP_VERSION,
            pending_writes,
            instance_lock,
        },
    )
}

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("fixed timestamp is valid")
}

fn minimal_gtd_bytes() -> Vec<u8> {
    synthetic_gtd_bytes(SyntheticGtdSpec {
        start: base_time(),
        point_count: 61,
        step_secs: 1,
        start_lat_deg: 51.5,
        start_lon_deg: -0.1,
        lat_step_deg: 0.0002,
        lon_step_deg: -0.00015,
        heading_deg: 270.0,
        speed_kmh: 22.0,
        eph_m: 2.4,
        sats_seen: 10,
        sats_in_fix: 7,
    })
}

/// Drop `file` into the app and step until the background load thread has
/// finished with it.
///
/// The thread sends a `Completed` message when done and `drain_load_channel`
/// (called at the start of every `ui()` frame) removes the job.
fn drop_file_and_wait_for_load(harness: &mut Harness<App>, file: TestDroppedFile) {
    harness.input_mut().dropped_files.push(Arc::new(file));
    harness.step();
    assert!(
        harness.step_until(|harness| harness.state().loader.loading_jobs.is_empty()),
        "the background load did not finish"
    );
}

/// Step the harness repeatedly until the query worker's result has landed.
fn step_until_query_result(harness: &mut Harness<App>) {
    assert!(
        harness.step_until(|harness| harness.state().query_window.matches().is_some()),
        "the query worker produced no result"
    );
}

fn load_three_overlapping_files(harness: &mut Harness<App>) {
    let t0 = base_time();
    let overlapping_files = [
        (
            "overlap_a.gtd",
            synthetic_gtd_bytes(SyntheticGtdSpec {
                start: t0,
                point_count: 240,
                step_secs: 1,
                start_lat_deg: 55.0000,
                start_lon_deg: 12.0000,
                lat_step_deg: 0.00005,
                lon_step_deg: 0.00008,
                heading_deg: 20.0,
                speed_kmh: 28.0,
                eph_m: 1.8,
                sats_seen: 14,
                sats_in_fix: 11,
            }),
        ),
        (
            "overlap_b.gtd",
            synthetic_gtd_bytes(SyntheticGtdSpec {
                start: t0,
                point_count: 240,
                step_secs: 1,
                start_lat_deg: 55.0003,
                start_lon_deg: 12.0002,
                lat_step_deg: 0.00006,
                lon_step_deg: 0.00007,
                heading_deg: 32.0,
                speed_kmh: 31.0,
                eph_m: 2.1,
                sats_seen: 13,
                sats_in_fix: 10,
            }),
        ),
        (
            "overlap_c.gtd",
            synthetic_gtd_bytes(SyntheticGtdSpec {
                start: t0,
                point_count: 240,
                step_secs: 1,
                start_lat_deg: 54.9998,
                start_lon_deg: 11.9997,
                lat_step_deg: 0.00004,
                lon_step_deg: 0.00009,
                heading_deg: 14.0,
                speed_kmh: 26.0,
                eph_m: 2.6,
                sats_seen: 12,
                sats_in_fix: 9,
            }),
        ),
    ];

    for (name, bytes) in overlapping_files {
        drop_file_and_wait_for_load(harness, TestDroppedFile::bytes(bytes, name));
    }
}

#[test]
fn drag_drop_gtd_path_loads_file() {
    let gtd_bytes = minimal_gtd_bytes();
    let tmp = tempfile::NamedTempFile::with_suffix(".gtd").expect("create temp file");
    std::io::Write::write_all(&mut tmp.as_file(), &gtd_bytes).expect("write temp gtd");
    let tmp_path = tmp.path().to_path_buf();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(&mut harness, TestDroppedFile::path(tmp_path));

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

#[test]
fn drag_drop_gtd_bytes_loads_file() {
    let gtd_bytes = minimal_gtd_bytes();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

/// Query results gray out when the data they were computed from changes -
/// here via a global-filter edit - and recover when it changes back.
#[test]
fn query_results_go_stale_when_the_filter_changes() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h".to_owned());
    }
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness);
    harness.run_steps(3);
    let stale_after_run = harness
        .state()
        .query_window
        .matches()
        .expect("run produced matches")
        .stale;
    assert!(!stale_after_run, "fresh results are not stale");

    // A minimum-distance filter changes the evaluated track set.
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .min_distance_km = Some(uom::si::f64::Length::new::<uom::si::length::kilometer>(
        999.0,
    ));
    harness.run_steps(3);
    let matches_stale = harness
        .state()
        .query_window
        .matches()
        .expect("results kept while stale")
        .stale;
    assert!(matches_stale, "results gray out when the filter changes");

    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .min_distance_km = None;
    harness.run_steps(3);
    let stale_after_revert = harness
        .state()
        .query_window
        .matches()
        .expect("results kept")
        .stale;
    assert!(!stale_after_revert, "reverting the filter un-grays results");
}

/// Gives every loaded track interference query values: installs a scheduler
/// whose archive values the cell each loaded fix sits in.
fn install_interference_archive_covering_loaded_fixes(
    harness: &mut Harness<'_, App>,
) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_interference()
        .expect("archive");
    let mut observations_by_day: BTreeMap<chrono::NaiveDate, Vec<HexObservation>> = BTreeMap::new();
    {
        let state = harness.state();
        let shared = state.shared.borrow();
        for file in shared.loaded_files.files() {
            for track in &file.tracks {
                for point in &track.points {
                    let Some(cell) = gt_jam::dataset::cell_at(point.tpv.lat(), point.tpv.lon())
                    else {
                        continue;
                    };
                    let observations = observations_by_day
                        .entry(point.tpv.time().utc().date_naive())
                        .or_default();
                    if !observations
                        .iter()
                        .any(|observation| observation.cell == cell)
                    {
                        observations.push(HexObservation {
                            cell,
                            good: 90,
                            bad: 10,
                        });
                    }
                }
            }
        }
    }
    for (day, observations) in observations_by_day {
        store
            .insert_day(day, "host", chrono::Utc::now(), &observations)
            .expect("insert");
    }
    install_interference_scheduler(harness, &store);
    dir
}

/// A run over a recording with archived interference stays live: the
/// per-track interference values the app hands the query fingerprint every
/// frame keep their `Arc` identity while the archive holds them.
#[test]
fn query_results_over_archived_interference_stay_fresh() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );
    let _archive = install_interference_archive_covering_loaded_fixes(&mut harness);

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h".to_owned());
    }
    harness.run_steps(3);
    assert!(
        !harness.state().jamming.query_values().is_empty(),
        "the loaded track must carry interference values for the fingerprint to compare"
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness);
    harness.run_steps(5);

    assert!(
        !harness
            .state()
            .query_window
            .matches()
            .expect("run produced matches")
            .stale,
        "its results stay live: nothing changed since the run"
    );
}

/// `snap_error` evaluates over a completed run's values without any network
/// step: points with a value match the draw query, and a re-snap grays the
/// results out through the fingerprint.
#[test]
fn query_matches_on_snap_error_after_a_run() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );
    let track = gt_types::TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
    inject_completed_run(&mut harness, track);

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where snap_error >= 2 m | draw".to_owned());
    }
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness);
    harness.run_steps(3);
    let matches = harness
        .state()
        .query_window
        .matches()
        .expect("run produced results");
    assert!(!matches.stale, "fresh results are not stale");
    assert!(
        matches
            .draws
            .first()
            .is_some_and(|layer| !layer.ranges.is_empty()),
        "snapped points must match the snap_error draw"
    );

    // A re-snap produces new values: the results gray out.
    inject_completed_run(&mut harness, track);
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .query_window
            .matches()
            .expect("results kept while stale")
            .stale,
        "a new run must gray snap_error results out"
    );
}

/// Clicking a point row pins that point, like a point row in the side panel:
/// the map then owns a pinned popup for it.
#[test]
fn query_point_row_click_pins_its_point() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h");
    harness.run_steps(3);

    let first_row = topmost_point_row(&harness).0.center();
    harness.press_drag_release(first_row, egui::Vec2::ZERO, 1);
    harness.run_steps(2);

    let sticky = harness
        .state()
        .shared
        .borrow()
        .highlight
        .sticky
        .expect("the row click pins its point");
    assert_eq!(
        sticky.track,
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    );
    assert_eq!(sticky.category, gt_types::DataCategory::Tpv);
}

/// The matches table lists a track, times and counts, and the points table
/// stretches its striping across the window: neither may widen the window,
/// which would cover the map beside it.
#[test]
fn a_run_leaves_the_query_window_at_its_default_width() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h");
    harness.run_steps(5);

    let width = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open")
        .width();
    assert!(
        width <= query::DEFAULT_WINDOW_WIDTH,
        "the results widened the window to {width}"
    );
}

/// Neither the results, a tab switch nor scrolling may make the window taller:
/// a window that claimed the height it could have would cover the plot below
/// it.
#[test]
fn the_results_leave_the_query_window_at_its_default_height() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let height_of = |harness: &Harness<'_, App>| {
        harness
            .window_rect(QUERY_WINDOW_TITLE)
            .expect("the query window is open")
            .height()
    };
    let with_results = height_of(&harness);
    assert!(
        with_results <= query::DEFAULT_WINDOW_HEIGHT,
        "the results grew the window to {with_results}"
    );

    harness.get_by_label("Examples").click();
    harness.run_steps(5);
    harness.get_by_label("Results").click();
    harness.run_steps(5);
    let after_tabs = height_of(&harness);
    assert!(
        after_tabs <= query::DEFAULT_WINDOW_HEIGHT,
        "switching tabs grew the window to {after_tabs}"
    );

    let rows = topmost_point_row(&harness).0.center();
    harness.scroll_wheel_at(rows, -RESULTS_WHEEL_POINTS, WHEEL_SETTLE_FRAMES);
    let after_scrolling = height_of(&harness);
    assert!(
        after_scrolling <= query::DEFAULT_WINDOW_HEIGHT,
        "scrolling the rows grew the window to {after_scrolling}"
    );
}

/// The demo recording loaded with `query` run over it, for the tests that
/// drive the results table. Its track matches in several stretches, which one
/// match table then lists.
fn demo_app_with_query_run(query: &str) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    run_query(&mut harness, query);
    harness
}

/// A demo-trip query with two matches of clearly different length, for the
/// tests that pick, sort and frame one of them.
const TWO_MATCH_QUERY: &str = "points | window 10 | where avg(velocity) > 25 km/h";

/// A demo-trip query with more matches than the matches table lists at once,
/// each of them holding rows for the points table below it.
const MANY_MATCH_QUERY: &str = "points | where accel < -0.2 m/s2";

/// The query window's title, which both its area and its accesskit node are
/// addressed by.
const QUERY_WINDOW_TITLE: &str = "Query";

/// Wheel points sent over the results, past the rows one viewport holds.
const RESULTS_WHEEL_POINTS: f32 = 200.0;

/// Frames a wheel scroll's smooth animation takes to come to rest.
const WHEEL_SETTLE_FRAMES: usize = 12;

/// Motion below which a widget counts as having stayed where it was.
const STATIONARY_TOLERANCE_PX: f32 = 0.5;

/// Length of the bare wall-clock label a point row's time column states.
const ROW_TIME_LEN: usize = "14:00:19".len();

/// Every widget labeled with a bare wall-clock time: the start and end columns
/// of the matches table, and the time column of the points table. A channel
/// run's samples are timed to the millisecond, so the time carries a fraction
/// there.
fn bare_time_labels<'a>() -> By<'a> {
    By::new().role(egui::accesskit::Role::Label).predicate(
        |node: &egui_kittest::kittest::AccessKitNode<'_>| {
            node.value().is_some_and(|value| {
                value.len() >= ROW_TIME_LEN
                    && value.contains(':')
                    && value
                        .chars()
                        .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
            })
        },
    )
}

/// The rows one of the results tab's two tables lists, top row first: the rect
/// of the row's first time cell and the time it states.
///
/// The caption naming the picked match starts at the left edge of the tab, and
/// so does the points table's time column. The matches table indents its times
/// behind the swatch, number and track columns.
fn time_rows(harness: &Harness<'_, App>, table: ResultsTable) -> Vec<(egui::Rect, String)> {
    let left_edge = harness.get_by_label_contains("Match ").rect().left();
    rows_in_reading_order(
        time_cells_in_window(harness, QUERY_WINDOW_TITLE)
            .into_iter()
            .filter(|(rect, _)| {
                let indented = rect.left() > left_edge + INDENT_TOLERANCE_PX;
                match table {
                    ResultsTable::Matches => indented,
                    ResultsTable::Points => !indented,
                }
            })
            .collect(),
    )
}

/// The rows the matches table lists in the window it was popped out into, top
/// row first. Every time that window states belongs to a match row.
fn popped_out_match_rows(harness: &Harness<'_, App>) -> Vec<(egui::Rect, String)> {
    rows_in_reading_order(time_cells_in_window(
        harness,
        query::results::MATCH_LIST_WINDOW_TITLE,
    ))
}

/// Every bare wall-clock label the window titled `title` states, with the rect
/// it was laid out in.
fn time_cells_in_window(harness: &Harness<'_, App>, title: &str) -> Vec<(egui::Rect, String)> {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .query_all(bare_time_labels())
        .filter_map(|node| Some((node.rect(), node.accesskit_node().value()?)))
        .collect()
}

/// `cells` in reading order: down the rows, and left to right within a row - a
/// match row states a start and an end, of which the start is kept.
fn rows_in_reading_order(mut cells: Vec<(egui::Rect, String)>) -> Vec<(egui::Rect, String)> {
    cells.sort_by(|(a, _), (b, _)| {
        a.top()
            .total_cmp(&b.top())
            .then_with(|| a.left().total_cmp(&b.left()))
    });
    cells.dedup_by(|(a, _), (b, _)| (a.top() - b.top()).abs() < INDENT_TOLERANCE_PX);
    cells
}

/// The matches listed in the window titled `title`, counted by the button each
/// row carries to frame the map on its match.
fn listed_match_count(harness: &Harness<'_, App>, title: &str) -> usize {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .query_all_by_role_and_label(egui::accesskit::Role::Button, ICON_FRAME_CORNERS)
        .count()
}

/// How far a cell may sit from the tab's left edge and still count as starting
/// there, and how far two cells' tops may differ and still be one row.
const INDENT_TOLERANCE_PX: f32 = 2.0;

/// Which of the results tab's two tables a test reads.
#[derive(Clone, Copy)]
enum ResultsTable {
    Matches,
    Points,
}

/// The topmost point row's time cell and the time it states.
fn topmost_point_row(harness: &Harness<'_, App>) -> (egui::Rect, String) {
    time_rows(harness, ResultsTable::Points)
        .into_iter()
        .next()
        .expect("the results list point rows")
}

/// The time stated by the topmost point row on display.
fn topmost_row_time(harness: &Harness<'_, App>) -> String {
    topmost_point_row(harness).1
}

/// The start time of the topmost match row on display.
fn topmost_match_row(harness: &Harness<'_, App>) -> (egui::Rect, String) {
    time_rows(harness, ResultsTable::Matches)
        .into_iter()
        .next()
        .expect("the results list matches")
}

/// The wheel over the points table scrolls its rows, and neither the matches
/// table above it nor the editor above the tab strip moves with them: the two
/// tables scroll on their own.
#[test]
fn the_wheel_over_the_results_scrolls_its_rows() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let run_button_top = |harness: &Harness<'_, App>| {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
            .rect()
            .top()
    };
    let first_time = topmost_row_time(&harness);
    let run_button_before = run_button_top(&harness);
    let first_match_before = topmost_match_row(&harness).0.top();

    let rows = topmost_point_row(&harness).0.center();
    harness.scroll_wheel_at(rows, -RESULTS_WHEEL_POINTS, WHEEL_SETTLE_FRAMES);

    let scrolled_time = topmost_row_time(&harness);
    assert!(
        scrolled_time > first_time,
        "the wheel scrolled to later rows: {first_time} then {scrolled_time}"
    );
    let editor_shift = (run_button_top(&harness) - run_button_before).abs();
    assert!(
        editor_shift < STATIONARY_TOLERANCE_PX,
        "the editor above the tabs moved {editor_shift} px with the rows"
    );
    let match_shift = (topmost_match_row(&harness).0.top() - first_match_before).abs();
    assert!(
        match_shift < STATIONARY_TOLERANCE_PX,
        "the matches table moved {match_shift} px with the point rows"
    );
}

/// A run with more matches than the matches table shows at once scrolls them
/// under its own header, leaving the points table below it where it is.
#[test]
fn the_wheel_over_the_matches_scrolls_only_them() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    let first_match = topmost_match_row(&harness);
    let first_point_before = topmost_point_row(&harness);

    harness.scroll_wheel_at(
        first_match.0.center(),
        -RESULTS_WHEEL_POINTS,
        WHEEL_SETTLE_FRAMES,
    );

    let scrolled_match = topmost_match_row(&harness).1;
    assert!(
        scrolled_match > first_match.1,
        "the wheel scrolled to later matches: {} then {scrolled_match}",
        first_match.1
    );
    assert_eq!(
        topmost_point_row(&harness).1,
        first_point_before.1,
        "the points of the picked match stay where they are"
    );
}

/// Pixels the splitter is dragged to give the matches table the height of the
/// points table below it, and the far longer drag that runs into either clamp.
const SPLITTER_DRAG_PX: f32 = 120.0;
const SPLITTER_DRAG_PAST_THE_CLAMP_PX: f32 = 400.0;

/// The query window's button moving the matches list into a window of its own.
/// The side panel offers the same icon, so this looks only in the query window.
fn pop_out_button<'h>(harness: &'h Harness<'_, App>) -> egui_kittest::Node<'h> {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, QUERY_WINDOW_TITLE)
        .get_by_label(ICON_ARROW_SQUARE_OUT)
}

/// The centre of the splitter band, read again after every drag: it sits where
/// the matches table now ends.
fn splitter_center(harness: &Harness<'_, App>) -> egui::Pos2 {
    harness
        .get_by_label(query::results::SPLITTER_LABEL)
        .rect()
        .center()
}

/// Dragging the splitter down gives the matches table the height the points
/// table had, listing the matches that were scrolled out of it. The window it
/// is in keeps its size: the splitter only divides what the tab already has.
#[test]
fn dragging_the_splitter_lists_more_matches() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    let listed = |harness: &Harness<'_, App>| {
        time_rows(harness, ResultsTable::Matches)
            .into_iter()
            .map(|(_, time)| time)
            .collect::<Vec<_>>()
    };
    let before = listed(&harness);
    let window_before = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open")
        .size();

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, SPLITTER_DRAG_PX),
        4,
    );
    harness.run_steps(3);

    let after = listed(&harness);
    assert!(
        after.len() > before.len(),
        "the drag listed more matches: {before:?} then {after:?}"
    );
    assert!(
        after.starts_with(&before),
        "the matches already listed stayed where they were: {before:?} then {after:?}"
    );
    let window_after = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open")
        .size();
    assert!(
        (window_after - window_before).length() < STATIONARY_TOLERANCE_PX,
        "the drag resized the window from {window_before:?} to {window_after:?}"
    );
}

/// A double-click on the splitter puts the boundary back where the tab opened
/// it.
#[test]
fn double_clicking_the_splitter_puts_the_matches_table_back() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    let default_rows = time_rows(&harness, ResultsTable::Matches).len();

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, SPLITTER_DRAG_PX),
        4,
    );
    harness.run_steps(3);
    let dragged_rows = time_rows(&harness, ResultsTable::Matches).len();
    assert!(
        dragged_rows > default_rows,
        "the drag listed more matches: {default_rows} then {dragged_rows}"
    );

    let splitter = splitter_center(&harness);
    harness.double_click_at(splitter);
    harness.run_steps(3);

    assert_eq!(
        time_rows(&harness, ResultsTable::Matches).len(),
        default_rows,
        "the double-click listed the matches the tab opened with again"
    );
}

/// However far the splitter is dragged, both tables keep rows on display:
/// neither can be collapsed to its header.
#[test]
fn the_splitter_keeps_rows_of_both_tables_on_display() {
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, -SPLITTER_DRAG_PAST_THE_CLAMP_PX),
        4,
    );
    harness.run_steps(3);
    let matches_left = time_rows(&harness, ResultsTable::Matches).len();
    assert!(
        matches_left >= query::results_split::MIN_SPLIT_ROWS,
        "dragging to the top left {matches_left} matches on display"
    );

    harness.press_drag_release(
        splitter_center(&harness),
        egui::vec2(0.0, SPLITTER_DRAG_PAST_THE_CLAMP_PX),
        4,
    );
    harness.run_steps(3);
    let points_left = time_rows(&harness, ResultsTable::Points).len();
    assert!(
        points_left >= query::results_split::MIN_SPLIT_ROWS,
        "dragging to the bottom left {points_left} point rows on display"
    );
}

/// The pop-out button moves the matches into a window of their own, leaving the
/// results tab to the picked match's rows. Closing that window puts them back.
#[test]
fn popping_the_matches_out_moves_them_into_their_own_window() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let matches = listed_match_count(&harness, QUERY_WINDOW_TITLE);
    assert_eq!(matches, 2, "the results tab lists the run's matches");

    pop_out_button(&harness).click();
    harness.run_steps(5);

    assert_eq!(
        listed_match_count(&harness, query::results::MATCH_LIST_WINDOW_TITLE),
        matches,
        "every match moved into the popped-out window"
    );
    assert_eq!(
        listed_match_count(&harness, QUERY_WINDOW_TITLE),
        0,
        "the results tab lists none of them any more"
    );
    assert_eq!(
        harness.query_all_by_label_contains("Match 1 ").count(),
        1,
        "the caption over the point rows stays in the results tab"
    );

    harness
        .get_by_role_and_label(
            egui::accesskit::Role::Window,
            query::results::MATCH_LIST_WINDOW_TITLE,
        )
        .get_by_label("Close window")
        .click();
    harness.run_steps(5);

    assert_eq!(
        harness
            .query_all_by_role_and_label(
                egui::accesskit::Role::Window,
                query::results::MATCH_LIST_WINDOW_TITLE
            )
            .count(),
        0,
        "closing the window took it off the screen"
    );
    assert_eq!(
        listed_match_count(&harness, QUERY_WINDOW_TITLE),
        matches,
        "the matches are listed in the results tab again"
    );
}

/// The popped-out window keeps the size it opened at however long it stays on
/// screen: a table claiming the height it could have would grow it by one
/// spacing every frame.
#[test]
fn the_popped_out_matches_window_keeps_its_default_size() {
    // More matches than the window can list, so its table fills the height it
    // was given rather than shrinking to its rows.
    let mut harness = demo_app_with_query_run(MANY_MATCH_QUERY);
    pop_out_button(&harness).click();
    harness.run_steps(5);

    let size_of = |harness: &Harness<'_, App>| {
        harness
            .window_rect(query::results::MATCH_LIST_WINDOW_TITLE)
            .expect("the matches list opened its own window")
            .size()
    };
    let opened = size_of(&harness);
    assert!(
        opened.x <= query::results::MATCH_LIST_WINDOW_WIDTH
            && opened.y <= query::results::MATCH_LIST_WINDOW_HEIGHT,
        "the matches window opened at {opened:?}"
    );

    harness.run_steps(30);
    let settled = size_of(&harness);
    assert!(
        (settled - opened).length() < STATIONARY_TOLERANCE_PX,
        "the matches window grew from {opened:?} to {settled:?}"
    );
}

/// The popped-out window and the results tab share what the tab kept: a match
/// picked in the window lists its rows in the query window.
#[test]
fn a_match_picked_in_the_popped_out_window_lists_its_points_in_the_query_window() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let first_point = topmost_row_time(&harness);

    pop_out_button(&harness).click();
    harness.run_steps(5);

    let second_match = popped_out_match_rows(&harness)
        .get(1)
        .map(|(rect, _)| rect.center())
        .expect("the popped-out window lists a second match");
    harness.press_drag_release(second_match, egui::Vec2::ZERO, 1);
    harness.run_steps(3);

    assert_eq!(
        harness.query_all_by_label_contains("Match 2 ").count(),
        1,
        "the caption in the query window names the picked match"
    );
    let second_point = topmost_row_time(&harness);
    assert!(
        second_point > first_point,
        "the query window lists the second match's points: {first_point} then {second_point}"
    );
}

/// The tab strip shows one list at a time: the history tab replaces the
/// results, and the results tab shows them again.
#[test]
fn the_history_tab_replaces_the_results() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h");
    assert_eq!(
        harness.query_all_by_label_contains("Show on map").count(),
        1,
        "the results tab opens on the run"
    );

    harness.get_by_label("Query history").click();
    harness.run_steps(3);
    assert_eq!(
        harness.query_all_by_label_contains("Show on map").count(),
        0,
        "the history tab replaces the results"
    );
    assert_eq!(
        harness
            .query_all_by_label_contains("points | where velocity")
            .count(),
        1,
        "the history lists the query that ran"
    );

    harness.get_by_label("Results").click();
    harness.run_steps(3);
    assert_eq!(
        harness.query_all_by_label_contains("Show on map").count(),
        1,
        "the results tab shows them again"
    );
}

/// Neither scrolling the results nor switching tabs may widen the window over
/// the map: a tab's scroll area uses the width the window has, never more.
#[test]
fn scrolling_and_switching_tabs_leave_the_query_window_at_its_default_width() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let rows = topmost_point_row(&harness).0.center();
    harness.scroll_wheel_at(rows, -RESULTS_WHEEL_POINTS, WHEEL_SETTLE_FRAMES);

    let width_of = |harness: &Harness<'_, App>| {
        harness
            .window_rect(QUERY_WINDOW_TITLE)
            .expect("the query window is open")
            .width()
    };
    let scrolled = width_of(&harness);
    assert!(
        scrolled <= query::DEFAULT_WINDOW_WIDTH,
        "scrolling the results widened the window to {scrolled}"
    );

    harness.get_by_label("Examples").click();
    harness.run_steps(5);
    let switched = width_of(&harness);
    assert!(
        switched <= query::DEFAULT_WINDOW_WIDTH,
        "the examples tab widened the window to {switched}"
    );
}

/// Height a row and the gap under it take, as a tolerance on where the last
/// listed row ends.
const ROW_HEIGHT_ALLOWANCE: f32 = 40.0;

/// The points table reaches down to the bottom of the window: it takes what
/// the matches table above it leaves.
#[test]
fn the_results_fill_the_rest_of_the_window() {
    let harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    let window = harness
        .window_rect(QUERY_WINDOW_TITLE)
        .expect("the query window is open");
    let lowest_row = time_rows(&harness, ResultsTable::Points)
        .into_iter()
        .map(|(rect, _)| rect.bottom())
        .fold(f32::MIN, f32::max);

    assert!(
        window.bottom() - lowest_row < ROW_HEIGHT_ALLOWANCE,
        "the rows stop at {lowest_row} in a window ending at {}",
        window.bottom()
    );
}

/// Clicking a match lists its rows in the points table below, and the caption
/// there names the match on display.
#[test]
fn clicking_a_match_lists_its_points() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    assert_eq!(
        harness.query_all_by_label_contains("Match 1 ").count(),
        1,
        "the first match is picked until another one is"
    );
    let first_point = topmost_row_time(&harness);

    let second_match = time_rows(&harness, ResultsTable::Matches)
        .get(1)
        .map(|(rect, _)| rect.center())
        .expect("the run lists a second match");
    harness.press_drag_release(second_match, egui::Vec2::ZERO, 1);
    harness.run_steps(3);

    assert_eq!(
        harness.query_all_by_label_contains("Match 2 ").count(),
        1,
        "the caption names the picked match"
    );
    let second_point = topmost_row_time(&harness);
    assert!(
        second_point > first_point,
        "the points table lists the second match: {first_point} then {second_point}"
    );
}

/// Clicking a column header orders the matches by that column, and clicking it
/// again reverses the order.
#[test]
fn a_column_header_click_sorts_the_matches() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    // The two matches of the run, told apart by how long each one ran.
    let longest_first = |harness: &Harness<'_, App>| {
        harness.get_by_label("1:01").rect().top() < harness.get_by_label("0:11").rect().top()
    };
    assert!(
        longest_first(&harness),
        "run order lists the long match first"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Label, "points")
        .click();
    harness.run_steps(3);
    assert!(
        longest_first(&harness),
        "the first click sorts largest first"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Label, "points")
        .click();
    harness.run_steps(3);
    assert!(
        !longest_first(&harness),
        "clicking the header again sorts smallest first"
    );
}

/// A match's own "Show on map" frames the map on that one match, tighter than
/// the run-wide button frames every match of the run.
#[test]
fn a_match_row_frames_the_map_on_that_match() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);

    harness.get_by_label_contains("Show on map").click();
    harness.run_steps(3);
    let framed_run = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the run's matches");

    // The second match is the shorter of the two, so framing it narrows the
    // viewport whichever way the first one framed.
    harness
        .get_all_by_role_and_label(egui::accesskit::Role::Button, ICON_FRAME_CORNERS)
        .nth(1)
        .expect("every match row offers its own button")
        .click();
    harness.run_steps(3);
    let framed_match = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the one match");

    assert!(
        framed_match.lon_max - framed_match.lon_min < framed_run.lon_max - framed_run.lon_min,
        "one match frames tighter than the whole run: \
         {framed_match:?} against {framed_run:?}"
    );
}

/// Hovering a value column's header explains the metric it holds, out of the
/// same catalog the editor documents that metric from.
#[test]
fn hovering_a_column_header_explains_its_metric() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h");

    let header = harness.get_by_label("velocity").rect().center();
    harness.hover_at_and_settle(header, 5);

    assert_eq!(
        harness.query_all_by_label_contains("ground speed").count(),
        1,
        "the header hover states what the metric measures"
    );
}

/// "Copy as TSV" writes the whole run to the clipboard: a header line naming
/// each column in its unit, then one line per matched point.
#[test]
fn copying_a_query_result_writes_a_tab_separated_table() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h | draw");

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, ICON_COPY)
        .click();
    harness.run_steps(1);

    let copied = copied_text(&harness);
    let matches = harness
        .state()
        .query_window
        .matches()
        .expect("the run produced matches");
    let matched_points: usize = matches.draws.first().map_or(0, |layer| {
        layer.ranges.values().flatten().map(Range::len).sum()
    });
    let mut lines = copied.lines();
    assert_eq!(lines.next(), Some("match\tpoint\ttime\tvelocity (km/h)"));
    assert_eq!(lines.next(), Some("1\t0\t11:33:20\t22.0"));
    assert_eq!(
        copied.lines().count(),
        matched_points + 1,
        "one line per matched point, under the header"
    );
}

/// The copy follows the order the matches table lists: sorting the matches
/// smallest first copies the smaller match's rows ahead of the larger one's.
#[test]
fn copying_after_sorting_writes_the_matches_in_the_listed_order() {
    let mut harness = demo_app_with_query_run(TWO_MATCH_QUERY);
    // The first click on the points header sorts largest first, the second
    // smallest first.
    for _ in 0..2 {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Label, "points")
            .click();
        harness.run_steps(3);
    }

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, ICON_COPY)
        .click();
    harness.run_steps(1);

    let copied = copied_text(&harness);
    let first_row = copied.lines().nth(1).expect("the copy lists rows");
    assert!(
        first_row.starts_with("2\t"),
        "the smaller match is copied first: {first_row}"
    );
}

/// The text the app last put on the clipboard.
fn copied_text(harness: &Harness<'_, App>) -> String {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            egui::OutputCommand::OpenUrl(_) | egui::OutputCommand::CopyImage(_) => None,
        })
        .expect("nothing was copied")
}

/// The accel fixture with a channel-source query run over it: two stretches of
/// matched samples for one table to list.
fn channel_app_with_query_run() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(accel_channel_gtd_bytes(28.0), "accel_demo.gtd"),
    );
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    run_query(&mut harness, "@accel | where @accel.x > 1 g");
    harness
}

/// A channel run lists a match per stretch of matched samples, and picking one
/// lists its samples below.
#[test]
fn a_channel_run_lists_a_match_per_stretch_of_samples() {
    let mut harness = channel_app_with_query_run();
    for stretch in ACCEL_HIGH_RANGES {
        assert_eq!(
            harness
                .query_all_by_role_and_label(
                    egui::accesskit::Role::Label,
                    &stretch.len().to_string()
                )
                .count(),
            1,
            "every matched stretch states how many samples it holds"
        );
    }
    let first_sample = topmost_row_time(&harness);

    let second_match = time_rows(&harness, ResultsTable::Matches)
        .get(1)
        .map(|(rect, _)| rect.center())
        .expect("the run lists a second stretch");
    harness.press_drag_release(second_match, egui::Vec2::ZERO, 1);
    harness.run_steps(3);

    let second_sample = topmost_row_time(&harness);
    assert!(
        second_sample > first_sample,
        "the samples table lists the second stretch: {first_sample} then {second_sample}"
    );
}

/// "Copy as TSV" writes a channel run the way it writes a points query: a
/// header line naming each column in the unit the track declared, then one
/// line per matched sample.
#[test]
fn copying_a_channel_result_writes_a_tab_separated_sample_table() {
    let mut harness = channel_app_with_query_run();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, ICON_COPY)
        .click();
    harness.run_steps(1);

    let copied = copied_text(&harness);
    let mut lines = copied.lines();
    assert_eq!(
        lines.next(),
        Some("match\tsample\ttime\tx (g)\ty (g)\tz (g)")
    );
    assert_eq!(
        lines.next(),
        Some("1\t60\t11:34:20.000\t1.500\t0.100\t0.980")
    );
    let samples: usize = ACCEL_HIGH_RANGES.iter().map(Range::len).sum();
    assert_eq!(
        copied.lines().count(),
        samples + 1,
        "one line per matched sample, under the header"
    );
}

/// A channel match's "Show on map" button is grayed out, stating on hover that
/// a sample has no position of its own.
#[test]
fn a_channel_match_states_why_it_cannot_frame_the_map() {
    let mut harness = channel_app_with_query_run();
    let button = harness
        .get_all_by_role_and_label(egui::accesskit::Role::Button, ICON_FRAME_CORNERS)
        .next()
        .expect("every matched stretch offers its own button");
    assert!(
        button.accesskit_node().is_disabled(),
        "a sample range cannot frame the map"
    );

    let center = button.rect().center();
    harness.hover_at_and_settle(center, 5);
    assert_eq!(
        harness
            .query_all_by_label_contains("Channel samples have no position")
            .count(),
        1,
        "the disabled button states why"
    );
}

/// The query results' "Show on map" frames the map on what the run drew: the
/// viewport narrows from the whole recording to the matched stretches.
#[test]
fn show_on_map_frames_the_query_matches() {
    let mut harness = demo_app_with_query_run("points | where velocity > 25 km/h");

    let whole_trip = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the loaded recording");

    harness.get_by_label_contains("Show on map").click();
    harness.run_steps(3);

    let framed_matches = harness
        .state()
        .map
        .viewport_geo_bounds()
        .expect("the map framed the matches");
    assert!(
        framed_matches.lon_max - framed_matches.lon_min < whole_trip.lon_max - whole_trip.lon_min,
        "the matched stretches frame tighter than the whole trip: \
         {framed_matches:?} against {whole_trip:?}"
    );
}

/// Hovering a row of the matches table cross-highlights the whole match: its
/// range lands in `hover_match` (the map halo band and the plot time band read
/// it) and the match's track gets hover focus.
#[test]
fn query_match_row_hover_highlights_the_match() {
    use gt_ui_types::HighlightScope;

    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h".to_owned());
    }
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness);
    harness.run_steps(3);

    let match_row = topmost_match_row(&harness).0.center();
    harness.hover_at(match_row);
    harness.run_steps(2);

    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let highlight = harness.state().shared.borrow().highlight;
    let hover_match = highlight.hover_match.expect("the row hover sets the match");
    assert_eq!(hover_match.track, track);
    assert!(
        hover_match.start < hover_match.end,
        "the hovered match covers a non-empty range"
    );
    assert_eq!(
        highlight.hover,
        Some(HighlightScope::Track(track)),
        "the match's track gets hover focus"
    );

    // Pointer off the row: the cross-highlight clears the next frame.
    harness.hover_at(egui::pos2(1.0, 1.0));
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .highlight
            .hover_match
            .is_none(),
        "the highlight clears when the pointer leaves the row"
    );
}

/// Anything that is not a recording is read as a log, so binary junk fails as
/// one: nothing in it carries a timestamp.
#[test]
fn drag_drop_binary_junk_reports_it_is_not_a_recognised_log() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(b"\xff\xfe\x00binary_junk".as_slice(), "mystery.bin"),
    );

    let error = harness.state().load_error.clone().unwrap_or_default();
    assert!(
        error.starts_with("Not a recognised log: no line has a timestamp in a known format"),
        "got {error:?}"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);
    assert_eq!(harness.state().logs.len(), 0);
}

#[test]
fn panel_detached_renders_without_panic() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    assert!(!harness.state().shared.borrow().tree.detached);

    harness.state_mut().shared.borrow_mut().tree.detached = true;
    harness.step();
    assert!(harness.state().shared.borrow().tree.detached);
}

/// Guard against blocking render paths in the detached panel.
///
/// # Background: the Wayland deadlock
///
/// The original implementation used `ctx.show_viewport_immediate()` to open
/// the panel in a real OS window.  On Wayland, eframe's wgpu painter calls
/// `pollster::block_on(painter.set_window(viewport_id, Some(window)))` once
/// per viewport per frame.  When a Wayland compositor suspends frame delivery
/// to a window (because it was minimised or moved behind another window),
/// that future never resolves and the call blocks forever, freezing the whole
/// application.  This code path is still present and unfixed in eframe 0.34.2.
///
/// The fix is to avoid creating a separate OS surface for the panel at all.
/// `Window` renders the detached panel as a floating overlay inside the
/// *same* OS window, so there is only one Wayland surface - the compositor
/// cannot suspend it independently of the main window.
///
/// # What this test checks
///
/// `egui_kittest` is headless. It cannot trigger the real Wayland deadlock.
/// What it *can* do is verify that the detached panel code path completes
/// each frame quickly and does not introduce any O(n²) loops or accidentally
/// blocking operations that would manifest even in a headless runner.
/// If a future change re-introduces a blocking call, this test will time out.
#[test]
fn detached_panel_steps_complete_within_time_budget() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    harness.state_mut().shared.borrow_mut().tree.detached = true;

    // 50 consecutive steps must all finish within 10 seconds total.
    // In a healthy headless runner each step takes well under 1 ms. The
    // budget is generous to survive slow CI machines.
    let deadline = Instant::now() + StdDuration::from_secs(10);
    for _ in 0..50 {
        assert!(
            Instant::now() < deadline,
            "step deadline exceeded - likely a blocking call in the detached panel render path"
        );
        harness.step();
    }

    // Docking must also work cleanly after repeated detached rendering.
    harness.state_mut().shared.borrow_mut().tree.detached = false;
    harness.step();
    assert!(!harness.state().shared.borrow().tree.detached);
}

/// Regression: the settings window used to close immediately after opening
/// because `clicked_elsewhere()` fired on the same frame as the button click.
#[test]
fn settings_window_stays_open_after_step() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step(); // initial render
    harness.state_mut().settings_open = true;
    harness.step(); // frame where window is first shown
    assert!(
        harness.state().settings_open,
        "settings window must stay open after opening"
    );
    harness.step(); // second frame - must still be open with no interaction
    assert!(
        harness.state().settings_open,
        "settings window must remain open across multiple frames"
    );
}

fn press_escape<State>(harness: &mut Harness<'_, State>) {
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}

#[test]
fn settings_window_closes_on_esc() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.step(); // window open
    press_escape(&mut harness);
    harness.step();
    assert!(
        !harness.state().settings_open,
        "ESC must close the settings window"
    );
}

/// Builds a harness with three overlapping files loaded and the plot settled,
/// shared setup for the legend drag/redock tests below.
fn harness_with_three_files_loaded() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(transient_app);
    harness.step();
    load_three_overlapping_files(&mut harness);
    harness.run_steps(20);
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 3);
    harness
}

/// Moves the legend overlay away from its docked position and expands it,
/// for tests that exercise dragging it back.
fn detach_legend(harness: &mut Harness<App>, offset: egui::Vec2) {
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.plot_state.file_legend_offset = offset;
        shared.plot_state.file_legend_collapsed = false;
    }
    harness.step();
}

/// The plot's file legend names recordings through the user's template, the
/// same source the side panel rows read - never the raw filename.
#[test]
fn plot_legend_follows_the_recording_name_template() {
    let mut harness = harness_with_three_files_loaded();
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .recording_name_template = "Rec: {filename}".to_owned();
    harness.run_steps(3);

    for name in [
        "Rec: overlap_a.gtd",
        "Rec: overlap_b.gtd",
        "Rec: overlap_c.gtd",
    ] {
        harness.get_by_label(name);
    }
    assert!(
        harness.query_by_label("overlap_a.gtd").is_none(),
        "the legend must not fall back to the raw filename"
    );
}

/// The remove-confirmation dialog names what it is about to remove the same
/// way every other surface does.
#[test]
fn remove_confirmation_follows_the_recording_name_template() {
    let mut harness = harness_with_three_files_loaded();
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.recording_name_template = "Rec: {filename}".to_owned();
        shared.tree.delete_confirm = Some(gt_side_panel::DeleteConfirmState {
            items: vec![gt_side_panel::NodeKey::Track(TrackRef::new(
                FileIdx::new(0),
                TrackIdx::new(0),
            ))],
            delete_permanently: false,
        });
    }
    harness.run_steps(3);

    harness.get_by_label_contains("Rec: overlap_a.gtd / #1");
}

#[test]
fn legend_redock_icon_resets_offset_to_default() {
    let mut harness = harness_with_three_files_loaded();
    detach_legend(&mut harness, egui::vec2(220.0, 120.0));

    harness.get_by_label(ICON_ARROW_LINE_UP_LEFT).click();
    harness.step();

    let offset = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        gt_plot::legend_is_docked(offset),
        "expected legend to re-dock at {:?}, got ({:.2},{:.2})",
        gt_plot::LEGEND_DOCK_OFFSET,
        offset.x,
        offset.y
    );
}

#[test]
fn dragging_files_header_moves_legend_overlay() {
    let mut harness = harness_with_three_files_loaded();

    let before = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    let start = harness.get_by_label(ICON_DOTS_SIX).rect().center();
    harness.press_drag_release(start, egui::vec2(120.0, 70.0), 1);

    let after = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        (after.x - before.x).abs() > 5.0 || (after.y - before.y).abs() > 5.0,
        "expected dragging Files header to move legend: before=({:.2},{:.2}) after=({:.2},{:.2})",
        before.x,
        before.y,
        after.x,
        after.y
    );
}

#[test]
fn dragging_files_header_far_across_many_frames_does_not_snap_back() {
    let mut harness = harness_with_three_files_loaded();

    let start = harness.get_by_label(ICON_DOTS_SIX).rect().center();
    harness.press_drag_release(start, egui::vec2(200.0, 150.0), 10);

    let offset = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        !gt_plot::legend_is_docked(offset),
        "expected legend dragged far away to stay detached, got ({:.2},{:.2})",
        offset.x,
        offset.y
    );
}

#[test]
fn dragging_legend_near_top_left_redocks_automatically() {
    let mut harness = harness_with_three_files_loaded();
    detach_legend(&mut harness, egui::vec2(220.0, 120.0));

    let start = harness.get_by_label(ICON_DOTS_SIX).rect().center();
    harness.press_drag_release(start, egui::vec2(-210.0, -110.0), 1);

    let offset = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        gt_plot::legend_is_docked(offset),
        "expected legend dropped near top-left to auto-redock at {:?}, got ({:.2},{:.2})",
        gt_plot::LEGEND_DOCK_OFFSET,
        offset.x,
        offset.y
    );
}

/// Snapshot of the app with the gold dataset loaded. Captures the side panel,
/// the map area, and the plot with the metric filter row (including the Sync
/// button, grid toggle, and metric chips).
#[test]
fn snapshot_app_with_file_loaded() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    // The app repaints continuously (map + background jobs). Run many frames
    // so the map zoom and plot layout converge before we snapshot.
    harness.inner.run_steps(60);

    // Use per-test tolerance: this snapshot includes live map/plot rendering,
    // so allow tiny pixel-level variance across runs and platforms.
    harness.snapshot_with_tolerance("app_with_file_loaded", 2.5, 4);
}

/// The load warning as the user meets it: the toast the application raises
/// for a recording the archives place a disturbance in, over a loaded
/// recording.
#[test]
fn snapshot_space_weather_warning_toast() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    harness.inner.run_steps(60);

    harness
        .state_mut()
        .toasts
        .warning(super::space_weather_warning::LOAD_WARNING);
    // Two frames put the toast past its slide-in without reaching its expiry.
    harness.inner.step();
    harness.inner.step();

    harness.snapshot_loose("space_weather_warning_toast");
}

/// Every level the map's environment indicator lists, and the reference
/// window a row's link opens on that metric's material.
#[test]
fn the_map_warning_levels_open_their_reference_windows() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.run_steps(2);

    harness
        .inner
        .get_by_label(egui_phosphor::regular::CLOUD_LIGHTNING)
        .click();
    harness.inner.run_steps(2);

    for level in &*super::space_weather_warning::WARNING_LEVELS {
        assert!(
            harness
                .inner
                .query_by_label_contains(&level.trigger)
                .is_some(),
            "the popup never states {:?}",
            level.trigger
        );
    }

    let interference = gt_jam::reference::AIRCRAFT_INTERFERENCE;
    harness
        .inner
        .get_by_label_contains(interference.link_question)
        .click();
    harness.inner.run_steps(2);

    assert!(harness.inner.state().reference_window.is_open());
    assert!(
        harness
            .inner
            .query_all_by_label_contains(interference.title)
            .next()
            .is_some(),
        "the reference window shows its title"
    );
}

/// The same loaded-file view under the light theme, so the side panel, chip row,
/// and plot are all exercised on a light background - the general light-mode
/// baseline alongside the plot- and badge-specific ones.
#[test]
fn snapshot_app_with_file_loaded_light() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    harness.inner.ctx.set_theme(egui::ThemePreference::Light);
    harness.inner.run_steps(60);

    harness.snapshot_with_tolerance("app_with_file_loaded_light", 2.5, 4);
}

/// Snapshot of the app zoomed into the cluster of Sahara desert tracks from
/// the gold dataset. All other tracks (antimeridian, southern hemisphere, etc.)
/// are hidden so only the closely-spaced Sahara tracks fill the map.
#[test]
fn snapshot_app_sahara_tracks() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );

    // Identify the Sahara tracks by latitude: they are all centred around
    // 23°N 13°E. The bounding_box is in (lon, lat) geo-types order, so
    // min.y / max.y are the south/north latitudes.
    let sahara_tracks: Vec<TrackRef> = {
        let state = harness.inner.state().shared.borrow();
        state.loaded_files[0]
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.metadata.bounding_box.min().y > 20.0)
            .map(|(i, _)| TrackRef {
                fi: FileIdx::new(0),
                index: TrackIdx::new(i),
            })
            .collect()
    };

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.tree.show_only_tracks(&sahara_tracks);
        state.zoom_to_visible_request = true;
    }

    harness.inner.run_steps(60);

    harness.snapshot_loose("app_sahara_tracks");
}

/// Snapshot of the demo trip along the Paris quays: a single track with a
/// 59 s tunnel fix-loss rendered as a dashed ghost stretch, custom and
/// event markers, and multi-constellation satellite data driving the
/// fix-quality colors. This is the screenshot embedded in README.md.
#[test]
fn snapshot_app_demo_trip() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }

    harness.inner.hover_at(egui::Pos2 { x: 480., y: 200. });

    // The app repaints continuously (map + background jobs). Run many frames
    // so the map zoom and plot layout converge before we snapshot.
    harness.inner.run_steps(60);

    harness.snapshot_loose("app_demo_trip");
}

/// Snapshot of the query window end to end over the demo trip: highlighted
/// editor text, the tab strip on its results, a run whose matches draw as
/// halos on the map, the run summary, and the match table filling the window.
#[test]
fn snapshot_app_query_window() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window.set_text(
        "points\n| window 10\n| where avg(velocity) > 25 km/h # demo\n| table time, velocity"
            .to_owned(),
    );
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    // The run executes on a worker thread, so step until its results land.
    step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    let match_count: usize = {
        let app = harness.inner.state();
        app.query_window.matches().map_or(0, |m| {
            m.draws
                .iter()
                .flat_map(|d| d.ranges.values())
                .map(Vec::len)
                .sum()
        })
    };
    assert!(match_count > 0, "the demo trip has stretches above 25 km/h");

    let history_len = harness.inner.state().query_window.history().len();
    assert_eq!(history_len, 1, "the run above is recorded in history");

    harness.snapshot_loose("app_query_window");
}

/// The same run with its matches popped out: the list fills a window of its
/// own and the results tab is left to the picked match's rows.
#[test]
fn snapshot_app_query_matches_window() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window.set_text(TWO_MATCH_QUERY.to_owned());
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    pop_out_button(&harness.inner).click();
    harness.inner.run_steps(10);

    harness.snapshot_loose("app_query_matches_window");
}

/// The query editor under the light theme, so the syntax-highlight colours
/// (keywords, numbers, idents, comments) are verified on the white editor
/// background where the dark-tuned palette was unreadable. Focused on the
/// editor: it sets highlighted text but does not run the query.
#[test]
fn snapshot_app_query_editor_light() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    harness.inner.ctx.set_theme(egui::ThemePreference::Light);
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    // Exercises every token class: keywords, numeric literals, a unit, idents,
    // and a comment.
    app.query_window.set_text(
        "points\n| window 10\n| where avg(velocity) > 25 km/h # keep the fast bits\n| table time, velocity"
            .to_owned(),
    );
    harness.inner.run_steps(8);

    harness.snapshot_loose("app_query_editor_light");
}

/// Hovering a match header in the results table: the map draws the highlight
/// blue halo band over the matched stretch and the plot shades the match's
/// time span.
#[test]
fn snapshot_app_query_match_hover() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window
        .set_text("points | window 10 | where avg(velocity) > 25 km/h".to_owned());
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    // Hover the larger match's row. The cross-highlight lands on the map and
    // plot a frame later.
    let match_row = topmost_match_row(&harness.inner).0.center();
    harness.inner.hover_at(match_row);
    harness.inner.run_steps(10);

    let hover_match = harness.inner.state().shared.borrow().highlight.hover_match;
    assert!(
        hover_match.is_some(),
        "hovering the header cross-highlights the match"
    );

    harness.snapshot_loose("app_query_match_hover");
}

/// Channels plot as their own toggleable category: the Channels toggle
/// reveals the accel chip and its component lines, the chip hides them
/// again, and both toggles round-trip. The demo trip carries a 25 Hz accel
/// channel, so the snapshot shows real IMU-shaped lines beneath the metrics.
#[test]
fn snapshot_app_plot_channels() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    harness.inner.run_steps(5);

    // Hidden by default: no channel chip until the section is revealed.
    assert!(
        harness.inner.query_by_label_contains("accel (g)").is_none(),
        "channel chips stay hidden while the section is collapsed"
    );

    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);
    assert!(
        harness
            .inner
            .state()
            .shared
            .borrow()
            .plot_state
            .show_channels,
        "the toggle reveals the channel section"
    );
    harness.inner.get_by_label_contains("accel (g)");

    // Declutter: keep only velocity and the channel visible so the snapshot
    // reads clearly (the accel lines sit near 1 g among km/h magnitudes).
    {
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::Velocity);
        }
    }
    harness.inner.run_steps(5);
    harness.snapshot_loose("app_plot_channels");

    // The chip toggles the channel's lines off without collapsing the section.
    harness.inner.get_by_label_contains("accel (g)").click();
    harness.inner.run_steps(3);
    let shared = harness.inner.state().shared.borrow();
    assert!(
        !shared.plot_state.channel_vis.is_visible("accel"),
        "clicking the chip hides the channel"
    );
    assert!(
        shared.plot_state.show_channels,
        "the section stays revealed"
    );
}

/// Light-theme plot render with a spread of series enabled. The plot canvas is
/// pure-ish white on a light theme, where the dark-mode series palette was
/// invisible. This is the baseline that guards the theme-aware `metric_color`
/// light variants so a regression there fails CI.
#[test]
fn snapshot_app_plot_light() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    // Force the light theme (startup settings default to system/dark in tests).
    harness.inner.ctx.set_theme(egui::ThemePreference::Light);

    // Enable a spread of seen/fix series across constellations so the snapshot
    // exercises the light palette's constellation coding and the seen-vs-fix
    // depth separation. (Util/slip families are advanced-gated and covered by
    // the contrast test rather than shown here.)
    {
        use gt_types::MetricKind as M;
        let shown = [
            M::SatsSeen,
            M::SatsFix,
            M::GpsSeen,
            M::GpsFix,
            M::GlonassSeen,
            M::GalileoSeen,
            M::BeidouSeen,
            M::Velocity,
        ];
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in M::iter() {
            vis.set(kind, shown.contains(&kind));
        }
    }
    harness.inner.run_steps(8);

    harness.snapshot_loose("app_plot_light");
}

/// A channel-source query end to end: filtering on a vector channel's
/// component (`@accel.x`) runs standalone, the results list the matched
/// samples in one table (time plus each component) under a name row per
/// matched stretch, and the map halos the track segments those samples cover.
#[test]
fn snapshot_app_query_channel_source() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(accel_channel_gtd_bytes(28.0), "accel_demo.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    app.query_window
        .set_text("@accel | where @accel.x > 1 g".to_owned());
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    // The matches table lists a row per crafted stretch, the samples of the
    // first one fill the table below it, and the map draws halos over the
    // segments those samples cover.
    let first_stretch = ACCEL_HIGH_RANGES.first().expect("two crafted stretches");
    harness.inner.get_by_label_contains(&format!(
        "Match 1 {MIDDLE_DOT} {} samples",
        first_stretch.len()
    ));

    harness.snapshot_loose("app_query_channel_source");
}

/// A points-source query that references a channel: the window's time span
/// collects the `@accel` samples, `max(norm(@accel))` reduces them, and the
/// matches land as point ranges - the results table shows the query's metric
/// columns and the map halos the matched stretches. The mixed flow, distinct
/// from the standalone channel source above.
#[test]
fn snapshot_app_query_points_with_channel() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(accel_channel_gtd_bytes(28.0), "accel_demo.gtd"),
    );

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    let app = harness.inner.state_mut();
    app.query_window.open = true;
    // Hard-maneuver detection: the fixture's baseline norm sits near 1 g
    // (gravity on z), the crafted stretches reach ~1.8 g.
    app.query_window.set_text(
        "points | window 10 | where max(norm(@accel)) > 1.2 g | table time, velocity".to_owned(),
    );
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    // The high-accel stretches match as point ranges on the track.
    let matches = harness
        .inner
        .state()
        .query_window
        .matches()
        .expect("the run produced matches")
        .clone();
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    assert!(
        !matches.draws[0].ranges_for(track).is_empty(),
        "the matched windows halo the track"
    );

    // Park the pointer off the table so the hovered-match cross-highlight (its
    // own snapshot) does not blend into this one.
    harness.inner.hover_at(egui::pos2(1.0, 1.0));
    harness.inner.run_steps(5);

    harness.snapshot_loose("app_query_points_with_channel");
}

/// Several queries compose in one editor: a `hide` filter plus two colored
/// `draw` layers, evaluated in sequence over the demo trip.
#[test]
fn snapshot_query_pipeline() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();
    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        // Hide the slow stretches, then outline the fast and the very-fast
        // survivors in two colors.
        app.query_window.set_text(
            "points | where velocity < 20 km/h | hide\n\n\
             points | where velocity > 30 km/h | draw\n\n\
             points | where velocity > 80 km/h | draw"
                .to_owned(),
        );
    }
    harness.inner.run_steps(5);
    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    {
        let matches = harness
            .inner
            .state()
            .query_window
            .matches()
            .expect("the pipeline produced a result");
        assert!(
            !matches.hidden.is_empty(),
            "the hide query removed the slow points"
        );
        assert_eq!(matches.draws.len(), 2, "two draw queries, two halo layers");
    }

    harness.snapshot_loose("query_pipeline");
}

/// The query history survives the settings flush/load roundtrip: a run is
/// captured by `collect_settings_for_flush` and restored by
/// `apply_startup_settings`.
#[test]
fn query_history_persists_across_settings_roundtrip() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );

    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h".to_owned());
    }
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness);
    harness.run_steps(3);

    // The flushed settings carry the run, and re-applying them restores it.
    let flushed = harness.state().collect_settings_for_flush();
    assert_eq!(flushed.query.history.len(), 1);
    assert_eq!(
        flushed.query.history[0].text,
        "points | where velocity > 1 km/h"
    );

    harness.state_mut().apply_startup_settings(&flushed);
    assert_eq!(harness.state().query_window.history().len(), 1);
}

/// An added TEC mirror reaches the settings file and points the scheduler at
/// the whole list on the way back in.
#[test]
fn tec_mirrors_persist_across_settings_roundtrip() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let before = harness.state().collect_snapshot();

    harness
        .state_mut()
        .tec_settings
        .mirrors
        .add(gt_ionex::Mirror::new(
            gt_ionex::MirrorBaseUrl::new("https://mirror.example"),
            gt_ionex::MirrorLayout::Jpl,
        ));

    assert!(
        harness.state().collect_snapshot() != before,
        "the autosaver sees the edited list"
    );
    let flushed = harness.state().collect_settings_for_flush();
    let toml = toml::to_string(&flushed).expect("settings serialize");
    let reloaded: crate::settings::Settings = toml::from_str(&toml).expect("settings parse");
    assert_eq!(reloaded.tec.mirrors, flushed.tec.mirrors);

    harness.state_mut().apply_startup_settings(&reloaded);
    assert_eq!(
        harness
            .state()
            .tec_settings
            .mirrors
            .as_slice()
            .iter()
            .map(|mirror| mirror.base_url.to_string())
            .collect::<Vec<_>>(),
        [
            gt_ionex::DEFAULT_BASE_URL,
            gt_ionex::cddis::DEFAULT_BASE_URL,
            "https://mirror.example",
        ]
    );
}

/// Channel plot toggles survive the settings flush/load roundtrip: the
/// revealed section and a hidden channel come back, and the TOML encoding
/// itself round-trips the dynamic name map.
#[test]
fn plot_channel_toggles_persist_across_settings_roundtrip() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    {
        let shared = harness.state_mut().shared.clone();
        let mut shared = shared.borrow_mut();
        shared.plot_state.show_channels = true;
        shared.plot_state.channel_vis.set("accel", false);
    }

    let flushed = harness.state().collect_settings_for_flush();
    assert!(flushed.plot.show_channels);
    assert_eq!(flushed.plot.channel.get("accel"), Some(&false));

    // Through the actual wire format, not just the struct.
    let toml = toml::to_string(&flushed).expect("settings serialize");
    let reloaded: crate::settings::Settings = toml::from_str(&toml).expect("settings parse");
    assert!(reloaded.plot.show_channels);
    assert_eq!(reloaded.plot.channel.get("accel"), Some(&false));

    harness.state_mut().apply_startup_settings(&reloaded);
    let shared = harness.state().shared.borrow();
    assert!(shared.plot_state.show_channels);
    assert!(!shared.plot_state.channel_vis.is_visible("accel"));
    assert!(shared.plot_state.channel_vis.is_visible("incline"));
}

/// The sparse<->dense component color conversions: empty stays empty, an
/// index gap widens with unset slots, and only overridden slots are stored.
#[test]
fn component_color_conversions_handle_gaps_and_empty_input() {
    use crate::app::{dense_component_colors, sparse_component_colors};
    use crate::settings::ComponentColor;

    assert!(dense_component_colors(&[]).is_empty());
    assert!(sparse_component_colors(&[None, None]).is_empty());

    let red = egui::Color32::from_rgb(255, 0, 0);
    let dense = dense_component_colors(&[ComponentColor {
        component: 2,
        rgba: red.to_array(),
    }]);
    assert_eq!(dense, vec![None, None, Some(red)], "gaps widen with unset");
    assert_eq!(
        sparse_component_colors(&dense),
        vec![ComponentColor {
            component: 2,
            rgba: red.to_array(),
        }]
    );
}

/// A picked component color persists through the actual settings wire
/// format, non-overridden slots included: the dense plot slots convert to
/// sparse stored entries and back without drift.
#[test]
fn channel_component_colors_persist_across_settings_roundtrip() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    let magenta = egui::Color32::from_rgb(255, 0, 200);
    {
        let shared = harness.state_mut().shared.clone();
        let mut shared = shared.borrow_mut();
        shared
            .plot_state
            .channel_component_colors
            .insert("accel".to_owned(), vec![None, Some(magenta), None]);
    }

    let flushed = harness.state().collect_settings_for_flush();
    let toml = toml::to_string(&flushed).expect("settings serialize");
    let reloaded: crate::settings::Settings = toml::from_str(&toml).expect("settings parse");
    harness.state_mut().apply_startup_settings(&reloaded);

    let shared = harness.state().shared.borrow();
    let colors = shared
        .plot_state
        .channel_component_colors
        .get("accel")
        .expect("override survives the roundtrip");
    assert_eq!(colors.first(), Some(&None), "unset slots stay unset");
    assert_eq!(colors.get(1), Some(&Some(magenta)));
}

/// The display mask persists through the actual settings wire format:
/// hidden categories survive the round trip, missing keys mean visible.
#[test]
fn display_mask_persists_across_settings_roundtrip() {
    use gt_ui_types::DisplayCategory;

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    {
        let shared = harness.state_mut().shared.clone();
        let mut shared = shared.borrow_mut();
        shared
            .display_mask
            .set_visible(DisplayCategory::GeneratedMarkers, false);
        shared
            .display_mask
            .set_visible(DisplayCategory::SatelliteLabels, false);
    }

    let flushed = harness.state().collect_settings_for_flush();
    let toml = toml::to_string(&flushed).expect("settings serialize");
    let reloaded: crate::settings::Settings = toml::from_str(&toml).expect("settings parse");

    harness.state_mut().apply_startup_settings(&reloaded);
    let shared = harness.state().shared.borrow();
    assert!(
        !shared
            .display_mask
            .is_visible(DisplayCategory::GeneratedMarkers)
    );
    assert!(
        !shared
            .display_mask
            .is_visible(DisplayCategory::SatelliteLabels)
    );
    assert!(shared.display_mask.is_visible(DisplayCategory::Tracks));

    // A config from before the display mask existed loads with every
    // category at its default: everything visible but the opt-in layer.
    let old_config: crate::settings::Settings =
        toml::from_str("[map]\nsync_to_map = false\n").expect("old config parses");
    assert_eq!(
        old_config.map.display_mask,
        gt_ui_types::DisplayMask::default()
    );
    assert!(
        !old_config
            .map
            .display_mask
            .is_visible(DisplayCategory::JammingHexes)
    );
}

/// The interference layer is off until enabled, and stays on once it is.
#[test]
fn showing_the_interference_layer_persists_across_settings_roundtrip() {
    use gt_ui_types::DisplayCategory;

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    assert!(
        !harness
            .state()
            .shared
            .borrow()
            .display_mask
            .is_visible(DisplayCategory::JammingHexes),
        "off on a fresh install"
    );

    {
        let shared = harness.state_mut().shared.clone();
        let mut shared = shared.borrow_mut();
        shared
            .display_mask
            .set_visible(DisplayCategory::JammingHexes, true);
    }

    let flushed = harness.state().collect_settings_for_flush();
    let toml = toml::to_string(&flushed).expect("settings serialize");
    let reloaded: crate::settings::Settings = toml::from_str(&toml).expect("settings parse");
    harness.state_mut().apply_startup_settings(&reloaded);

    assert!(
        harness
            .state()
            .shared
            .borrow()
            .display_mask
            .is_visible(DisplayCategory::JammingHexes),
        "the choice survives a restart"
    );
}

/// Build an app with one loaded file and the query window open. Shared setup
/// for the interactive query-history tests.
fn app_with_query_window_open() -> Harness<'static, App> {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    harness
}

/// Run a query through the Run button and wait for its result.
fn run_query(harness: &mut Harness<App>, text: &str) {
    harness.state_mut().query_window.set_text(text.to_owned());
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(harness);
    harness.run_steps(3);
}

/// "Reset filters" clears the query filter too, not just the global filter, so
/// the map fully returns to normal.
#[test]
fn reset_filters_clears_the_query_filter() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h");
    assert!(
        harness.state().query_window.filter_active(),
        "the run produced an active query filter"
    );

    // Close the window so it can't overlap the side panel's reset button.
    harness.state_mut().query_window.open = false;
    harness.run_steps(2);
    harness.get_by_label_contains("Reset filters").click();
    harness.run_steps(3);

    assert!(
        harness.state().query_window.matches().is_none(),
        "Reset filters drops the query results"
    );
    assert!(!harness.state().query_window.filter_active());
}

/// Toggling pin flips the entry's pin and deleting removes it - the two
/// interactive history mutations, driven through the widgets.
#[test]
fn query_history_pin_and_delete_via_ui() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h");

    harness.get_by_label("Query history").click();
    harness.run_steps(3);

    let revision_before = harness.state().query_window.history_revision();
    harness.get_by_label(ICON_PUSH_PIN).click();
    harness.run_steps(3);
    {
        let window = &harness.state().query_window;
        assert!(window.history()[0].pinned, "clicking pin pins the entry");
        assert!(
            window.history_revision() > revision_before,
            "pinning bumps the revision so settings flush"
        );
    }

    harness.get_by_label(ICON_X).click();
    harness.run_steps(3);
    assert!(
        harness.state().query_window.history().is_empty(),
        "clicking the delete button removes the entry"
    );
}

/// Clicking an example fills the editor with its text and does not run.
#[test]
fn query_example_loads_into_editor_without_running() {
    let mut harness = app_with_query_window_open();

    harness.get_by_label("Examples").click();
    harness.run_steps(3);
    harness.get_by_label("Weak fix").click();
    harness.run_steps(3);

    let window = &harness.state().query_window;
    assert_eq!(window.text(), "points\n| where sats_fix < 6");
    assert!(
        window.matches().is_none() && window.history().is_empty(),
        "loading an example only fills the editor - it never runs"
    );
}

/// Ctrl+Enter (Cmd+Enter) runs the current query, mirroring the Run button.
#[test]
fn query_ctrl_enter_runs() {
    let mut harness = app_with_query_window_open();
    harness
        .state_mut()
        .query_window
        .set_text("points | where velocity > 1 km/h".to_owned());
    harness.run_steps(3);

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.step();
    step_until_query_result(&mut harness);
    harness.run_steps(3);

    assert!(
        harness.state().query_window.matches().is_some(),
        "Ctrl+Enter starts a run"
    );
    assert_eq!(
        harness.state().query_window.history().len(),
        1,
        "the Ctrl+Enter run is recorded in history"
    );
}

/// A key-press event for the given key with no modifiers.
fn key_press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Open the query window with `text`, focus the editor with the caret at the
/// end, and step until the autocomplete popup has candidates.
fn editor_with_popup(text: &str) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window.set_text(text.to_owned());
    }
    harness.run_steps(2);
    focus_query_editor_at_end(&harness, text);
    harness.run_steps(3);
    harness
}

/// Enter accepts the highlighted candidate, replacing the partial word. A
/// stage keyword gets a trailing space so the next token can be typed straight
/// away.
#[test]
fn autocomplete_enter_accepts_top_candidate() {
    let mut harness = editor_with_popup("points | wh");
    assert_eq!(
        harness.state().query_window.autocomplete_names(),
        vec!["where".to_owned(), "with".to_owned()]
    );

    harness.input_mut().events.push(key_press(egui::Key::Enter));
    harness.run_steps(2);

    assert_eq!(harness.state().query_window.text(), "points | where ");
}

/// Arrow keys move the selection before Enter accepts it.
#[test]
fn autocomplete_arrow_down_then_enter_accepts_second() {
    let mut harness = editor_with_popup("points | wh");

    harness
        .input_mut()
        .events
        .push(key_press(egui::Key::ArrowDown));
    harness.run_steps(1);
    harness.input_mut().events.push(key_press(egui::Key::Enter));
    harness.run_steps(2);

    assert_eq!(harness.state().query_window.text(), "points | with ");
}

/// Esc dismisses the popup without editing the text or closing the window, and
/// the popup stays closed until the text changes again.
#[test]
fn autocomplete_esc_dismisses_without_editing() {
    let mut harness = editor_with_popup("points | wh");

    harness
        .input_mut()
        .events
        .push(key_press(egui::Key::Escape));
    harness.run_steps(2);

    let window = &harness.state().query_window;
    assert!(
        window.autocomplete_names().is_empty(),
        "Esc closes the popup"
    );
    assert_eq!(
        window.text(),
        "points | wh",
        "Esc leaves the text unchanged"
    );
    assert!(window.open, "Esc dismisses the popup, not the window");
}

/// The blank line after a query stays quiet - no `points` popup before a
/// character is typed - and continuation typing (`| …`) is analyzed in the
/// context of the chunk above, not as a fresh query.
#[test]
fn no_popup_on_the_blank_line_after_a_query() {
    let mut harness = editor_with_popup("points\n");
    assert!(
        harness.state().query_window.autocomplete_names().is_empty(),
        "the empty line after a query must not pop `points`"
    );

    harness
        .input_mut()
        .events
        .push(egui::Event::Text("| d".to_owned()));
    harness.run_steps(3);
    let names = harness.state().query_window.autocomplete_names();
    assert!(
        names.iter().any(|n| n == "draw"),
        "continuation typing completes stage keywords in context: {names:?}"
    );
}

/// An eagerly opened empty-prefix popup (units after a number) is passive:
/// Enter still breaks the line instead of inserting a unit.
#[test]
fn passive_unit_popup_lets_enter_break_the_line() {
    let mut harness = editor_with_popup("points | where velocity > 30");
    let names = harness.state().query_window.autocomplete_names();
    assert_eq!(
        names.first().map(String::as_str),
        Some("km/h"),
        "the unit popup is open on the empty prefix: {names:?}"
    );

    harness.input_mut().events.push(key_press(egui::Key::Enter));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | where velocity > 30\n",
        "Enter breaks the line; the passive popup does not claim it"
    );
}

/// Tab accepts even a passive popup, and a unit accepted directly after a
/// digit gets its separating space.
#[test]
fn tab_accepts_a_passive_unit_with_a_separating_space() {
    let mut harness = editor_with_popup("points | where velocity > 30");
    harness.input_mut().events.push(key_press(egui::Key::Tab));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | where velocity > 30 km/h"
    );
}

/// Accepting a function inserts its parentheses with the caret inside them.
#[test]
fn accepting_a_function_inserts_parentheses() {
    let mut harness = editor_with_popup("points | window 3 | where av");
    harness.input_mut().events.push(key_press(egui::Key::Enter));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | window 3 | where avg()"
    );
    // Typing lands inside the parentheses.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("velocity".to_owned()));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | window 3 | where avg(velocity)"
    );
}

/// Ctrl+Space opens the popup on demand where the automatic path waits for a
/// typed character.
#[test]
fn ctrl_space_opens_the_popup_manually() {
    let mut harness = editor_with_popup("points | ");
    assert!(
        harness.state().query_window.autocomplete_names().is_empty(),
        "a stage position waits for the first character"
    );

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.run_steps(3);
    let names = harness.state().query_window.autocomplete_names();
    assert!(
        names.iter().any(|n| n == "where"),
        "Ctrl+Space offers the stage keywords: {names:?}"
    );
}

/// With the editor focused, Esc first unfocuses it. Only a second Esc closes
/// the query window.
#[test]
fn esc_unfocuses_the_editor_before_closing_the_window() {
    let mut harness = editor_with_popup("points | draw");
    assert!(
        harness.state().query_window.autocomplete_names().is_empty(),
        "nothing completes after a display mode"
    );

    harness
        .input_mut()
        .events
        .push(key_press(egui::Key::Escape));
    harness.run_steps(2);
    assert!(
        harness.state().query_window.open,
        "the first Esc only unfocuses the editor"
    );

    harness
        .input_mut()
        .events
        .push(key_press(egui::Key::Escape));
    harness.run_steps(2);
    assert!(
        !harness.state().query_window.open,
        "the second Esc closes the window"
    );
}

/// A standalone comment paragraph between queries is skipped, so it neither
/// errors nor blocks running the real query.
#[test]
fn comment_only_chunk_does_not_block_run() {
    let mut harness = app_with_query_window_open();
    run_query(
        &mut harness,
        "# scratch note between queries\n\npoints | where velocity > 1 km/h",
    );
    assert!(
        harness.state().query_window.matches().is_some(),
        "the comment paragraph must not disable Run"
    );
}

/// The vector channel's per-component hues and the chip's hover legend, on
/// a fixture whose y-scale keeps the three accel lines visibly apart (the
/// demo-trip snapshot squeezes them into one line against velocity's
/// scale). The tooltip maps each component color square to its name.
#[test]
fn snapshot_app_plot_channel_components() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();
    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(accel_channel_gtd_bytes(0.9), "accel.gtd"),
    );
    harness.inner.run_steps(5);

    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);
    // Metric lines (satellite counts, heading at 20 deg) dwarf the accel
    // values. Hide them all so the y-scale lets the three component lines
    // separate visibly.
    {
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        for kind in <gt_types::MetricKind as strum::IntoEnumIterator>::iter() {
            shared.plot_state.metric_vis.set(kind, false);
        }
    }
    harness.inner.run_steps(2);
    harness.inner.get_by_label_contains("accel (g)").hover();
    // Tooltips appear after egui's hover delay.
    for _ in 0..60 {
        harness.inner.run();
    }
    harness.snapshot_loose("app_plot_channel_components");
}

/// The channel chip's right-click menu carries one color entry per
/// component plus the reset - the editing surface that stays open, unlike
/// a hover tooltip.
#[test]
fn channel_chip_menu_offers_component_colors() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();
    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(accel_channel_gtd_bytes(0.9), "accel.gtd"),
    );
    harness.inner.run_steps(5);
    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);

    harness
        .inner
        .get_by_label_contains("accel (g)")
        .click_secondary();
    harness.inner.step();
    for label in ["Color of accel.x", "Color of accel.y", "Color of accel.z"] {
        assert!(
            harness.inner.query_by_label_contains(label).is_some(),
            "the chip menu should offer {label}"
        );
    }
    assert!(
        harness.inner.query_by_label("Reset colors").is_none(),
        "no reset without an override"
    );

    // With an override in place, the reset entry appears.
    harness.inner.key_press(egui::Key::Escape);
    harness.inner.run_steps(2);
    harness
        .inner
        .state_mut()
        .shared
        .borrow_mut()
        .plot_state
        .channel_component_colors
        .insert(
            "accel".to_owned(),
            vec![None, Some(egui::Color32::from_rgb(255, 0, 200)), None],
        );
    harness
        .inner
        .get_by_label_contains("accel (g)")
        .click_secondary();
    harness.inner.step();
    harness.inner.get_by_label("Reset colors").click_accesskit();
    harness.inner.run_steps(2);
    assert!(
        harness
            .inner
            .state()
            .shared
            .borrow()
            .plot_state
            .channel_component_colors
            .is_empty(),
        "reset must drop the channel's overrides"
    );
}

/// A user-picked component color reaches every surface at once: the line,
/// the chip's bar strip, and the hover legend square all draw `accel.y` in
/// the override instead of the derived hue.
#[test]
fn snapshot_app_plot_channel_color_override() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();
    drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(accel_channel_gtd_bytes(0.9), "accel.gtd"),
    );
    harness.inner.run_steps(5);

    harness.inner.get_by_label_contains("Channels").click();
    harness.inner.run_steps(3);
    {
        let state = harness.inner.state_mut();
        let mut shared = state.shared.borrow_mut();
        for kind in <gt_types::MetricKind as strum::IntoEnumIterator>::iter() {
            shared.plot_state.metric_vis.set(kind, false);
        }
        shared.plot_state.channel_component_colors.insert(
            "accel".to_owned(),
            vec![None, Some(egui::Color32::from_rgb(255, 0, 200)), None],
        );
    }
    harness.inner.run_steps(2);
    harness.inner.get_by_label_contains("accel (g)").hover();
    for _ in 0..60 {
        harness.inner.run();
    }
    harness.snapshot_loose("app_plot_channel_color_override");
}

/// The fixture stretches whose `accel` x-component exceeds 1 g, shared by the
/// value generation and the expected-match assertion so they cannot drift.
const ACCEL_HIGH_RANGES: [std::ops::Range<usize>; 2] = [60..120, 180..200];

/// Synthetic `.gtd` bytes whose track carries an aligned 3-component `accel`
/// channel in g, one sample per nav fix. The [`ACCEL_HIGH_RANGES`] stretches
/// exceed 1 g on x, so an `@accel.x` filter has multi-sample matches to table
/// on the window and halo on the map.
fn accel_channel_gtd_bytes(speed_kmh: f64) -> Vec<u8> {
    let spec = SyntheticGtdSpec {
        start: base_time(),
        point_count: 240,
        step_secs: 1,
        start_lat_deg: 55.0,
        start_lon_deg: 12.0,
        lat_step_deg: 0.00005,
        lon_step_deg: 0.00008,
        heading_deg: 20.0,
        speed_kmh,
        eph_m: 1.8,
        sats_seen: 14,
        sats_in_fix: 11,
    };
    let mut times = Vec::with_capacity(spec.point_count);
    let mut values = Vec::with_capacity(spec.point_count * 3);
    for i in 0..spec.point_count {
        times.push(spec.start + Duration::seconds(i as i64));
        let x = if ACCEL_HIGH_RANGES.iter().any(|r| r.contains(&i)) {
            1.5
        } else {
            0.2
        };
        values.extend([x, 0.1, 0.98]);
    }
    let channel = Channel::builder()
        .name("accel")
        .unit(Unit::G)
        .description("IMU acceleration")
        .components(["x", "y", "z"])
        .times(times)
        .values(values)
        .build()
        .expect("fixture channel is valid");
    gt_test_utils::synthetic_gtd_bytes_with_channels(spec, vec![channel])
}

/// A loaded file carrying one scalar channel (no points), for driving the `@`
/// completion path through the app: `schema_from_files` builds the schema the
/// popup offers from.
fn push_file_with_channel(harness: &mut Harness<App>, name: &str, unit: &str) {
    use gt_types::{Channel, FileSource, LoadedFile, LoadedTrack, TrackLod};
    let channel = Channel {
        name: name.to_owned(),
        unit: Some(ChannelUnit::from_file_label(unit)),
        period: None,
        description: None,
        components: vec![],
        times: vec![],
        values: vec![],
    };
    let file = LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks: vec![LoadedTrack {
            metadata: gt_test_utils::empty_track_metadata(),
            points: vec![],
            lod: TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
            channels: vec![channel],
        }],
        event_marker_styles: std::collections::HashMap::new(),
        orphaned_event_markers: vec![],
        source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        load_warnings: vec![],
    };
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .loaded_files
        .push(file, gt_loaded_files::FileHistory::None);
    harness.step();
}

/// The `@` channel popup, driven through the whole app path: the loaded file's
/// channel reaches the schema, the popup offers it, and accepting inserts the
/// `@name` reference.
#[test]
fn channel_popup_offers_and_inserts_a_loaded_channel() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    push_file_with_channel(&mut harness, "accel", "g");
    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window.set_text("@ac".to_owned());
    }
    harness.run_steps(2);
    focus_query_editor_at_end(&harness, "@ac");
    harness.run_steps(3);

    assert_eq!(
        harness.state().query_window.autocomplete_names(),
        vec!["@accel".to_owned()],
        "the loaded channel is offered for the typed sigil"
    );
    harness.input_mut().events.push(key_press(egui::Key::Enter));
    harness.run_steps(2);
    assert_eq!(harness.state().query_window.text(), "@accel");
}

/// A channel-source query mixed with a points query cannot run. The editor
/// says why.
#[test]
fn mixed_channel_queries_explain_why_run_is_disabled() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    push_file_with_channel(&mut harness, "accel", "g");
    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h\n\n@accel | where @accel > 1 g".to_owned());
    }
    harness.run_steps(3);

    harness.get_by_label_contains("must be the only query in the editor");
}

/// Clicking a popup row accepts that candidate (the deferred click path, not
/// the keyboard path).
#[test]
fn autocomplete_click_accepts_candidate() {
    let mut harness = editor_with_popup("points | wh");
    // The "with" row is identified by its unique summary text.
    harness
        .get_by_label_contains("set satellite-analysis parameters")
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().query_window.text(), "points | with ");
}

/// Right-clicking the toolbar query button while a filter is active offers
/// "Clear query filter", which clears it.
#[test]
fn toolbar_context_menu_clears_query_filter() {
    let mut harness = app_with_query_window_open();
    run_query(&mut harness, "points | where velocity > 1 km/h");
    // Close the window so the toolbar shows the active-filter alert.
    harness.state_mut().query_window.open = false;
    harness.run_steps(3);
    assert!(harness.state().query_window.filter_active());

    harness
        .get_by_label_contains(ICON_TERMINAL_WINDOW)
        .click_secondary();
    harness.run_steps(2);
    harness.get_by_label_contains("Clear query filter").click();
    harness.run_steps(3);

    assert!(
        !harness.state().query_window.filter_active(),
        "the context menu cleared the query filter"
    );
}

/// Focus the query editor and drop the caret at the end of `text`, so the
/// caret-driven autocomplete and hover paths run in a snapshot.
fn focus_query_editor_at_end(harness: &Harness<App>, text: &str) {
    let editor_id = egui::Id::new(super::query::EDITOR_ID_SALT);
    harness.ctx.memory_mut(|m| m.request_focus(editor_id));
    let mut state = TextEdit::load_state(&harness.ctx, editor_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(text.chars().count()),
        )));
    TextEdit::store_state(&harness.ctx, editor_id, state);
}

/// The autocomplete popup: candidates under the caret, the top one
/// highlighted, capped to five rows with a footer noting the rest.
#[test]
fn snapshot_query_autocomplete_popup() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 460.0))
        .eframe(build_app);
    harness.inner.step();

    // A prefix that matches many metrics, so the popup overflows five rows.
    let text = "points | where s";
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        app.query_window.set_text(text.to_owned());
    }
    harness.inner.run_steps(3);
    focus_query_editor_at_end(&harness.inner, text);
    harness.inner.run_steps(3);

    let names = harness.inner.state().query_window.autocomplete_names();
    assert!(
        names.len() > 5,
        "the popup overflows five rows and shows a footer, got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "sats_fix"),
        "s-metrics are offered: {names:?}"
    );

    harness.snapshot_loose("query_autocomplete_popup");
}

/// A checker error: an error icon and the red problem, then the suggestion as
/// a plain "Hint:" line below.
#[test]
fn snapshot_query_error() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 260.0))
        .eframe(build_app);
    harness.inner.step();
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        // Per-point metric in a window: the message splits into problem + hint.
        app.query_window
            .set_text("points | window 10 | where velocity >= 10 km/h".to_owned());
    }
    // Editor left unfocused so no completion popup covers the error.
    harness.inner.run_steps(3);
    // Loose: the error text rasterizes a pixel or two differently between the
    // local baseline and CI's software renderer.
    harness.snapshot_loose("query_error");
}

/// Hovering a construct in the editor shows a Rust-doc-style tooltip: name and
/// kind, summary, then the fuller explanation and an example.
#[test]
fn snapshot_query_hover_docs() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 460.0))
        .eframe(build_app);
    harness.inner.step();

    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | window 10 | where avg(velocity) > 30 km/h".to_owned());
    }
    harness.inner.run_steps(3);

    // Hover the `window` token. It starts after "points | " on the first
    // line. The editor's text begins near the top-left of the window content.
    let editor = harness
        .inner
        .get_by_role(egui::accesskit::Role::MultilineTextInput);
    let rect = editor.rect();
    let hover = egui::pos2(rect.left() + 96.0, rect.top() + 10.0);
    harness.inner.run_steps(2);
    harness.inner.hover_at(hover);
    // The hover doc appears only after the pointer has rested, so step past
    // the delay (steps advance the mock clock a frame at a time).
    harness.inner.run_steps(40);

    harness.snapshot("query_hover_docs");
}

/// The hover doc stays up while the pointer moves within its token, hides off
/// any token, and re-arms the rest delay before the next token's doc shows.
#[test]
fn query_hover_doc_sticks_within_its_token() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 460.0))
        .eframe(build_app);
    harness.inner.step();
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | window 10 | where avg(velocity) > 30 km/h".to_owned());
    }
    harness.inner.run_steps(3);

    let editor = harness
        .inner
        .get_by_role(egui::accesskit::Role::MultilineTextInput);
    let rect = editor.rect();
    // Inside the `window` token, as in `snapshot_query_hover_docs`.
    let in_token = egui::pos2(rect.left() + 96.0, rect.top() + 10.0);
    harness.inner.hover_at(in_token);
    harness.inner.run_steps(40);
    assert!(
        harness.inner.state().query_window.hover_doc_shown(),
        "the doc shows once the pointer has rested on the token"
    );

    // Nudge one character to the right, still inside `window`. The frame that
    // processes the movement has a freshly reset rest timer, so only the
    // stickiness keeps the doc up.
    harness.inner.hover_at(in_token + egui::vec2(7.0, 0.0));
    harness.inner.run_steps(1);
    assert!(
        harness.inner.state().query_window.hover_doc_shown(),
        "the doc stays up while the pointer moves within its token"
    );

    // The blank editor space below the text is off any token.
    harness
        .inner
        .hover_at(egui::pos2(rect.left() + 96.0, rect.bottom() - 10.0));
    harness.inner.run_steps(1);
    assert!(
        !harness.inner.state().query_window.hover_doc_shown(),
        "the doc hides once the pointer leaves the token"
    );

    // Back on the token: the delay is armed again, then the doc returns.
    harness.inner.hover_at(in_token);
    harness.inner.run_steps(1);
    assert!(
        !harness.inner.state().query_window.hover_doc_shown(),
        "a token entered just now waits for the pointer to rest"
    );
    harness.inner.run_steps(40);
    assert!(
        harness.inner.state().query_window.hover_doc_shown(),
        "the doc returns after the pointer rests on the token again"
    );
}

#[test]
fn snapshot_app_three_overlapping_files() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();
    load_three_overlapping_files(&mut harness.inner);
    assert_eq!(harness.inner.state().shared.borrow().loaded_files.len(), 3);

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    harness.inner.run_steps(70);
    // Full-app map+plot render, so it carries the same GPU/anti-aliasing
    // nondeterminism as the other map snapshots and uses the shared loose
    // tolerance rather than a tighter hand-picked count that the macOS runner
    // drifts past.
    harness.snapshot_loose("app_three_overlapping_files");
}

impl SettingsPage {
    /// The control this page renders last, which is the first to fall out of
    /// the window when the page grows.
    fn last_control_label(self) -> &'static str {
        match self {
            Self::Processing => "Restore defaults",
            Self::Analysis => "Mark masked-out used satellites",
            Self::AircraftInterference
            | Self::GeomagneticIndices
            | Self::IonosphericTec
            | Self::SolarFlares => crate::app::backfill_ui::DOWNLOAD_HISTORY_LABEL,
            Self::SnapToRoad => "GPS accuracy",
            Self::Interface => "Mapbox token",
            Self::Application => crate::app::environment_storage_ui::AUTO_PRUNE_LABEL,
        }
    }

    /// Gated with the snapshot test that uses it: without `self-update` the
    /// Application page renders one row fewer and no baseline matches.
    #[cfg(feature = "self-update")]
    fn snapshot_file_stem(self) -> &'static str {
        match self {
            Self::Processing => "settings_window_processing",
            Self::Analysis => "settings_window_analysis",
            Self::AircraftInterference => "settings_window_aircraft_interference",
            Self::GeomagneticIndices => "settings_window_geomagnetic_indices",
            Self::IonosphericTec => "settings_window_ionospheric_tec",
            Self::SolarFlares => "settings_window_solar_flares",
            Self::SnapToRoad => "settings_window_snap_to_road",
            Self::Interface => "settings_window_interface",
            Self::Application => "settings_window_application",
        }
    }
}

/// Opens the settings window so every page renders the same way on every run:
/// both download ranges are pinned and one snap option is set.
fn harness_with_settings_window_open<'a>() -> (TestHarness<'a, App>, PathBuf) {
    let (mut harness, config_path) = TestHarness::builder()
        .size(egui::vec2(940.0, 720.0))
        .eframe(build_app);
    harness.inner.step();
    // Search radius set (its drag value active), the other two unset (grayed,
    // never hidden): the snap page shows both states of its optional rows.
    harness.inner.state_mut().snap_settings.search_radius_m = Some(25.0);
    harness.inner.state_mut().settings_open = true;
    pin_settings_dates(harness.inner.state_mut());
    (harness, config_path)
}

// The Application page renders a `self-update`-only row (the update check), so
// the window's appearance depends on that feature. Gating the snapshot on it means
// the reference image can only ever be generated and compared in the same
// configuration CI uses (`just test` / `just test-snapshots` both enable it).
// Without this, regenerating snapshots in a build that lacks the feature would
// silently drop that page and break macOS CI. Any future feature-dependent
// snapshot must be gated the same way.
#[cfg(feature = "self-update")]
#[test]
fn snapshot_settings_pages() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    for page in SettingsPage::iter() {
        harness.inner.state_mut().settings_page = page;
        harness.run();
        harness.snapshot(page.snapshot_file_stem());
    }
}

/// The window opens at one size that holds every page at default content, and
/// keeps that size as the page changes.
#[test]
fn settings_window_keeps_one_size_across_pages() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    let mut opened_size = None;
    for page in SettingsPage::iter() {
        harness.inner.state_mut().settings_page = page;
        harness.run();

        let window_rect = harness
            .inner
            .ctx
            .memory(|m| m.area_rect(egui::Id::new(settings_ui::WINDOW_ID)))
            .expect("the settings window is open");
        let last_control = harness
            .inner
            .get_by_label_contains(page.last_control_label())
            .rect();
        assert!(
            last_control.max.y <= window_rect.max.y,
            "{:?} overflows the window: {} ends at {}, the window at {}",
            page,
            page.last_control_label(),
            last_control.max.y,
            window_rect.max.y
        );

        let opened_size = *opened_size.get_or_insert(window_rect.size());
        assert_eq!(
            window_rect.size(),
            opened_size,
            "{page:?} resized the window"
        );
    }
}

/// Types `query` into the settings window's search field. The field is focused
/// by its own id: the app behind the window renders text fields of its own,
/// which [`HarnessInteraction::type_into_text_input`] would match as well.
fn type_into_settings_search(harness: &mut TestHarness<'_, App>, query: &str) {
    harness.inner.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(settings_ui::search::QUERY_FIELD_ID));
    });
    harness.run();
    harness
        .inner
        .input_mut()
        .events
        .push(egui::Event::Text(query.to_owned()));
    harness.run();
}

/// A renamed row must not leave the search behind: every label a page declares
/// searchable is one the page renders.
#[test]
fn every_settings_page_renders_the_labels_it_declares() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    for page in SettingsPage::iter() {
        harness.inner.state_mut().settings_page = page;
        harness.run();

        let window_rect = harness
            .inner
            .ctx
            .memory(|m| m.area_rect(egui::Id::new(settings_ui::WINDOW_ID)))
            .expect("the settings window is open");
        for label in page.searchable_labels() {
            assert!(
                harness
                    .inner
                    .query_all_by_label_contains(label)
                    .any(|node| window_rect.contains_rect(node.rect())),
                "{page:?} declares {label:?} searchable but renders no such label"
            );
        }
    }
}

/// A source page's reference link opens the reference window on that source's
/// material.
#[rstest::rstest]
#[case(
    SettingsPage::GeomagneticIndices,
    gt_solar::reference::GEOMAGNETIC_ACTIVITY
)]
#[case(SettingsPage::IonosphericTec, gt_ionex::reference::IONOSPHERIC_TEC)]
#[case(SettingsPage::SolarFlares, gt_flare::reference::SOLAR_FLARES)]
#[case(
    SettingsPage::AircraftInterference,
    gt_jam::reference::AIRCRAFT_INTERFERENCE
)]
fn a_source_page_opens_its_reference_window(
    #[case] page: SettingsPage,
    #[case] document: gt_ui_types::reference::ReferenceDocument,
) {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.inner.state_mut().settings_page = page;
    harness.run();

    harness
        .inner
        .get_by_label_contains(document.link_question)
        .click();
    harness.run();

    assert!(harness.inner.state().reference_window.is_open());
    assert!(
        harness
            .inner
            .query_all_by_label_contains(document.title)
            .next()
            .is_some(),
        "the reference window shows its title"
    );
}

#[test]
fn an_empty_query_lists_every_page_in_the_rail() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    for page in SettingsPage::iter() {
        assert!(
            harness
                .inner
                .query_all_by_label_contains(page.rail_label())
                .next()
                .is_some(),
            "{page:?} is missing from the rail"
        );
    }
}

#[rstest::rstest]
#[case::lowercase("elevation")]
#[case::mixed_case("ElevAtion")]
fn clicking_a_search_match_opens_its_page(#[case] query: &str) {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    type_into_settings_search(&mut harness, query);

    assert!(
        harness
            .inner
            .query_all_by_label_contains(SettingsPage::SnapToRoad.rail_label())
            .next()
            .is_none(),
        "a page the query does not reach stays out of the rail"
    );
    assert_eq!(
        harness.inner.state().settings_page,
        SettingsPage::Processing
    );

    harness
        .inner
        .get_by_label_contains("Elevation mask")
        .click();
    harness.run();
    assert_eq!(harness.inner.state().settings_page, SettingsPage::Analysis);
}

/// One Escape press dismisses one level: the query first, the window second.
#[test]
fn escape_clears_the_query_before_it_closes_the_window() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    type_into_settings_search(&mut harness, "elevation");

    press_escape(&mut harness.inner);
    harness.run();
    assert!(
        harness.inner.state().settings_open,
        "the first Escape clears the query and leaves the window open"
    );
    assert!(
        harness
            .inner
            .query_all_by_label_contains(SettingsPage::SnapToRoad.rail_label())
            .next()
            .is_some(),
        "the cleared query restores the whole rail"
    );

    press_escape(&mut harness.inner);
    harness.run();
    assert!(
        !harness.inner.state().settings_open,
        "the second Escape closes the window"
    );
}

#[test]
fn snapshot_settings_window_search_matches() {
    let (mut harness, _config_path) = harness_with_settings_window_open();
    harness.run();
    type_into_settings_search(&mut harness, "clock");
    harness.run();
    harness.snapshot("settings_window_search_matches");
}

/// Clicks the tickbox of the settings row headed `label`.
///
/// A settings row is its label beside its control, and the tickbox carries no
/// label of its own: it is the one drawn across the label's row.
fn click_settings_row_tickbox(harness: &mut Harness<App>, label: &str) {
    let row = harness.get_by_label_contains(label).rect();
    let tickbox = harness
        .query_all(By::new().role(egui::accesskit::Role::CheckBox))
        .find(|node| row.y_range().contains(node.rect().center().y));
    match tickbox {
        Some(tickbox) => tickbox.click(),
        None => panic!("the settings row {label:?} draws a tickbox"),
    }
    harness.run_steps(2);
}

/// Load one recording and give it the metadata the name template draws on.
fn load_recording_with_metadata(harness: &mut Harness<App>) {
    drop_file_and_wait_for_load(
        harness,
        TestDroppedFile::bytes(minimal_gtd_bytes(), "ride.gtd"),
    );
    let mut shared = harness.state().shared.borrow_mut();
    if let Some(file) = shared.loaded_files.get_mut(0) {
        file.metadata.title = Some("Morning ride".to_owned());
        file.metadata.device = Some("u-blox F9P".to_owned());
    }
    shared.recording_name_template = "{title} - {device}".to_owned();
}

/// A stored recording for the guide's preview line to fall back on.
fn stored_recording_entry(identity: &str, title: &str) -> gt_store::RecordingEntry {
    gt_store::RecordingEntry {
        db_ref: gt_store::DatabaseRef {
            identity: identity.to_owned(),
            group_name: "2026-01-01T00:00:00Z_0".to_owned(),
        },
        meta: gt_store::RecordingMeta {
            start_us: 0,
            end_us: 0,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        },
        total_tracks: 1,
        hidden_tracks: 0,
        title: Some(title.to_owned()),
        device: Some("u-blox F9P".to_owned()),
        notes: None,
        travel_mode: None,
        channels: Vec::new(),
    }
}

/// The template guide opens while the field has focus and previews the template
/// twice: with every token filled by its own name, and on a real recording. A
/// loaded recording is the preview's source even when history holds one too.
#[test]
fn name_template_guide_previews_the_loaded_recording() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(transient_app);
    harness.step();
    load_recording_with_metadata(&mut harness);
    harness
        .state_mut()
        .history_window
        .set_entries(vec![stored_recording_entry(
            "auto:stored.gtd",
            "Stored ride",
        )]);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);
    assert!(
        harness.query_by_label("title - device").is_none(),
        "the guide must stay closed until the field takes focus"
    );

    harness.get_by_label_contains("Recording name").focus();
    harness.run_steps(3);

    harness.get_by_label("title - device");
    harness.get_by_label("Morning ride - u-blox F9P");
}

/// With nothing loaded, the preview falls back to the most recent recording in
/// history, which names its file after its identity.
#[test]
fn name_template_guide_previews_a_history_recording() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(transient_app);
    harness.step();
    {
        let mut older = stored_recording_entry("auto:older.gtd", "Older ride");
        older.meta.start_us = 1_000;
        let mut newest = stored_recording_entry("auto:newest.gtd", "Newest ride");
        newest.meta.start_us = 2_000;
        harness
            .state_mut()
            .history_window
            .set_entries(vec![older, newest]);
        harness.state().shared.borrow_mut().recording_name_template =
            "{title} - {identity} - {filename}".to_owned();
    }
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    harness.get_by_label_contains("Recording name").focus();
    harness.run_steps(3);

    harness.get_by_label("Newest ride - newest.gtd - newest.gtd");
}

/// Both previews follow the field as it is typed in.
#[test]
fn name_template_guide_previews_follow_the_typed_template() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(transient_app);
    harness.step();
    load_recording_with_metadata(&mut harness);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    let field = harness.get_by_label_contains("Recording name");
    field.focus();
    field.type_text(" ({identity})");
    harness.run_steps(3);

    assert_eq!(
        harness.state().shared.borrow().recording_name_template,
        "{title} - {device} ({identity})"
    );
    harness.get_by_label("title - device (identity)");
    harness.get_by_label("Morning ride - u-blox F9P (ride.gtd)");
}

/// With no recording loaded and none in history, the preview line explains why
/// it is empty.
#[test]
fn name_template_guide_explains_a_missing_recording() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    harness.get_by_label_contains("Recording name").focus();
    harness.run_steps(3);

    harness.get_by_label("No recording loaded or in history");
}

/// The Interface page's token field drives the token the map fetches satellite
/// tiles with. It is the page's last text field, below the name template.
#[test]
fn the_interface_page_edits_the_token_the_map_reads() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    let field = harness.bottommost_matching(By::new().role(egui::accesskit::Role::TextInput));
    field.focus();
    field.type_text("token-from-settings");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);

    assert_eq!(harness.state().map.mapbox_token(), "token-from-settings");
}

/// The page grays the satellite layer until a token is set, per DESIGN.md. Its
/// entry is the topmost "Satellite" match: the map's own ungated picker renders
/// the same entry lower on screen.
#[test]
fn the_interface_page_gates_the_satellite_layer_on_a_token() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(820.0, 620.0))
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Interface;
    harness.run_steps(3);

    assert!(
        harness
            .topmost_matching(By::new().label_contains("Satellite"))
            .accesskit_node()
            .is_disabled()
    );

    harness.state_mut().map.set_mapbox_token("tok".to_owned());
    harness.run_steps(2);
    harness
        .topmost_matching(By::new().label_contains("Satellite"))
        .click();
    harness.run_steps(2);

    assert_eq!(harness.state().map.layer(), gt_map::MapLayer::Satellite);
}

/// The guide as it shows while the user edits the template: the token list, an
/// example, and both preview lines. The preview recording comes from history, so
/// no track ink renders behind the window. Feature-gated like
/// `snapshot_settings_window` - it captures the settings window the guide hangs
/// off.
#[cfg(feature = "self-update")]
#[test]
fn snapshot_recording_name_template_guide() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(820.0, 620.0))
        .eframe(build_app);
    harness.inner.step();
    harness
        .inner
        .state_mut()
        .history_window
        .set_entries(vec![stored_recording_entry(
            "auto:ride.gtd",
            "Morning ride",
        )]);
    harness
        .inner
        .state()
        .shared
        .borrow_mut()
        .recording_name_template = "{title} - {device}".to_owned();
    harness.inner.state_mut().settings_open = true;
    harness.inner.state_mut().settings_page = SettingsPage::Interface;
    harness.inner.run_steps(3);
    harness
        .inner
        .get_by_label_contains("Recording name")
        .focus();
    harness.inner.run_steps(3);
    // The GL and software renderers disagree by one antialiased pixel at the
    // guide window's edge, in opposite directions, so no baseline passes both
    // at the default threshold.
    harness.snapshot_with_threshold("recording_name_template_guide", 1.5);
}

/// The update prompt as a user installed via the shell/PowerShell installer
/// sees it: a prominent "Update and restart" plus lower-key Later / Skip.
#[cfg(feature = "self-update")]
#[test]
fn snapshot_update_prompt_self_update() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 400.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().update_checker =
        super::update::UpdateChecker::available_for_test("0.2.0", true);
    harness.run();
    harness.snapshot("update_prompt_self_update");
}

/// A non-self-updatable build (Homebrew / MSI / manual download) shows no
/// dialog. It exposes the available version for the subtle menu-bar badge.
#[cfg(feature = "self-update")]
#[test]
fn non_self_update_uses_badge_not_dialog() {
    let badge = super::update::UpdateChecker::available_for_test("0.2.0", false);
    assert_eq!(badge.badge_version().as_deref(), Some("0.2.0"));

    let self_updatable = super::update::UpdateChecker::available_for_test("0.2.0", true);
    assert_eq!(self_updatable.badge_version(), None);
}

/// Settles the pointer on the widget labelled `label` and clicks it, then runs
/// the frames the click's effect needs to reach the app state.
#[cfg(feature = "self-update")]
fn click_settled(harness: &mut Harness<'_, App>, label: &str) {
    harness.get_by_label(label).hover();
    harness.run_steps(2);
    harness.get_by_label(label).click();
    harness.run_steps(3);
}

/// The storage controls appear in the History window and on the settings
/// window's Application page, both driving the one setting: what one window
/// writes, the other reads.
#[cfg(feature = "self-update")]
#[test]
fn storage_controls_drive_one_setting_from_both_windows() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(transient_app);
    harness.step();
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .reopen_history_database(&dir.path().join("recordings.h5"), &ctx);

    // The Application page turns auto-pruning on.
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    assert!(
        harness.step_until(|h| h.query_by_label("Auto-prune when over").is_some()),
        "the Application page shows the auto-prune controls"
    );
    click_settled(&mut harness, "Auto-prune when over");
    assert!(
        harness.state().storage_settings.auto_prune_enabled,
        "the Application page's auto-prune switch writes the setting"
    );

    // Clicking the History window's confirmation toggle proves it reads the
    // Application page's write and writes the same setting back: the toggle
    // only takes a click while auto-pruning is on.
    harness.state_mut().settings_open = false;
    harness.state_mut().history_window.open = true;
    assert!(
        harness.step_until(|h| h.query_by_label("Confirm before pruning").is_some()),
        "the History window shows the auto-prune controls"
    );
    click_settled(&mut harness, "Confirm before pruning");
    assert!(
        !harness.state().storage_settings.auto_prune_confirm,
        "the History window's confirmation toggle writes the setting"
    );

    // Auto-storing off in the History window empties the loader's database
    // path, the same live effect the Application page has.
    click_settled(&mut harness, AUTO_STORE_LABEL);
    assert!(
        !harness.state().storage_settings.enabled,
        "the History window's auto-store checkbox writes the setting"
    );
    assert_eq!(harness.state().loader.db_path, None);

    // The Application page reads the History window's write: its auto-store
    // checkbox turns storing back on, and the loader's path returns.
    harness.state_mut().history_window.open = false;
    harness.state_mut().settings_open = true;
    assert!(
        harness.step_until(|h| h.query_by_label(AUTO_STORE_LABEL).is_some()),
        "the Application page shows the auto-store checkbox"
    );
    click_settled(&mut harness, AUTO_STORE_LABEL);
    assert!(
        harness.state().storage_settings.enabled,
        "the Application page's auto-store checkbox writes the setting"
    );
    assert!(harness.state().loader.db_path.is_some());
}

#[test]
fn snapshot_history_locked_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(crate::app::storage::HistoryFailure::Locked(
        PathBuf::from("geotrace.h5"),
    ));
    harness.run();
    harness.snapshot("history_locked_dialog");
}

#[test]
fn snapshot_history_corrupt_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(
        crate::app::storage::HistoryFailure::Unreadable(PathBuf::from("geotrace.h5")),
    );
    harness.run();
    harness.snapshot("history_corrupt_dialog");
}

/// Startup hands the app the databases a completed open produced. The worker
/// it carries replaces the one the app was holding, and the loader takes the
/// path that worker stores under.
#[test]
fn adopting_an_open_storage_installs_its_history_worker() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let recordings_path = store.recordings_path();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    assert!(
        harness.state().history.path().is_none(),
        "the harness starts with storage disabled"
    );

    let opened = crate::app::storage::OpenStorage {
        history: crate::app::history_db::HistoryWorker::spawn(
            store.open_recordings().expect("recordings"),
            harness.ctx.clone(),
            gt_pending_writes::PendingWrites::default(),
        ),
        history_failure: None,
        archive: None,
        geomagnetic_indices: None,
        tec_maps: None,
        solar_flares: None,
        unavailable_archives: UnavailableArchives::default(),
    };
    harness.state_mut().adopt_open_storage(opened);

    assert_eq!(
        harness.state().history.path(),
        Some(recordings_path.as_path()),
        "the adopted worker is the one the app now stores through"
    );
    assert_eq!(
        harness.state().loader.db_path.as_deref(),
        Some(recordings_path.as_path()),
        "the loader stores into the adopted database"
    );
}

/// Adopting a storage-open failure has to raise its prompt itself: the open
/// reports the failure, not the app.
#[test]
fn a_history_failure_in_the_adopted_storage_raises_its_prompt() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    harness
        .state_mut()
        .adopt_open_storage(crate::app::storage::OpenStorage {
            history: crate::app::history_db::HistoryWorker::disabled(),
            history_failure: Some(crate::app::storage::HistoryFailure::Busy(PathBuf::from(
                "recordings.h5",
            ))),
            archive: None,
            geomagnetic_indices: None,
            tec_maps: None,
            solar_flares: None,
            unavailable_archives: UnavailableArchives::default(),
        });
    harness.step();

    assert!(
        harness
            .query_by_label_contains("Another process has the recording history database open")
            .is_some(),
        "the busy prompt is up"
    );
}

/// The app as it starts with its databases still opening, and the sender the
/// test lands them through. `paths` are the ones a command line named.
///
/// The open is taken over before the harness's first frame, so nothing is
/// adopted until the test says so.
fn app_with_the_databases_still_opening<'a>(
    paths: &[PathBuf],
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    app_with_the_databases_still_opening_for(paths, WriteAccess::Owner)
}

/// [`app_with_the_databases_still_opening`] for a session with the given
/// write access, which is what decides whether anything it loads is stored.
fn app_with_the_databases_still_opening_for<'a>(
    paths: &[PathBuf],
    write_access: WriteAccess,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    let (sender_tx, sender_rx) = mpsc::channel();
    let harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(|cc| {
            let mut app = transient_app_with_the_instance_lock(
                cc,
                paths,
                SharedDataDirectoryLock::marking_nothing(),
                PendingWrites::new(write_access),
            );
            sender_tx.send(app.storage_open.take_over_for_test()).ok();
            app
        });
    let databases = sender_rx.recv().expect("the app was built");
    (harness, databases)
}

/// Every database under `store`, as a finished open hands them over.
fn storage_opened_in(
    store: &gt_store::Store,
    ctx: &egui::Context,
    pending_writes: &gt_pending_writes::PendingWrites,
) -> OpenStorage {
    OpenStorage {
        history: crate::app::history_db::HistoryWorker::spawn(
            store
                .open_recordings()
                .expect("open the recordings database"),
            ctx.clone(),
            pending_writes.clone(),
        ),
        history_failure: None,
        archive: store.open_interference().ok(),
        geomagnetic_indices: store.open_geomagnetic_indices().ok(),
        tec_maps: store.open_tec_maps().ok(),
        solar_flares: store.open_solar_flares().ok(),
        unavailable_archives: UnavailableArchives::default(),
    }
}

/// Lands `store` behind the app, as the open thread does when it finishes.
fn land_the_databases(
    harness: &mut Harness<'_, App>,
    databases: &mpsc::Sender<OpenStorage>,
    store: &gt_store::Store,
) {
    let pending_writes = harness.state().pending_writes.clone();
    let opened = storage_opened_in(store, &harness.ctx, &pending_writes);
    databases.send(opened).expect("the app holds the receiver");
    harness.step();
}

/// The window is painted and takes input from the first frame, with the
/// databases still opening behind it.
#[test]
fn the_window_takes_input_while_the_databases_open_and_adopts_them_when_they_land() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);
    harness.step();

    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::Opening)
    );
    harness.get_by_label_contains(OPENING_DATABASES);
    harness.get_by_label_contains(ICON_GEAR).click();
    harness.step();
    assert!(
        harness.state().settings_open,
        "the window answered a click while the databases were opening"
    );

    land_the_databases(&mut harness, &databases, &store);

    assert_eq!(harness.state().storage_open.databases_pending(), None);
    assert_eq!(
        harness.state().history.path(),
        Some(store.recordings_path().as_path()),
        "the app stores through the database the open landed"
    );
    assert!(
        harness.query_by_label_contains(OPENING_DATABASES).is_none(),
        "the overlay went once the databases landed"
    );
}

/// A file named on the command line waits for the databases: loading it before
/// they land would leave it unstored, with nothing to store it later.
#[test]
fn a_file_named_before_the_databases_land_is_loaded_and_stored_once_they_do() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let gtd_path = dir.path().join("queued.gtd");
    std::fs::write(&gtd_path, minimal_gtd_bytes()).expect("write the recording");

    let (mut harness, databases) = app_with_the_databases_still_opening(&[gtd_path]);
    harness.run_steps(3);

    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the load waits for the databases"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the file that waited for the databases did not load"
    );
    let stored = harness
        .step_until_some(|_| {
            let recordings = Recordings::open_or_create(&store.recordings_path()).ok()?;
            let listed = recordings.list_recordings().ok()?;
            (!listed.is_empty()).then_some(listed)
        })
        .expect("the recording was never stored");
    assert_eq!(stored.len(), 1);
}

/// A read-only session stores nothing it opens: the recording is loaded into
/// the window, and the recording history beside it is left as it was.
#[test]
fn a_recording_loaded_in_a_read_only_session_is_not_stored() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let gtd_path = dir.path().join("read-only.gtd");
    std::fs::write(&gtd_path, minimal_gtd_bytes()).expect("write the recording");
    let (mut harness, databases) =
        app_with_the_databases_still_opening_for(&[gtd_path], WriteAccess::ReadOnly);
    harness.run_steps(3);

    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the recording did not load"
    );
    let recordings =
        Recordings::open_or_create(&store.recordings_path()).expect("open the recording history");
    assert_eq!(
        recordings.list_recordings().expect("list").len(),
        0,
        "the read-only session stored the recording it loaded"
    );
    assert_eq!(
        harness.state().pending_writes.snapshot().recently_finished,
        Vec::<String>::new(),
        "the read-only session registered a write"
    );
}

/// A drop that arrives before the databases waits for them the same way a
/// command-line path does.
#[test]
fn a_file_dropped_before_the_databases_land_loads_once_they_do() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);

    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(TestDroppedFile::bytes(
            minimal_gtd_bytes().as_slice(),
            "dropped.gtd",
        )));
    harness.run_steps(3);
    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the drop waits for the databases"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the drop that waited for the databases did not load"
    );
}

/// Pasted log text waits for the databases like any other load: a load that
/// started before them would trip the invariant adoption asserts.
#[test]
fn log_text_pasted_before_the_databases_land_loads_once_they_do() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);

    harness.input_mut().events.push(egui::Event::Paste(
        "2026-01-01 14:02:11 navsyncd: queue empty\n".to_owned(),
    ));
    harness.run_steps(3);
    assert!(
        harness.state().logs.is_empty(),
        "the paste loaded before the databases landed"
    );

    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| harness.state().logs.len() == 1),
        "the paste that waited for the databases did not load"
    );
}

/// A storage open that ends without reporting leaves the run storing nothing,
/// and says so instead of failing quietly.
#[test]
fn a_storage_open_that_never_reports_still_runs_the_loads_that_waited() {
    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);

    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(TestDroppedFile::bytes(
            minimal_gtd_bytes().as_slice(),
            "dropped.gtd",
        )));
    harness.run_steps(3);
    drop(databases);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the drop never loaded once the open gave up"
    );
    assert_eq!(harness.state().storage_open.databases_pending(), None);
}

/// The app started on `data_directory`, which the caller's own
/// [`DataDirectoryLock`] holds, with the files a command line named.
///
/// The app takes its own lock on that very directory, which is refused for as
/// long as the caller keeps its lock - the same answer a second GeoTrace gets
/// from the first.
fn app_waiting_for_the_data_directory<'a>(
    paths: &[PathBuf],
    data_directory: &Path,
) -> Harness<'a, App> {
    let instance_lock = SharedDataDirectoryLock::acquire(Some(data_directory));
    assert_eq!(
        instance_lock.ownership(),
        DataDirectoryOwnership::HeldByAnotherInstance,
        "the app is meant to start out waiting"
    );
    let paths = paths.to_vec();
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            transient_app_with_the_instance_lock(
                cc,
                &paths,
                instance_lock,
                PendingWrites::default(),
            )
        })
}

/// A second GeoTrace on a data directory the first is using opens no database
/// of its own: recovery here would run against archives the first is part-way
/// through rewriting. Its window is up and takes input all the same.
#[test]
fn a_data_directory_another_instance_holds_is_waited_for_and_nothing_is_opened() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());

    harness.step();

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    harness.get_by_label_contains("Its window is open");
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "the databases were opened under the instance holding the directory"
    );
    assert!(
        harness.query_by_label_contains(OPENING_DATABASES).is_none(),
        "nothing is opening, so nothing says it is"
    );

    harness.get_by_label_contains(ICON_GEAR).click();
    harness.step();

    assert!(
        harness.state().settings_open,
        "the window answered a click while it waited for the data directory"
    );
}

/// The lock is what says the directory is in use: a status file that is gone
/// or unreadable leaves the dialog with nothing to name, and it says only
/// what the lock proves.
#[rstest::rstest]
#[case::missing(None)]
#[case::corrupt(Some(&b"{not json"[..]))]
fn a_held_data_directory_without_a_readable_status_still_says_it_is_held(
    #[case] status_file: Option<&[u8]>,
) {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let status_path = directory.path().join(gt_instance_lock::STATUS_FILE_NAME);
    match status_file {
        None => std::fs::remove_file(&status_path).expect("remove the status file"),
        Some(bytes) => std::fs::write(&status_path, bytes).expect("write the status file"),
    }
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());

    harness.step();

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    harness.get_by_label_contains("What it is doing is unknown");
}

/// The wait ends by itself: the app takes the directory the instance holding
/// it let go of, and opens what it held back.
#[test]
fn the_wait_ends_when_the_instance_holding_the_data_directory_lets_go() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);

    drop(holder);

    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the app never opened its databases"
    );
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the dialog outlived the wait"
    );
    assert_eq!(
        InstanceStatus::read_from(directory.path()).map(|status| status.state),
        Some(InstanceState::Running),
        "the app marks the data directory as its own once it takes it"
    );
}

/// A lock file that stops opening says nothing about who has the directory,
/// and whatever stopped it may pass: the wait goes on through the retries
/// that lock file is given, and ends by taking the lock once it opens again.
#[test]
fn a_lock_file_that_briefly_cannot_be_opened_leaves_the_wait_running() {
    let parent = tempfile::tempdir().expect("temp dir");
    let data_directory = parent.path().join("data");
    let holder = DataDirectoryLock::acquire(Some(&data_directory));
    let mut harness = app_waiting_for_the_data_directory(&[], &data_directory);
    harness.step();
    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);

    drop(holder);
    std::fs::remove_dir_all(&data_directory).expect("remove the data directory");
    std::fs::write(&data_directory, b"not a directory").expect("put a file in its place");
    thread::sleep(DATA_DIRECTORY_RETRY_INTERVAL);
    harness.run_steps(3);

    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "the databases opened on a directory this instance never locked"
    );
    harness.get_by_label_contains(LOCK_FILE_UNUSABLE_TITLE);

    std::fs::remove_file(&data_directory).expect("clear the way");

    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the wait never ended once the lock file could be opened"
    );
}

/// Waiting is not a trap: the window closes on request, and an app on its way
/// out takes no directory and opens no database.
#[test]
fn a_window_closed_while_it_waits_for_the_data_directory_opens_nothing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();

    request_window_close(&mut harness);
    assert!(
        harness.step_until(
            |harness| root_viewport_commands(harness).contains(&egui::ViewportCommand::Close)
        ),
        "the window never closed"
    );
    drop(holder);
    thread::sleep(DATA_DIRECTORY_RETRY_INTERVAL);
    harness.run_steps(3);

    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "a closing app retried the directory and opened the databases on it"
    );
}

/// A file named on the command line of a second GeoTrace waits for the data
/// directory, and is loaded and stored once this instance owns it.
#[test]
fn a_file_named_while_the_data_directory_is_held_loads_once_it_frees() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let gtd_path = directory.path().join("queued.gtd");
    std::fs::write(&gtd_path, minimal_gtd_bytes()).expect("write the recording");
    let mut harness = app_waiting_for_the_data_directory(&[gtd_path], directory.path());
    harness.run_steps(3);

    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the load ran while another instance held the data directory"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    drop(holder);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the file that waited for the data directory did not load"
    );
}

/// A drop lands in the same queue, which the dialog being up does not stop.
#[test]
fn a_file_dropped_while_the_data_directory_is_held_loads_once_it_frees() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());

    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(TestDroppedFile::bytes(
            minimal_gtd_bytes().as_slice(),
            "dropped.gtd",
        )));
    harness.run_steps(3);
    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the drop loaded while another instance held the data directory"
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    drop(holder);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the drop that waited for the data directory did not load"
    );
}

/// Pasted log text waits for the data directory like any other load: paste is
/// its own surface, and has escaped this queue before.
#[test]
fn log_text_pasted_while_the_data_directory_is_held_loads_once_it_frees() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());

    harness.input_mut().events.push(egui::Event::Paste(
        "2026-01-01 14:02:11 navsyncd: queue empty\n".to_owned(),
    ));
    harness.run_steps(3);
    assert!(
        harness.state().logs.is_empty(),
        "the paste loaded while another instance held the data directory"
    );

    drop(holder);

    assert!(
        harness.step_until(|harness| harness.state().logs.len() == 1),
        "the paste that waited for the data directory did not load"
    );
}

/// Reports `holder` as shutting down with one archive compaction left, which
/// is what its status file then names.
fn report_the_holder_as_compacting_an_archive(holder: &mut DataDirectoryLock) {
    let pending_writes = PendingWrites::default();
    let _compaction = pending_writes
        .try_begin(
            "Compacting the TEC archive",
            WriteKind::ArchiveCompaction {
                archive: "ionospheric TEC",
            },
        )
        .expect("the registry is running");
    holder.mark_shutting_down(&pending_writes);
}

/// Takes write access as the user does: the button in the wait dialog, then
/// the confirmation it leads to.
fn take_over_write_access(harness: &mut Harness<'_, App>) {
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);
    harness.get_by_label("Take over").click();
    harness.run_steps(3);
}

/// The wait is not a dead end: the button opens a confirmation naming what
/// the instance holding the directory is doing, which it reads afresh for as
/// long as the confirmation is up.
#[test]
fn the_take_over_confirmation_names_what_the_other_instance_is_doing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let mut holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();

    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.get_by_label_contains(TAKE_OVER_CONFIRMATION_TITLE);
    harness.get_by_label_contains("window is open");
    harness.get_by_label_contains(TAKE_OVER_WARNING);
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the confirmation and the wait dialog are stacked on the same anchor"
    );

    report_the_holder_as_compacting_an_archive(&mut holder);

    assert!(
        harness.step_until(|harness| harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some()),
        "the confirmation names a state the other instance has left"
    );
    harness.get_by_label_contains("still finishing these writes");
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "the databases opened before the user answered the confirmation"
    );
}

/// Cancelling leaves everything as it was: the wait dialog is back and
/// nothing has been opened.
#[test]
fn cancelling_the_take_over_returns_to_the_wait() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.get_by_label("Cancel").click();
    harness.run_steps(3);

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    assert!(
        harness
            .query_by_label_contains(TAKE_OVER_CONFIRMATION_TITLE)
            .is_none(),
        "the confirmation outlived the cancel"
    );
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "a cancelled take-over opened the databases"
    );
}

/// Escape answers the confirmation the way Cancel does, as it does for every
/// other destructive confirmation.
#[test]
fn escape_cancels_the_take_over_confirmation() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        Some(DatabasesPending::WaitingForTheDataDirectory),
        "escape opened the databases"
    );
}

/// Taking over opens the databases with the other instance still holding the
/// lock, and the loads that waited run against them.
#[test]
fn taking_over_opens_the_databases_and_runs_the_loads_that_waited() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let gtd_path = directory.path().join("queued.gtd");
    std::fs::write(&gtd_path, minimal_gtd_bytes()).expect("write the recording");
    let mut harness = app_waiting_for_the_data_directory(&[gtd_path], directory.path());
    harness.run_steps(3);
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    take_over_write_access(&mut harness);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the file that waited for the data directory never loaded"
    );
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        None,
        "the take-over left the databases unopened"
    );
    assert_eq!(
        harness.state().instance_lock.ownership(),
        DataDirectoryOwnership::HeldByAnotherInstance,
        "the take-over took the lock instead of proceeding without it"
    );
    assert_eq!(
        harness.state().instance_taken_over_from,
        Some(TakenOverInstance {
            process_id: Some(process::id())
        }),
        "the take-over left no record of the instance it took write access from"
    );
    assert_eq!(
        harness.state().environment_deletes_blocked_by(),
        None,
        "the delete controls stayed grayed with the reason the wait gave"
    );
}

/// Taking over reads the archives before it opens any of them: the other
/// instance may be part-way through a delete right now, and what that leaves
/// is the user's to answer, not the open's to recover.
#[test]
fn taking_over_reads_the_archives_before_it_opens_them() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness
        .get_by_label_contains(TAKE_OVER_BUTTON_LABEL)
        .click();
    harness.run_steps(3);

    harness.get_by_label("Take over").click();

    assert!(
        harness.step_until(|harness| matches!(
            harness.state().storage_open,
            StorageOpen::InspectingArchives { .. }
        )),
        "the take-over opened the archives without reading them for an interrupted delete"
    );
}

/// Taking the lock late is a promotion and nothing more: the instance that
/// took over becomes the marked owner without reopening anything.
#[test]
fn the_lock_freed_after_a_take_over_makes_this_instance_the_marked_owner() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    take_over_write_access(&mut harness);
    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the take-over never opened the databases"
    );

    drop(holder);

    assert!(
        harness.step_until(|harness| harness.state().instance_lock.ownership()
            == DataDirectoryOwnership::MarkedByThisInstance),
        "this instance never took the data directory the other one let go of"
    );
    assert_eq!(
        InstanceStatus::read_from(directory.path()).map(|status| status.process_id),
        Some(process::id()),
        "the status file describes another instance than the one holding the directory"
    );
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        None,
        "the promotion opened the databases a second time"
    );
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the promotion put the app back in the wait"
    );
    assert!(
        harness.state().background_mark_retry.is_none(),
        "the retry goes on after this instance became the marked owner"
    );
}

/// Starts the session read-only as the user does: the wait dialog's button,
/// which leads to no confirmation.
fn start_the_session_read_only(harness: &mut Harness<'_, App>) {
    harness
        .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
        .click();
    harness.run_steps(3);
}

/// The wait offers a second way out: reading the recordings and archives
/// beside the instance that owns the data directory, which leaves that
/// instance's mark where it is.
#[test]
fn starting_read_only_leaves_the_wait_and_opens_the_databases_without_the_lock() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    harness.get_by_label_contains(DATA_DIRECTORY_HELD_TITLE);

    start_the_session_read_only(&mut harness);

    assert_eq!(
        harness.state().pending_writes.write_access(),
        WriteAccess::ReadOnly,
        "the session went on writing after the user chose to read"
    );
    assert_eq!(
        harness.state().storage_open.databases_pending(),
        None,
        "the read-only choice left the databases unopened"
    );
    assert_eq!(
        harness.state().instance_lock.ownership(),
        DataDirectoryOwnership::NoDataDirectory,
        "the read-only session kept a claim on the data directory"
    );
    assert!(
        harness
            .query_by_label_contains(DATA_DIRECTORY_HELD_TITLE)
            .is_none(),
        "the wait dialog outlived the read-only choice"
    );
}

/// No promotion, ever: the instance that owns the data directory letting go
/// leaves the read-only session as it is, and the directory free for whoever
/// starts next.
#[test]
fn a_read_only_session_does_not_become_the_owner_when_the_other_instance_exits() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    start_the_session_read_only(&mut harness);

    drop(holder);
    thread::sleep(DATA_DIRECTORY_RETRY_INTERVAL);
    harness.run_steps(3);

    assert!(
        DataDirectoryLock::acquire(Some(directory.path())).marks_the_data_directory(),
        "the read-only session holds the lock the next instance needs"
    );
    assert_eq!(
        harness.state().instance_lock.ownership(),
        DataDirectoryOwnership::NoDataDirectory,
        "the read-only session took the data directory the other instance let go of"
    );
    assert_eq!(
        harness.state().pending_writes.write_access(),
        WriteAccess::ReadOnly,
        "the read-only session started writing once the directory was free"
    );
}

/// The marker states what the session is, as the debug-build warning does,
/// and a session that owns the data directory shows none.
#[test]
fn only_a_read_only_session_shows_the_read_only_marker() {
    let directory = tempfile::tempdir().expect("temp dir");
    let holder = DataDirectoryLock::acquire(Some(directory.path()));
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());
    harness.step();
    assert!(
        harness
            .query_by_label_contains(READ_ONLY_MARKER_LABEL)
            .is_none(),
        "a session that has yet to choose is marked as read-only"
    );

    start_the_session_read_only(&mut harness);

    let marker = harness.get_by_label_contains(READ_ONLY_MARKER_LABEL).rect();
    harness.hover_at_and_settle(marker.center(), 3);
    harness.get_by_label_contains(&format!(
        "Another GeoTrace (process {}) owns the data directory",
        process::id()
    ));
    drop(holder);
}

/// The file a command line named waits through the choice: it loads once the
/// databases are open, and the read-only session stores none of it.
#[test]
fn a_file_queued_while_waiting_loads_read_only_and_is_not_stored() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let store = gt_store::Store::open_in(directory.path());
    let gtd_path = directory.path().join("queued.gtd");
    std::fs::write(&gtd_path, minimal_gtd_bytes()).expect("write the recording");
    let mut harness = app_waiting_for_the_data_directory(&[gtd_path], directory.path());
    harness.run_steps(3);
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);

    harness
        .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
        .click();
    // The load runs against a real recording history from here on: a test
    // run opens no database of its own, and this is the one frame the choice
    // takes before that lands.
    harness.step();
    let databases = harness.state_mut().storage_open.take_over_for_test();
    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1),
        "the file that waited for the data directory never loaded"
    );
    let recordings =
        Recordings::open_or_create(&store.recordings_path()).expect("open the recording history");
    assert_eq!(
        recordings.list_recordings().expect("list").len(),
        0,
        "the read-only session stored the recording that waited for it"
    );
}

/// Paste is its own load surface and has escaped this queue before, so the
/// read-only exit from the wait carries it too.
#[test]
fn log_text_pasted_while_waiting_loads_in_the_read_only_session_it_starts() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _holder = DataDirectoryLock::acquire(Some(directory.path()));
    let store = gt_store::Store::open_in(directory.path());
    let mut harness = app_waiting_for_the_data_directory(&[], directory.path());

    harness.input_mut().events.push(egui::Event::Paste(
        "2026-01-01 14:02:11 navsyncd: queue empty\n".to_owned(),
    ));
    harness.run_steps(3);
    assert!(
        harness.state().logs.is_empty(),
        "the paste loaded while another instance held the data directory"
    );

    harness
        .get_by_label_contains(START_READ_ONLY_BUTTON_LABEL)
        .click();
    harness.step();
    let databases = harness.state_mut().storage_open.take_over_for_test();
    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| harness.state().logs.len() == 1),
        "the paste that waited for the data directory never loaded"
    );
}

/// The interference archive's day index, which is where a delete records
/// that it is part-way through.
const INTERFERENCE_DAYS: GroupPath<'static> = GroupPath(schema::DAYS_GROUP);

/// A data directory whose interference archive holds two days with a delete
/// marked part-way through it, as an instance killed mid-delete leaves it.
fn data_directory_with_an_interrupted_interference_delete() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let path = store.interference_path();
    {
        let archive = store.open_interference().expect("the interference archive");
        for offset in 0..2 {
            archive
                .insert_day(
                    chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap_or_default()
                        + chrono::TimeDelta::days(offset),
                    "host",
                    chrono::Utc::now(),
                    &[],
                )
                .expect("archive a day");
        }
    }
    drop(store);
    day_archive::mark_delete_in_flight(&path, INTERFERENCE_DAYS).expect("mark the delete");
    dir
}

/// The app part-way through the open a take-over runs: the archives under
/// `root` have been read, and the prompts for what that found are up.
fn app_asking_about_the_archives_under<'a>(root: &Path) -> Harness<'a, App> {
    app_asking_about(archive_recovery::inspect_archives_under(root.to_owned()))
}

/// The same, for findings this process cannot produce on its own: libhdf5
/// hands one process the same open file twice rather than refusing it.
fn app_asking_about<'a>(inspected: InspectedArchives) -> Harness<'a, App> {
    let (mut harness, _databases) = app_with_the_databases_still_opening(&[]);
    harness
        .state_mut()
        .storage_open
        .inspect_archives_for_test(inspected);
    harness.run_steps(3);
    harness
}

fn wait_for_the_archives_to_open(harness: &mut Harness<'_, App>) {
    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the open the answers started never finished"
    );
}

/// The recovery an instance that is gone left behind is the user's to make
/// after a take-over: the prompt names the archive and what recovering costs,
/// and recovering opens it with those days discarded.
#[test]
fn recovering_after_a_take_over_opens_the_archive_with_its_days_discarded() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).interference_path();
    let mut harness = app_asking_about_the_archives_under(dir.path());

    harness.get_by_label_contains("Recover the aircraft interference archive?");
    harness.get_by_label_contains("discards the 2 archived days it holds");
    harness.get_by_label(RECOVER_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    let archive = harness
        .state()
        .jamming
        .archive()
        .expect("the recovered archive is open");
    assert_eq!(archived_days(&archive), []);
    assert_eq!(
        JamStore::interrupted_delete_at(&path).expect("read the archive"),
        None,
        "the archive was opened with the interrupted delete still in it"
    );
    assert_eq!(
        harness
            .state()
            .unavailable_archives
            .of(EnvironmentArchive::AircraftInterference),
        None
    );
}

/// Leaving it alone costs the archive for the session and nothing on disk:
/// the file is byte-for-byte what it was, and the archives nobody was asked
/// about open beside it.
#[test]
fn leaving_an_interrupted_delete_unrecovered_writes_nothing_to_the_archive() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).interference_path();
    let untouched = std::fs::read(&path).expect("the archive as the delete left it");
    let mut harness = app_asking_about_the_archives_under(dir.path());

    harness.get_by_label(LEAVE_UNRECOVERED_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        std::fs::read(&path).expect("read the archive"),
        untouched,
        "the archive the user left alone was written to"
    );
    assert_eq!(
        JamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "the days are gone, or the delete no longer reads as interrupted"
    );
    assert!(
        !harness.state().jamming.archive_available(),
        "the archive was opened after the user left it unrecovered"
    );
    assert_eq!(
        harness
            .state()
            .unavailable_archives
            .of(EnvironmentArchive::AircraftInterference),
        Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
    );
    assert!(
        harness.state().tec_maps.archive_available(),
        "one archive left closed closed the others too"
    );
}

/// Escape answers the way the button that discards nothing does, as it does
/// for every other destructive confirmation.
#[test]
fn escape_leaves_the_interrupted_delete_unrecovered() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).interference_path();
    let mut harness = app_asking_about_the_archives_under(dir.path());

    harness.key_press(egui::Key::Escape);
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        JamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "escape recovered the interrupted delete"
    );
    assert_eq!(
        harness
            .state()
            .unavailable_archives
            .of(EnvironmentArchive::AircraftInterference),
        Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
    );
}

/// An archive the other GeoTrace still has open cannot be recovered here, so
/// no recovery is offered: the user is told what it costs and the open goes
/// on without it.
#[test]
fn an_archive_the_other_instance_holds_is_reported_as_in_use() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut harness = app_asking_about(InspectedArchives::of_findings_under(
        dir.path().to_owned(),
        vec![(
            EnvironmentArchive::AircraftInterference,
            InterruptedDeleteFinding::HeldByTheOtherInstance,
        )],
    ));

    harness.get_by_label_contains("The aircraft interference archive is in use");
    assert!(
        harness.query_by_label(RECOVER_BUTTON_LABEL).is_none(),
        "a recovery was offered for an archive this instance cannot open"
    );
    harness.get_by_label(ARCHIVE_IN_USE_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        harness
            .state()
            .unavailable_archives
            .of(EnvironmentArchive::AircraftInterference),
        Some(ArchiveUnavailable::HeldByTheOtherInstance)
    );
    assert!(
        !harness.state().jamming.archive_available(),
        "the archive the other instance holds was opened here"
    );
}

/// Escape answers the in-use notice the way its one button does, so a stray
/// keypress cannot open an archive the other GeoTrace holds.
#[test]
fn escape_leaves_the_archive_the_other_instance_holds_alone() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut harness = app_asking_about(InspectedArchives::of_findings_under(
        dir.path().to_owned(),
        vec![(
            EnvironmentArchive::AircraftInterference,
            InterruptedDeleteFinding::HeldByTheOtherInstance,
        )],
    ));

    harness.key_press(egui::Key::Escape);
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        harness
            .state()
            .unavailable_archives
            .of(EnvironmentArchive::AircraftInterference),
        Some(ArchiveUnavailable::HeldByTheOtherInstance)
    );
    assert!(
        !harness.state().jamming.archive_available(),
        "escape opened the archive the other instance holds"
    );
}

/// A delete interrupted after the archives were read is not recovered behind
/// the user's back: the open declines what nobody was asked about, and the
/// archive keeps its days.
#[test]
fn an_interrupted_delete_nobody_was_asked_about_is_declined() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).interference_path();
    let mut harness = app_asking_about(InspectedArchives::of_findings_under(
        dir.path().to_owned(),
        Vec::new(),
    ));

    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        JamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "the open recovered a delete nobody was asked about"
    );
    assert_eq!(
        harness
            .state()
            .unavailable_archives
            .of(EnvironmentArchive::AircraftInterference),
        Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
    );
    assert!(harness.state().tec_maps.archive_available());
}

/// Never merely empty, per DESIGN.md: the controls that need an archive left
/// unrecovered are grayed and say why it is not there.
#[test]
fn an_archive_left_unrecovered_says_why_on_the_controls_that_need_it() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let mut harness = app_asking_about_the_archives_under(dir.path());
    harness.get_by_label(LEAVE_UNRECOVERED_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::AircraftInterference;
    harness.run_steps(3);
    harness.hover_and_settle(By::new().label_contains(DOWNLOAD_HISTORY_LABEL), 3);
    harness.get_by_label_contains(
        "The interference archive is unavailable this session: an interrupted delete in it was \
         left unrecovered",
    );

    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);
    let interference_row = harness
        .topmost_matching(By::new().label_contains(DELETE_ALL_LABEL))
        .rect()
        .center();
    harness.hover_at_and_settle(interference_row, 3);
    harness.get_by_label_contains(
        &DeleteBlocker::ArchiveUnavailable(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
            .hover_text(),
    );
}

/// The prompts are not a trap either: the window closes on request, and an
/// app on its way out opens no archive.
#[test]
fn a_window_closed_while_an_interrupted_delete_is_asked_about_opens_nothing() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).interference_path();
    let mut harness = app_asking_about_the_archives_under(dir.path());
    harness.get_by_label_contains("Recover the aircraft interference archive?");
    assert!(
        harness.state().pending_writes.is_idle(),
        "a write is registered while the open waits on a person, which a close would wait for"
    );

    request_window_close(&mut harness);

    assert!(
        harness.step_until(closed_the_window),
        "the window never closed"
    );
    assert_eq!(
        JamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "a closing app recovered the interrupted delete"
    );
    assert!(!harness.state().jamming.archive_available());
}

/// Never hidden, per DESIGN.md: the controls that need an archive are grayed
/// while the archives open, and say what they are waiting for.
#[test]
fn the_environment_controls_are_grayed_while_the_archives_open() {
    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);

    for delete in harness.query_all_by_label_contains(DELETE_ALL_LABEL) {
        assert!(delete.accesskit_node().is_disabled());
    }
    let prune = harness.get_by_label_contains(PRUNE_BUTTON_LABEL);
    assert!(prune.accesskit_node().is_disabled());

    harness.hover_and_settle(By::new().label_contains(PRUNE_BUTTON_LABEL), 3);
    harness.get_by_label_contains(&DeleteBlocker::ArchivesOpening.hover_text());

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    // A day to delete, or the control stays grayed for having nothing to act on.
    store
        .open_interference()
        .expect("open the archive")
        .insert_day(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 20).expect("date"),
            "host",
            chrono::Utc::now(),
            &[],
        )
        .expect("insert");
    land_the_databases(&mut harness, &databases, &store);
    harness.run_steps(3);

    assert!(
        !harness
            .get_by_label_contains(PRUNE_BUTTON_LABEL)
            .accesskit_node()
            .is_disabled(),
        "the archives landed, so the delete is live again"
    );
    assert!(
        harness
            .query_by_label_contains(&DeleteBlocker::ArchivesOpening.hover_text())
            .is_none(),
        "the opening hover text outlived the open"
    );
}

/// Never hidden, per DESIGN.md: in a read-only session every control that
/// would write to an archive is grayed and says the session changes none.
#[test]
fn the_environment_controls_are_grayed_in_a_read_only_session() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    // A day to delete, or the controls stay grayed for having nothing to act
    // on, whatever the session may write.
    store
        .open_interference()
        .expect("open the archive")
        .insert_day(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 20).expect("date"),
            "host",
            chrono::Utc::now(),
            &[],
        )
        .expect("insert");
    let (mut harness, databases) =
        app_with_the_databases_still_opening_for(&[], WriteAccess::ReadOnly);
    land_the_databases(&mut harness, &databases, &store);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);

    assert_eq!(
        harness.state().environment_deletes_blocked_by(),
        Some(DeleteBlocker::ReadOnlySession)
    );
    for delete in harness.query_all_by_label_contains(DELETE_ALL_LABEL) {
        assert!(delete.accesskit_node().is_disabled());
    }
    assert!(
        harness
            .get_by_label_contains(PRUNE_BUTTON_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_label_contains(ENVIRONMENT_AUTO_PRUNE_LABEL)
            .accesskit_node()
            .is_disabled(),
        "the setting takes no input: a read-only session auto-prunes nothing"
    );
    harness.hover_and_settle(By::new().label_contains(PRUNE_BUTTON_LABEL), 3);
    harness.get_by_label_contains(&DeleteBlocker::ReadOnlySession.hover_text());

    harness.state_mut().settings_page = SettingsPage::AircraftInterference;
    harness.run_steps(3);

    assert!(
        harness
            .get_by_label_contains(DOWNLOAD_HISTORY_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    harness.hover_and_settle(By::new().label_contains(DOWNLOAD_HISTORY_LABEL), 3);
    harness.get_by_label_contains("This session is read-only: nothing is downloaded into the");
}

/// The recording storage controls are grayed the same way: a read-only
/// session stores no recording and prunes none.
#[test]
fn the_recording_storage_controls_are_grayed_in_a_read_only_session() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) =
        app_with_the_databases_still_opening_for(&[], WriteAccess::ReadOnly);
    land_the_databases(&mut harness, &databases, &store);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);

    assert!(
        harness
            .get_by_label_contains(AUTO_STORE_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    harness.hover_and_settle(By::new().label_contains(AUTO_STORE_LABEL), 3);
    harness.get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);

    for auto_prune in ["Auto-prune when over", "Confirm before pruning"] {
        assert!(
            harness
                .get_by_label_contains(auto_prune)
                .accesskit_node()
                .is_disabled(),
            "{auto_prune} is live in a session that stores no recording"
        );
    }
}

/// The download control on a source page is grayed the same way: there is
/// nowhere to download to until the archive is open.
#[test]
fn the_download_control_is_grayed_while_the_archive_opens() {
    let (mut harness, _databases) = app_with_the_databases_still_opening(&[]);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::AircraftInterference;
    harness.run_steps(3);

    let download = harness.get_by_label_contains(DOWNLOAD_HISTORY_LABEL);
    assert!(download.accesskit_node().is_disabled());

    harness.hover_and_settle(By::new().label_contains(DOWNLOAD_HISTORY_LABEL), 3);
    harness.get_by_label_contains("The interference archive is still opening");
}

/// The startup auto-prune acts on the archives, so it runs when they land -
/// at construction there is nothing to delete from.
#[test]
fn the_environment_auto_prune_runs_when_the_archives_land() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let archive = store.open_interference().expect("the interference archive");
    archive
        .insert_day(old, "host", chrono::Utc::now(), &[])
        .expect("archive a day past any offered age");

    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);
    enable_environment_auto_prune(&mut harness, 12);

    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.step_until(|harness| !harness.state().environment_prune_running()),
        "the delete did not finish"
    );
    assert_eq!(archived_days(&archive), []);
}

/// A storage that lands after the close began installs nothing: the worker
/// shutdown already ended is not replaced by a live one, which would keep the
/// database open past the close.
#[test]
fn a_storage_landing_after_the_close_began_installs_no_worker() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);
    harness.step();

    harness.state_mut().begin_shutdown();
    land_the_databases(&mut harness, &databases, &store);

    assert!(
        harness.state().history.path().is_none(),
        "the close left the app with no worker"
    );
    assert!(
        harness.state().loader.db_path.is_none(),
        "nothing is stored while the app closes"
    );
    assert!(
        harness.step_until(|harness| harness.state().shutdown.close_allowed()),
        "the app did not close"
    );
}

/// "Try again" on a database that still will not open puts the prompt back
/// rather than leaving the user with no history and no explanation. Uses an
/// unreadable file, since holding a real lock needs a second process.
#[test]
fn a_failed_retry_restores_the_prompt() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("recordings.h5");
    std::fs::write(&path, b"not a database").expect("write");

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().history_failure =
        Some(crate::app::storage::HistoryFailure::Busy(path.clone()));

    let ctx = harness.ctx.clone();
    harness.state_mut().reopen_history_database(&path, &ctx);

    assert_eq!(
        harness.state().history_failure,
        Some(crate::app::storage::HistoryFailure::Unreadable(path)),
        "the retry reclassifies instead of clearing the prompt"
    );
    assert!(harness.state().history.path().is_none());
}

/// A shutdown that has begun refuses all three recovery paths: each writes to
/// the recordings database. `recreate_history_database` renames the file before
/// it reopens it, and a quit in between leaves no database.
#[rstest::rstest]
#[case::reopen(|app: &mut App, path: &Path, ctx: &egui::Context| {
    app.reopen_history_database(path, ctx);
})]
#[case::recover(|app: &mut App, path: &Path, ctx: &egui::Context| {
    app.recover_history_database(path, ctx);
})]
#[case::recreate(|app: &mut App, path: &Path, ctx: &egui::Context| {
    app.recreate_history_database(path, true, ctx);
})]
fn a_history_database_recovery_is_refused_once_shutdown_has_begun(
    #[case] recover: fn(&mut App, &Path, &egui::Context),
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let path = store.recordings_path();
    drop(
        store
            .open_recordings()
            .expect("create the recordings database"),
    );
    let files_in_the_data_directory = || {
        let mut names: Vec<std::ffi::OsString> = std::fs::read_dir(dir.path())
            .expect("read the data directory")
            .filter_map(|entry| Some(entry.ok()?.file_name()))
            .collect();
        names.sort();
        names
    };
    let before = files_in_the_data_directory();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state().pending_writes.begin_shutdown();
    let ctx = harness.ctx.clone();

    recover(harness.state_mut(), &path, &ctx);

    assert!(
        harness.state().history.path().is_none(),
        "the refused recovery opened the recordings database"
    );
    assert_eq!(
        files_in_the_data_directory(),
        before,
        "the refused recovery renamed, removed or created a file"
    );
}

/// An app that took write access from another instance, with `failure` set as
/// the recordings database's open would have set it.
fn app_after_a_take_over_with<'a>(
    failure: crate::app::storage::HistoryFailure,
) -> Harness<'a, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().instance_taken_over_from = Some(TakenOverInstance {
        process_id: Some(4210),
    });
    harness.state_mut().history_failure = Some(failure);
    harness.run_steps(2);
    harness
}

/// The busy prompt names the GeoTrace the user took write access from: they
/// chose to keep it running.
#[test]
fn the_busy_prompt_after_a_take_over_names_the_instance_that_still_has_the_database() {
    let harness = app_after_a_take_over_with(crate::app::storage::HistoryFailure::Busy(
        PathBuf::from("recordings.h5"),
    ));

    harness.get_by_label_contains(
        "Another GeoTrace (process 4210) still has the recording history database open",
    );
    harness.get_by_label_contains("not stored until it exits");
    assert!(
        harness
            .query_by_label_contains("Close it and try again")
            .is_none(),
        "the prompt asks for the GeoTrace the user chose to keep running to be closed"
    );
    harness.get_by_label("Try again");
}

/// The lock clear is grayed after a take-over: clearing it while the other
/// GeoTrace writes can corrupt the database.
#[test]
fn the_locked_prompt_after_a_take_over_grays_the_lock_clear() {
    let mut harness = app_after_a_take_over_with(crate::app::storage::HistoryFailure::Locked(
        PathBuf::from("recordings.h5"),
    ));

    let clear = harness.get_by_label_contains(CLEAR_LOCK_BUTTON_LABEL);
    assert!(
        clear.accesskit_node().is_disabled(),
        "the clear is live while another GeoTrace has the database open"
    );
    let center = clear.rect().center();
    harness.hover_at_and_settle(center, 5);
    harness.get_by_label_contains(
        "Another GeoTrace (process 4210) still has the recording history database open",
    );
}

/// A database held by another instance is a wait, not a repair, so this
/// prompt offers neither the lock clear nor the recreate.
#[test]
fn snapshot_history_busy_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(crate::app::storage::HistoryFailure::Busy(
        PathBuf::from("geotrace.h5"),
    ));
    harness.run();
    harness.snapshot("history_busy_dialog");
}

#[test]
fn snapshot_history_resegment_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_resegment = Some(super::ResegmentPrompt {
        db_ref: gt_store::DatabaseRef {
            identity: "auto:ride.gtd".to_owned(),
            group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
        },
        filename: "ride.gtd".to_owned(),
        bytes: std::sync::Arc::from(Vec::<u8>::new()),
        stored: gt_store::StoredSegmentation {
            track_split_gap_us: 60_000_000,
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 4.0,
        },
        hidden_positions: Vec::new(),
        marker_settings_changed: false,
    });
    harness.run();
    harness.snapshot("history_resegment_dialog");
}

#[test]
fn snapshot_load_warnings_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state().shared.borrow_mut().warnings_popup = Some((
        "ride_2025-05-23.gtd".to_owned(),
        vec![
            LoadWarning {
                count: 3,
                issue: "satellite(s) with PRN 0".to_owned(),
                description: "PRN 0 is reserved and undefined in NMEA".to_owned(),
            },
            LoadWarning {
                count: 2,
                issue: "satellite(s) with elevation > 90°".to_owned(),
                description: "above the zenith; valid NMEA elevation range is [0°, 90°]"
                    .to_owned(),
            },
            LoadWarning {
                count: 5,
                issue: "satellite(s) with SNR ≈ 99 dB-Hz".to_owned(),
                description: "common sentinel value for unavailable signal strength; omit the SNR field when no measurement is available".to_owned(),
            },
        ],
    ));
    harness.run();
    harness.snapshot("load_warnings_dialog");
}

#[test]
fn snapshot_snap_consent_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().snap_consent_prompt = true;
    harness.run();
    harness.snapshot("snap_consent_dialog");
}

/// The service link only shows for the default FOSSGIS host - its terms do
/// not apply to a self-hosted server.
#[test]
fn consent_service_link_gates_on_the_default_host() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().snap_consent_prompt = true;
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Read more about the routing service")
            .is_some(),
        "the default host should offer the service description"
    );

    harness.inner.state_mut().snap_settings.server_url = "https://valhalla.example.com".to_owned();
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Read more about the routing service")
            .is_none(),
        "a self-hosted server must not link to the FOSSGIS terms"
    );
}

/// The consent dialog once the mode choice was already made: a single
/// plain Agree, no mode paragraph.
#[test]
fn snapshot_snap_consent_dialog_mode_chosen() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().snap_settings.auto_snap = Some(false);
    harness.inner.state_mut().snap_consent_prompt = true;
    harness.run();
    harness.snapshot("snap_consent_dialog_mode_chosen");
}

/// The one-time auto prompt for uploads acknowledged before auto mode
/// existed.
#[test]
fn snapshot_snap_auto_prompt() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(build_app);
    harness.inner.step();
    harness
        .inner
        .state_mut()
        .snap_settings
        .acknowledge_consent();
    let state = harness.inner.state_mut();
    let mut shared = state.shared.borrow_mut();
    let points = gt_test_utils::nav_test_data();
    let file = gt_track_builder::build_loaded_file(
        "ride.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(std::path::PathBuf::from("ride.gtd")),
        gt_track_builder::FileMeta::default(),
        vec![],
    );
    shared
        .loaded_files
        .push(file, gt_loaded_files::FileHistory::None);
    shared.sync_tree_from_loaded_files();
    drop(shared);
    harness.run();
    harness.snapshot_loose("snap_auto_prompt");
}

#[test]
fn snap_consent_agree_persists_the_server_host() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    assert!(!harness.state().snap_settings.consent_granted());
    harness.state_mut().snap_consent_prompt = true;
    harness.step();

    // A fresh app renders the map-layer popup open, and the first synthetic
    // click is spent dismissing it before the dialog's buttons receive
    // anything - so click once to settle the popup, then click for real.
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - manual only")
        .click();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - manual only")
        .click();
    harness.run_steps(3);

    assert!(!harness.state().snap_consent_prompt, "dialog must close");
    assert!(
        harness.state().snap_settings.consent_granted(),
        "agreeing must record consent for the configured server's host"
    );
    assert_eq!(
        harness.state().snap_settings.auto_snap,
        Some(false),
        "the agree variant must persist the mode choice"
    );
}

#[test]
fn snap_consent_escape_declines_without_persisting() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().snap_consent_prompt = true;
    harness.step();

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(
        !harness.state().snap_consent_prompt,
        "Escape must close the dialog"
    );
    assert!(
        !harness.state().snap_settings.consent_granted(),
        "declining must not record consent - the next trigger re-prompts"
    );
    assert_eq!(
        harness.state().snap_settings.auto_snap,
        Some(false),
        "declined consent must never leave auto uploads armed"
    );
}

/// Uploads acknowledged before auto mode existed: the one-time prompt
/// appears once a snappable track is loaded, and the choice persists.
#[test]
fn auto_prompt_appears_once_after_earlier_consent() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    harness.step();
    assert_eq!(
        harness.state().snap_settings.auto_snap,
        None,
        "no prompt without a snappable track"
    );

    push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    harness.run_steps(2);

    // First synthetic click settles the startup map-layer popup (see
    // snap_consent_agree_persists_the_server_host). The second only fires
    // while the prompt is still open: unlike the sibling consent dialogs,
    // this prompt sometimes receives the first click already, and a second
    // Enter-equivalent click after it closed would go to the map.
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Snap automatically")
        .click();
    harness.run_steps(3);
    if harness.state().snap_settings.auto_snap.is_none() {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Snap automatically")
            .click();
        harness.run_steps(3);
    }

    assert_eq!(harness.state().snap_settings.auto_snap, Some(true));
    assert!(harness.state().snap_settings.auto_snap_active());
}

/// Auto mode armed without acknowledged uploads (the settings checkbox):
/// the consent dialog opens on the first load with a snappable track, and
/// nothing is enqueued until the user responds.
#[test]
fn auto_without_consent_prompts_before_anything_is_sent() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().snap_settings.auto_snap = Some(true);
    harness.step();
    assert!(
        !harness.state().snap_consent_prompt,
        "no prompt without a snappable track"
    );

    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    harness.run_steps(2);

    assert!(harness.state().snap_consent_prompt);
    assert!(
        harness.state().snap.activity_for(track).is_none(),
        "nothing may be enqueued before consent"
    );
}

/// Offline pauses auto mode: the sweep enqueues nothing even with auto
/// active and an unsnapped track loaded.
#[test]
fn auto_sweep_is_paused_offline() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    harness.state_mut().snap_settings.auto_snap = Some(true);
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    harness.run_steps(3);

    assert!(harness.state().snap_settings.auto_snap_active());
    assert!(
        harness.state().snap.activity_for(track).is_none(),
        "offline must pause the auto queue"
    );
}

/// The harness reaches `App::new_with_config`, the same constructor `main`
/// uses. A test run opens neither the user's recordings database nor their
/// interference archive.
#[test]
fn the_test_harness_opens_no_user_databases() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    assert!(
        harness.state().history.path().is_none(),
        "no recordings database"
    );
    assert!(
        !harness.state().jamming.archive_available(),
        "no interference archive"
    );
    assert!(
        harness.state().loader.db_path.is_none(),
        "nothing for the loader to store into"
    );
}

/// Installs an interference scheduler whose archive holds `days`, and hands
/// the archive back so a test can read what a delete left in it.
fn install_interference_archive(
    harness: &mut Harness<'_, App>,
    days: &[chrono::NaiveDate],
) -> (tempfile::TempDir, Arc<gt_store::JamStore>) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_interference()
        .expect("archive");
    for day in days {
        store
            .insert_day(*day, "host", chrono::Utc::now(), &[])
            .expect("insert");
    }
    install_interference_scheduler(harness, &store);
    (dir, store)
}

/// Point the app at `store` for interference, with nothing to fetch from.
fn install_interference_scheduler(harness: &mut Harness<'_, App>, store: &Arc<gt_store::JamStore>) {
    let ctx = harness.ctx.clone();
    harness.state_mut().jamming = crate::app::jamming::JammingScheduler::new(
        ctx,
        Some(Arc::clone(store)),
        gt_jam::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
}

fn archived_days(store: &gt_store::JamStore) -> Vec<chrono::NaiveDate> {
    store
        .days()
        .expect("read the archive index")
        .into_iter()
        .map(|stored| stored.day)
        .collect()
}

fn app_with_interference_days<'a>(
    days: &[chrono::NaiveDate],
) -> (Harness<'a, App>, tempfile::TempDir, Arc<gt_store::JamStore>) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let (dir, store) = install_interference_archive(&mut harness, days);
    (harness, dir, store)
}

fn enable_environment_auto_prune(harness: &mut Harness<'_, App>, max_age_months: u32) {
    let settings = &mut harness.state_mut().environment_storage_settings;
    settings.auto_prune_enabled = true;
    settings.auto_prune_max_age_months = max_age_months;
}

/// A day older than any age the control offers stays archived while
/// auto-pruning is off, which is how a fresh install runs.
#[test]
fn environment_auto_pruning_is_off_until_it_is_ticked() {
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, store) = app_with_interference_days(&[old]);

    assert!(harness.state().environment_auto_prune_request().is_none());

    harness.state_mut().auto_prune_environment_days();
    harness.run_steps(3);
    assert_eq!(archived_days(&store), [old]);
}

/// With nothing loaded the archives lose every day past the configured age
/// and keep the ones inside it.
#[test]
fn environment_auto_pruning_deletes_the_days_past_the_configured_age() {
    let today = chrono::Utc::now().date_naive();
    let recent = today - chrono::TimeDelta::days(2);
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, store) = app_with_interference_days(&[old, recent]);
    enable_environment_auto_prune(&mut harness, 12);

    harness.state_mut().auto_prune_environment_days();
    assert!(
        harness.step_until(|harness| !harness.state().environment_prune_running()),
        "the delete did not finish"
    );

    assert_eq!(archived_days(&store), [recent]);
}

/// A day a loaded recording needs survives however old it is: the schedulers
/// would fetch it again as soon as it went.
#[test]
fn environment_auto_pruning_keeps_the_days_the_loaded_recording_needs() {
    let recorded = base_time().date_naive();
    let before_the_recording = recorded - chrono::TimeDelta::days(1);
    let (mut harness, _dir, store) = app_with_interference_days(&[before_the_recording, recorded]);
    enable_environment_auto_prune(&mut harness, 1);

    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(minimal_gtd_bytes().as_slice(), "ride.gtd"),
    );
    assert!(
        harness.step_until(|harness| !harness.state().environment_prune_running()),
        "the delete did not finish"
    );

    assert_eq!(
        archived_days(&store),
        [recorded],
        "the recording's own day is older than the configured age and stays"
    );
}

/// No delete starts once shutdown has begun: the archives keep their days and
/// the process has no rewrite to wait for.
#[test]
fn environment_pruning_does_not_start_during_shutdown() {
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, store) = app_with_interference_days(&[old]);
    enable_environment_auto_prune(&mut harness, 12);
    harness.state().pending_writes.begin_shutdown();

    harness.state_mut().auto_prune_environment_days();
    harness.run_steps(3);

    assert!(!harness.state().environment_prune_running());
    assert_eq!(archived_days(&store), [old]);
}

/// The close request a window manager sends when the close button is pressed.
fn request_window_close(harness: &mut Harness<'_, App>) {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
}

fn root_viewport_commands(harness: &Harness<'_, App>) -> Vec<egui::ViewportCommand> {
    harness
        .output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|viewport| viewport.commands.clone())
        .unwrap_or_default()
}

fn closed_the_window(harness: &Harness<'_, App>) -> bool {
    root_viewport_commands(harness).contains(&egui::ViewportCommand::Close)
}

/// Pressing the close button with nothing pending: the app takes the close
/// over, writes the settings, and closes the window itself.
#[test]
fn closing_the_window_takes_the_close_over_and_writes_the_settings() {
    let (mut harness, config_path) = TestHarness::builder().eframe(build_app);
    harness.inner.step();
    assert!(
        !config_path.exists(),
        "nothing has written the settings yet"
    );

    request_window_close(&mut harness.inner);
    harness.inner.step();

    assert!(
        root_viewport_commands(&harness.inner).contains(&egui::ViewportCommand::CancelClose),
        "the close was cancelled instead of tearing the app down"
    );
    assert!(config_path.exists(), "shutdown wrote the settings");
    assert!(
        harness.inner.step_until(closed_the_window),
        "the window never closed"
    );
}

/// A read-only session persists nothing, on the way out as much as during
/// the run: neither flush creates the settings file.
#[test]
fn closing_a_read_only_session_writes_no_settings() {
    let (mut harness, config_path) = TestHarness::builder().eframe(|cc, path, fading| {
        build_app_with_write_access(cc, path, fading, WriteAccess::ReadOnly)
    });
    harness.inner.step();

    harness.inner.state_mut().flush_settings();
    assert!(
        !config_path.exists(),
        "a settings change wrote the settings file"
    );

    request_window_close(&mut harness.inner);
    harness.inner.step();

    assert!(
        !config_path.exists(),
        "the flush the shutdown performs wrote the settings file"
    );
    assert!(
        harness.inner.step_until(closed_the_window),
        "the window never closed"
    );
}

/// The write held running while the shutdown window is on screen.
const TEC_COMPACTION: gt_pending_writes::WriteKind =
    gt_pending_writes::WriteKind::ArchiveCompaction {
        archive: "ionospheric TEC",
    };

/// An app with a write running and nothing asked of it yet.
fn app_with_a_running_write<'a>() -> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let compaction = harness
        .state()
        .pending_writes
        .try_begin("Compacting the TEC archive", TEC_COMPACTION)
        .expect("the registry is running");
    (harness, compaction)
}

/// An app that answered a close request while a write is still running, on the
/// frame shutdown began: the grace has not elapsed yet.
fn app_closing_over_a_running_write<'a>() -> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard)
{
    let (mut harness, compaction) = app_with_a_running_write();

    request_window_close(&mut harness);
    harness.step();
    (harness, compaction)
}

fn step_until_the_shutdown_window_is_up(harness: &mut Harness<'_, App>) {
    assert!(
        harness.step_until(|harness| harness.query_by_label("Shutting down").is_some()),
        "the shutdown window never came up"
    );
}

fn shrank_the_window(harness: &Harness<'_, App>) -> bool {
    root_viewport_commands(harness)
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::InnerSize(_)))
}

/// A write that was running when the close button was pressed holds the window
/// open. The normal UI keeps painting until the shutdown window replaces it,
/// and the window closes once the write finishes.
#[test]
fn the_normal_ui_paints_until_the_shutdown_window_replaces_it() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    assert!(
        harness.query_by_label("File").is_some(),
        "the normal UI stopped painting during the grace"
    );
    assert!(harness.query_by_label("Shutting down").is_none());
    assert!(
        !closed_the_window(&harness),
        "the window closed over a running write"
    );

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        harness.query_by_label("File").is_none(),
        "the shutdown window paints alongside the normal UI"
    );

    drop(compaction);
    assert!(
        harness.step_until(closed_the_window),
        "the window never closed after the write finished"
    );
}

/// A termination signal takes the same path as the close button: shutdown
/// begins, the shutdown window comes up over the running write, and the
/// window closes once that write finishes. There is no close event to cancel.
///
/// No other test sees the process-global flag this raises: every test runs
/// in its own process under `cargo nextest`.
#[test]
fn a_termination_signal_begins_the_same_shutdown_without_a_close_to_cancel() {
    let (mut harness, compaction) = app_with_a_running_write();

    TERMINATION_SIGNAL_FLAG.raise();
    harness.step();

    assert!(
        !root_viewport_commands(&harness).contains(&egui::ViewportCommand::CancelClose),
        "a close that was never requested was cancelled"
    );
    step_until_the_shutdown_window_is_up(&mut harness);
    assert!(
        !closed_the_window(&harness),
        "the window closed over a running write"
    );

    drop(compaction);
    assert!(
        harness.step_until(closed_the_window),
        "the window never closed after the write finished"
    );
}

/// A signal arriving after the close button started shutdown is still only
/// the first one: the close button already promised the writes would finish.
/// A force quit here would end this test's process.
#[test]
fn a_signal_after_the_close_button_leaves_the_writes_running() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    TERMINATION_SIGNAL_FLAG.raise();
    harness.step();

    step_until_the_shutdown_window_is_up(&mut harness);
    assert!(
        !closed_the_window(&harness),
        "the window closed over a running write"
    );

    drop(compaction);
    assert!(
        harness.step_until(closed_the_window),
        "the window never closed after the write finished"
    );
}

/// The shutdown window names the write it is waiting for, how far it has got
/// and which step it is on.
#[test]
fn the_shutdown_window_shows_a_running_write_with_its_progress_and_stage() {
    let (mut harness, compaction) = app_closing_over_a_running_write();
    compaction.set_progress(0.25);
    compaction.set_stage("Rewriting maps");

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some(),
        "the shutdown window never named the write it is waiting for"
    );
    assert!(harness.query_by_label("Rewriting maps").is_some());
    assert_eq!(
        harness
            .get_by_role(egui::accesskit::Role::ProgressIndicator)
            .accesskit_node()
            .numeric_value(),
        Some(25.0)
    );
    drop(compaction);
}

/// The writes shutdown already got through are listed as done.
#[test]
fn the_shutdown_window_marks_the_writes_that_finished() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        harness
            .query_by_label(&format!("{ICON_CHECK} Saving settings"))
            .is_some(),
        "the settings flush that shutdown ran is not listed as done"
    );
    drop(compaction);
}

/// "Run in background" closes the window without waiting: the write keeps
/// running, and the wait after `run_native` returns is what finishes it.
#[test]
fn running_in_the_background_closes_the_window_while_the_write_runs() {
    let (mut harness, compaction) = app_closing_over_a_running_write();
    step_until_the_shutdown_window_is_up(&mut harness);

    // The window has just shrunk: the button only takes a click at the place
    // the harness reports once the new size is laid out.
    harness.run_steps(2);

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run in background")
        .click();

    assert!(
        harness.step_until(closed_the_window),
        "the window stayed up after running in the background"
    );
    assert!(
        !harness.state().pending_writes.is_idle(),
        "the window closed only once the write had finished"
    );
    drop(compaction);
}

/// A shutdown window whose force-quit confirmation the user has just opened,
/// with the write it is waiting for still running.
fn app_with_the_force_quit_confirmation_open<'a>()
-> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard) {
    let (mut harness, compaction) = app_closing_over_a_running_write();
    step_until_the_shutdown_window_is_up(&mut harness);
    // The window has just shrunk: the button only takes a click at the place
    // the harness reports once the new size is laid out.
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label(&TEC_COMPACTION.interruption_cost())
            .is_none(),
        "the confirmation was up before anyone asked to quit"
    );

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Force quit…")
        .click();
    harness.run_steps(2);
    (harness, compaction)
}

/// "Force quit…" asks first, and the confirmation states what stopping the
/// running write costs.
#[test]
fn force_quit_confirms_and_names_what_the_running_write_costs() {
    let (harness, compaction) = app_with_the_force_quit_confirmation_open();

    assert!(
        harness
            .query_by_label(&TEC_COMPACTION.interruption_cost())
            .is_some(),
        "the confirmation never named what quitting over the write costs"
    );
    drop(compaction);
}

/// Cancelling the confirmation goes back to the shutdown window, which is
/// still up and still waiting for the write.
#[test]
fn cancelling_the_force_quit_confirmation_returns_to_the_shutdown_window() {
    let (mut harness, compaction) = app_with_the_force_quit_confirmation_open();

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
        .click();
    harness.run_steps(2);

    assert!(
        harness
            .query_by_label(&TEC_COMPACTION.interruption_cost())
            .is_none(),
        "the confirmation stayed up after it was cancelled"
    );
    assert!(harness.query_by_label("Shutting down").is_some());
    assert!(
        harness
            .query_by_label_contains("Compacting the TEC archive")
            .is_some(),
        "the shutdown window stopped listing the write it is waiting for"
    );
    assert!(!closed_the_window(&harness), "cancelling closed the window");
    assert!(!harness.state().pending_writes.is_idle());
    drop(compaction);
}

/// The window shrinks to the shutdown window's size as it comes up, and is
/// left at whatever size the user drags it to from then on.
#[test]
fn the_shutdown_window_shrinks_the_window_once() {
    let (mut harness, compaction) = app_closing_over_a_running_write();

    step_until_the_shutdown_window_is_up(&mut harness);

    assert!(
        shrank_the_window(&harness),
        "the window never shrank to the shutdown window's size"
    );
    for _ in 0..3 {
        harness.step();
        assert!(!shrank_the_window(&harness), "the window shrank again");
    }
    drop(compaction);
}

/// The instance that owns `data_directory`, with a write running that nothing
/// finishes on its own. A close request there raises the shutdown window over
/// that write, and the status this instance keeps is what a second one reads.
fn app_holding_the_data_directory_over_a_running_write<'a>(
    data_directory: &Path,
) -> (Harness<'a, App>, gt_pending_writes::PendingWriteGuard) {
    let instance_lock = SharedDataDirectoryLock::acquire(Some(data_directory));
    assert_eq!(
        instance_lock.ownership(),
        DataDirectoryOwnership::MarkedByThisInstance,
        "the holder is meant to own the data directory it reports on"
    );
    let pending_writes = PendingWrites::default();
    let compaction = pending_writes
        .try_begin("Compacting the TEC archive", TEC_COMPACTION)
        .expect("the registry is running");
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            transient_app_with_the_instance_lock(cc, &[], instance_lock, pending_writes)
        });
    harness.step();
    (harness, compaction)
}

/// The window being up is no reason to switch to it once its shutdown has
/// begun: the wait names the writes that instance is finishing instead.
#[test]
fn an_instance_shutting_down_with_its_window_up_is_named_with_what_it_is_writing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (mut holder, compaction) =
        app_holding_the_data_directory_over_a_running_write(directory.path());
    let mut waiting = app_waiting_for_the_data_directory(&[], directory.path());
    waiting.step();
    waiting.get_by_label_contains("Its window is open");

    request_window_close(&mut holder);
    step_until_the_shutdown_window_is_up(&mut holder);

    assert!(
        waiting.step_until(|waiting| waiting
            .query_by_label_contains("Compacting the TEC archive")
            .is_some()),
        "the wait never named the write the shutting-down instance is finishing"
    );
    waiting.get_by_label_contains("It is shutting down");
    drop(compaction);
}

/// The shutdown window keeps the status file current while it is up: a write
/// that finishes there drops off what a second instance reads.
#[test]
fn the_shutdown_window_reports_the_writes_left_as_they_finish() {
    let directory = tempfile::tempdir().expect("temp dir");
    let (mut holder, compaction) =
        app_holding_the_data_directory_over_a_running_write(directory.path());

    request_window_close(&mut holder);
    step_until_the_shutdown_window_is_up(&mut holder);

    let status = InstanceStatus::read_from(directory.path()).expect("the status file");
    assert_eq!(status.state, InstanceState::ShuttingDown);
    assert!(
        reports_the_compaction(&status),
        "the shutdown window never reported the write it is waiting for"
    );

    drop(compaction);
    thread::sleep(MINIMUM_INTERVAL_BETWEEN_STATUS_WRITES);

    assert!(
        holder.step_until(|_| InstanceStatus::read_from(directory.path())
            .is_some_and(|status| !reports_the_compaction(&status))),
        "the shutdown window went on reporting a write that had finished"
    );
}

/// Whether the compaction the shutdown is waiting for is among the writes
/// `status` names.
fn reports_the_compaction(status: &InstanceStatus) -> bool {
    status
        .pending_writes
        .iter()
        .any(|write| write.label == "Compacting the TEC archive")
}

/// A close frame runs in well under this even on a loaded CI machine, while
/// joining the held-open history worker on it would never return.
const CLOSE_FRAME_BUDGET: StdDuration = StdDuration::from_secs(5);

/// The history worker ends on a thread of its own: the close frame returns
/// while the worker is still on its loop, and the window closes once the
/// worker's thread ends.
///
/// The test holds that worker's request channel open, so a close frame that
/// joined the worker itself would never return. The app therefore runs on a
/// thread of its own and reports the close frame back over a channel: a
/// receive that times out fails the test within the budget.
#[test]
fn closing_the_window_hands_the_history_worker_to_its_own_thread() {
    let (close_frame_returned, close_frame_report) = mpsc::channel();
    let closing_app = thread::Builder::new()
        .name("shutdown-close-frame".to_owned())
        .spawn(move || {
            let dir = tempfile::tempdir().expect("temp dir");
            let mut harness = Harness::builder()
                .with_wait_for_pending_images(false)
                .build_eframe(transient_app);
            harness.step();
            let (worker, held_open) = crate::app::history_db::HistoryWorker::spawn_held_open(
                open_temporary_history_database(&dir.path().join("geotrace.h5")),
                harness.ctx.clone(),
                harness.state().pending_writes.clone(),
            );
            harness.state_mut().history = worker;

            request_window_close(&mut harness);
            harness.step();
            close_frame_returned.send(()).ok();

            assert!(
                !harness.state().history.available(),
                "the app kept a worker whose drop would join on the GUI thread"
            );
            harness.run_steps(3);
            assert!(
                !closed_the_window(&harness),
                "the window closed while the worker's write was still registered"
            );

            held_open.release();

            assert!(
                harness.step_until(closed_the_window),
                "the window never closed after the worker's thread ended"
            );
        })
        .expect("spawn the thread the app runs on");

    assert!(
        close_frame_report.recv_timeout(CLOSE_FRAME_BUDGET).is_ok(),
        "the close frame waited for the history worker on the GUI thread"
    );
    if let Err(panic) = closing_app.join() {
        panic::resume_unwind(panic);
    }
}

/// A closing app sends no new snap request: the auto sweep a load armed is
/// dropped once the close began.
#[test]
fn the_auto_snap_sweep_is_paused_once_the_close_began() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    // A scheduler that queues the way an online run does, over a transport
    // that reaches nothing.
    harness.state_mut().snap = crate::app::snap::SnapScheduler::new(
        harness.ctx.clone(),
        gt_fetch::TransportSource::Offline,
        false,
    );
    harness.state_mut().offline = false;
    harness.state_mut().snap_settings.acknowledge_consent();
    harness.state_mut().snap_settings.auto_snap = Some(true);
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    // Armed the way a load or a snap dialog arms it, on the frame the close
    // request arrives.
    harness.state_mut().snap_auto_sweep = true;

    request_window_close(&mut harness);
    harness.run_steps(3);

    assert!(harness.state().snap_settings.auto_snap_active());
    assert!(
        harness.state().snap.activity_for(track).is_none(),
        "a closing app enqueues no snap run"
    );
}

/// A second delete never starts underneath a running one: both would rewrite
/// the same columns.
#[test]
fn environment_auto_pruning_waits_for_a_running_delete() {
    let old = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap_or_default();
    let (mut harness, _dir, _store) = app_with_interference_days(&[old]);
    enable_environment_auto_prune(&mut harness, 12);
    let request = harness
        .state()
        .environment_auto_prune_request()
        .expect("the archive holds a day past the age");

    let ctx = harness.ctx.clone();
    harness.state_mut().start_environment_prune(&ctx, request);

    assert!(
        harness.state().environment_auto_prune_request().is_none(),
        "a delete is already running"
    );
}

/// Push a file built with the given travel mode into the app's loaded files,
/// returning the ref of its single track.
fn push_file_with_travel_mode(
    harness: &mut Harness<'_, App>,
    name: &str,
    travel_mode: Option<gt_types::TravelMode>,
) -> gt_types::TrackRef {
    push_file_with(
        harness,
        name,
        travel_mode,
        gt_loaded_files::FileHistory::None,
    )
}

/// Push a two-track recording (the tracks split at a 10 minute gap),
/// returning its file index.
fn push_two_track_file(harness: &mut Harness<'_, App>, name: &str) -> gt_types::FileIdx {
    let points = gt_test_utils::fixtures::nav_data_with_gap(30, 30);
    let fi = push_points_as(
        harness,
        name,
        &points,
        None,
        gt_loaded_files::FileHistory::None,
    );
    let track_count = {
        let state = harness.state();
        let shared = state.shared.borrow();
        fi.get(shared.loaded_files.files()).map(|f| f.tracks.len())
    };
    assert_eq!(
        track_count,
        Some(2),
        "the gap must split the recording in two"
    );
    fi
}

fn push_file_with(
    harness: &mut Harness<'_, App>,
    name: &str,
    travel_mode: Option<gt_types::TravelMode>,
    history: gt_loaded_files::FileHistory,
) -> gt_types::TrackRef {
    let points = gt_test_utils::nav_test_data();
    let fi = push_points_as(harness, name, &points, travel_mode, history);
    gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(0))
}

/// Build a recording from `points` and push it into the app's loaded
/// files, returning its file index.
fn push_points_as(
    harness: &mut Harness<'_, App>,
    name: &str,
    points: &[gt_types::NavPoint],
    travel_mode: Option<gt_types::TravelMode>,
    history: gt_loaded_files::FileHistory,
) -> gt_types::FileIdx {
    let file = gt_track_builder::build_loaded_file(
        name.to_owned(),
        points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(std::path::PathBuf::from(name)),
        gt_track_builder::FileMeta {
            travel_mode,
            ..gt_track_builder::FileMeta::default()
        },
        vec![],
    );
    let state = harness.state_mut();
    let mut shared = state.shared.borrow_mut();
    shared.loaded_files.push(file, history);
    let fi = gt_types::FileIdx::new(shared.loaded_files.files().len() - 1);
    let files = shared.loaded_files.files().to_vec();
    shared.tree.sync_from_loaded_files(&files);
    fi
}

/// A boat-declared file's track resolves to the unsnappable row (hover names
/// the mode), while an undeclared file stays idle (no entry).
#[test]
fn snap_row_views_marks_declared_roadless_modes_unsnappable() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let boat =
        push_file_with_travel_mode(&mut harness, "boat.gtd", Some(gt_types::TravelMode::Boat));
    let plain = push_file_with_travel_mode(&mut harness, "plain.gtd", None);

    let rows = harness.state().snap_row_views();

    assert_eq!(
        rows.get(&boat),
        Some(&gt_side_panel::SnapRowView::Unsnappable {
            travel_mode: "Boat".to_owned()
        })
    );
    assert_eq!(rows.get(&plain), None, "an undeclared file stays idle");
}

/// The full consent round trip for a snap trigger: the request parks on
/// `pending_snap` and raises the dialog, agreeing takes it (the run is
/// queued - a no-op in the offline test app, so only the take is
/// observable), declining drops it.
#[test]
fn snap_request_parks_on_consent_and_agree_takes_it() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);

    harness.state_mut().handle_snap_request(vec![track]);
    assert!(harness.state().snap_consent_prompt);
    assert_eq!(harness.state().pending_snap.track_refs, vec![track]);
    harness.step();

    // First synthetic click settles the startup map-layer popup (see
    // snap_consent_agree_persists_the_server_host), the second lands.
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - snap automatically")
        .click();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - snap automatically")
        .click();
    harness.run_steps(3);

    assert!(!harness.state().snap_consent_prompt);
    assert!(
        harness.state().pending_snap.track_refs.is_empty(),
        "agreeing must take the parked request and queue it"
    );
    assert!(harness.state().snap_settings.consent_granted());
    assert_eq!(harness.state().snap_settings.auto_snap, Some(true));
}

#[test]
fn snap_request_parked_on_consent_is_dropped_on_decline() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);

    harness.state_mut().handle_snap_request(vec![track]);
    harness.step();

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(!harness.state().snap_consent_prompt);
    assert!(
        harness.state().pending_snap.track_refs.is_empty(),
        "declining must drop the parked request"
    );
    assert!(!harness.state().snap_settings.consent_granted());
    assert_eq!(
        harness.state().snap.activity_for(track),
        None,
        "nothing may be queued without consent"
    );
}

/// The snap error series is built once per run: consecutive frames hand
/// out the same `Arc` (the plot's mipmap cache keys off it), and a new run
/// for the track produces a new one.
#[test]
fn snap_error_series_is_stable_across_frames() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    inject_completed_run(&mut harness, track);

    let first = harness.state_mut().snap_error_view();
    let second = harness.state_mut().snap_error_view();
    let (a, b) = (
        first.points_by_track.get(&track).expect("series present"),
        second.points_by_track.get(&track).expect("series present"),
    );
    assert!(
        Arc::ptr_eq(a, b),
        "consecutive frames must reuse the same series allocation"
    );

    // A new run for the same track invalidates: fresh points, fresh Arc.
    inject_completed_run(&mut harness, track);
    let third = harness.state_mut().snap_error_view();
    let c = third.points_by_track.get(&track).expect("series present");
    assert!(
        !Arc::ptr_eq(a, c),
        "a new run must produce a new series allocation"
    );
}

/// The costing override flow: a "Snap again as" choice beats the declared
/// travel mode - a boat-declared (unsnappable) track becomes snappable
/// under the chosen costing, and clearing state on index changes does not
/// lose the content-keyed override.
#[test]
fn costing_override_beats_the_declared_mode() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let track =
        push_file_with_travel_mode(&mut harness, "boat.gtd", Some(gt_types::TravelMode::Boat));
    harness.run_steps(2);

    {
        let state = harness.state();
        let shared = state.shared.borrow();
        let files = shared.loaded_files.files();
        let file = track.fi.get(files).expect("file");
        let loaded = track.resolve(files).expect("track");
        assert_eq!(
            state.effective_costing(file, loaded),
            None,
            "boat: unsnappable"
        );
    }

    // Consent is pending in a fresh app, so the choice parks on the
    // consent dialog and changes nothing yet.
    let request_pedestrian = |harness: &mut Harness<'_, App>| {
        harness.state_mut().handle_snap_costing_request(
            gt_side_panel::SnapCostingTarget::Track(track),
            gt_ui_types::SnapCosting::Pedestrian,
        );
    };
    let resolved_costing = |harness: &Harness<'_, App>| {
        let state = harness.state();
        let shared = state.shared.borrow();
        let files = shared.loaded_files.files();
        let file = track.fi.get(files)?;
        let loaded = track.resolve(files)?;
        state.effective_costing(file, loaded)
    };
    request_pedestrian(&mut harness);
    assert!(harness.state().snap_consent_prompt);
    assert_eq!(harness.state().pending_snap.track_refs, vec![track]);
    assert_eq!(
        resolved_costing(&harness),
        None,
        "parked: still unsnappable"
    );

    harness.state_mut().snap_settings.acknowledge_consent();
    request_pedestrian(&mut harness);
    assert_eq!(
        resolved_costing(&harness),
        Some(gt_snap::wire::Costing::Pedestrian),
        "the override beats the road-less declaration"
    );
}

/// With consent already granted, a "Snap again as" choice must dispatch
/// under the chosen costing - not the plainly resolved one. Discriminated
/// via the cache: with runs cached under both costings (auto being the
/// displayed one), confirming the bicycle choice replaces the bicycle
/// entry and leaves auto's alone only when the dispatch actually resolves
/// the override (regression test for the dispatch ignoring it and hitting
/// the auto entry instead).
#[test]
fn costing_override_reaches_the_dispatched_run() {
    use crate::app::snap::{SnapCacheKey, SnapRun};
    use gt_snap::merge::{self, SnapWarningReporter};
    use gt_snap::request_plan::SnapParams;
    use gt_snap::wire::Costing;

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    harness.step();

    let seeded = |harness: &Harness<'_, App>, costing: Costing| {
        let state = harness.state();
        let shared = state.shared.borrow();
        let loaded = track.resolve(shared.loaded_files.files()).expect("track");
        let params = SnapParams::new(costing);
        let key = SnapCacheKey::new(
            loaded,
            params,
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        );
        let run = SnapRun::new(
            merge::merge(
                &gt_snap::request_plan::plan(&[]),
                params,
                &[],
                &SnapWarningReporter::default(),
            ),
            Vec::new(),
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        );
        (key, run)
    };
    // Bicycle first, then auto: auto is the displayed run, bicycle sits
    // only in the dedupe cache.
    let (key, run) = seeded(&harness, Costing::Bicycle);
    harness.state_mut().snap.insert_run(key, run);
    let (key, run) = seeded(&harness, Costing::Auto);
    harness.state_mut().snap.insert_run(key, run);

    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Track(track),
        gt_ui_types::SnapCosting::Bicycle,
    );
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Snap again")
        .click();
    harness.run_steps(3);

    let state = harness.state();
    assert!(
        !state.snap_consent_prompt,
        "consent was granted - the request must not re-prompt"
    );
    let shared = state.shared.borrow();
    let loaded = track.resolve(shared.loaded_files.files()).expect("track");
    let cached = |costing| {
        state
            .snap
            .has_cached_run(loaded, state.snap_settings.params(costing))
    };
    assert!(
        !cached(Costing::Bicycle),
        "the dispatch resolved the override, replacing the bicycle entry"
    );
    assert!(cached(Costing::Auto), "the other costing's run is kept");
}

/// Whether the scheduler still holds a cached auto-costing run for `track`.
fn has_cached_auto_run(harness: &Harness<'_, App>, track: gt_types::TrackRef) -> bool {
    let state = harness.state();
    let shared = state.shared.borrow();
    let loaded = track.resolve(shared.loaded_files.files()).expect("track");
    state.snap.has_cached_run(
        loaded,
        state.snap_settings.params(gt_snap::wire::Costing::Auto),
    )
}

/// A harness whose single track already has a cached auto-costing run,
/// with the auto choice requested and its dialog on screen.
fn harness_prompting_to_replace_the_auto_run<'a>() -> (Harness<'a, App>, gt_types::TrackRef) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    harness.step();
    inject_completed_run(&mut harness, track);
    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Track(track),
        gt_ui_types::SnapCosting::Auto,
    );
    harness.run_steps(3);
    (harness, track)
}

/// A "Snap again as" choice for a costing the track already has a run for
/// prompts before replacing it, and cancelling keeps that run.
#[test]
fn costing_choice_with_a_cached_run_prompts_before_replacing_it() {
    let (mut harness, track) = harness_prompting_to_replace_the_auto_run();

    assert_eq!(
        harness.state().snap_replace_prompt.map(|p| p.choice),
        Some(gt_ui_types::SnapCosting::Auto)
    );
    assert_eq!(
        harness.state().snap.activity_for(track),
        None,
        "nothing runs while the dialog is open"
    );

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(harness.state().snap_replace_prompt.is_none());
    assert!(
        has_cached_auto_run(&harness, track),
        "cancelling keeps the stored run"
    );
}

/// Confirming the dialog forgets the cached run, so the choice reaches the
/// server instead of redisplaying what the track already had.
#[test]
fn confirming_the_replace_prompt_discards_the_cached_run() {
    let (mut harness, track) = harness_prompting_to_replace_the_auto_run();

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Snap again")
        .click();
    harness.run_steps(3);

    assert!(harness.state().snap_replace_prompt.is_none());
    assert!(
        !has_cached_auto_run(&harness, track),
        "the confirmed choice must reach the server, not the cache"
    );
}

/// Add `track` to the panel's selection, as clicking its row would.
fn select_track(harness: &mut Harness<'_, App>, track: gt_types::TrackRef) {
    let state = harness.state_mut();
    let mut shared = state.shared.borrow_mut();
    shared
        .tree
        .selection
        .insert(gt_side_panel::NodeKey::Track(track));
}

/// The session costing override stored for `track`, if any.
fn costing_override(
    harness: &Harness<'_, App>,
    track: gt_types::TrackRef,
) -> Option<gt_snap::wire::Costing> {
    let state = harness.state();
    let shared = state.shared.borrow();
    let loaded = track.resolve(shared.loaded_files.files())?;
    state
        .snap_costing_overrides
        .get(&crate::app::snap::TrackContentKey::new(loaded))
        .copied()
}

/// The scope dialog's counts separate the recording's selected tracks from
/// all of them, and report how many already have data for the costing.
#[test]
fn recording_scope_counts_separate_selected_from_all() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let fi = push_two_track_file(&mut harness, "tour.gtd");
    harness.step();
    let second = gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(1));
    inject_completed_run(
        &mut harness,
        gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(0)),
    );
    let prompt = crate::app::SnapScopePrompt {
        fi,
        choice: gt_ui_types::SnapCosting::Auto,
    };

    let unselected = harness.state().snap_scope_counts(prompt);
    assert_eq!(
        unselected.selected,
        crate::app::modals::SnapScopeCount::default()
    );

    select_track(&mut harness, second);
    let counts = harness.state().snap_scope_counts(prompt);
    assert_eq!(
        counts.selected,
        crate::app::modals::SnapScopeCount {
            tracks: 1,
            already_snapped: 0
        }
    );
    assert_eq!(
        counts.all,
        crate::app::modals::SnapScopeCount {
            tracks: 2,
            already_snapped: 1
        }
    );
}

/// Each scope button runs exactly its own tracks: their cached runs for
/// the chosen costing are replaced and they take the override, while the
/// tracks outside the scope keep theirs.
#[rstest::rstest]
#[case::selected_scope("Snap selected tracks", &[1])]
#[case::all_scope("Snap all tracks", &[0, 1])]
#[case::cancel("Cancel", &[])]
fn recording_scope_dialog_snaps_the_chosen_scope(#[case] button: &str, #[case] expected: &[usize]) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    let fi = push_two_track_file(&mut harness, "tour.gtd");
    harness.step();
    let track = |ti| gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(ti));
    inject_completed_run(&mut harness, track(0));
    inject_completed_run(&mut harness, track(1));
    select_track(&mut harness, track(1));

    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Recording(fi),
        gt_ui_types::SnapCosting::Auto,
    );
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, button)
        .click();
    harness.run_steps(3);

    assert!(harness.state().snap_scope_prompt.is_none());
    for ti in 0..2 {
        let in_scope = expected.contains(&ti);
        assert_eq!(
            costing_override(&harness, track(ti)),
            in_scope.then_some(gt_snap::wire::Costing::Auto),
            "track {ti} override"
        );
        assert_eq!(
            has_cached_auto_run(&harness, track(ti)),
            !in_scope,
            "track {ti} cached run"
        );
    }
}

/// A bulk choice on an app without consent parks the whole batch on one
/// dialog and touches nothing until it is accepted: the tracks keep their
/// runs while the question is open, and agreeing releases the batch.
#[test]
fn recording_scope_parks_the_whole_batch_on_one_consent_dialog() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let fi = push_two_track_file(&mut harness, "tour.gtd");
    harness.step();
    let track = |ti| gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(ti));
    inject_completed_run(&mut harness, track(0));
    inject_completed_run(&mut harness, track(1));

    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Recording(fi),
        gt_ui_types::SnapCosting::Auto,
    );
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Snap all tracks")
        .click();
    harness.run_steps(3);

    assert!(harness.state().snap_consent_prompt);
    assert_eq!(harness.state().pending_snap.track_refs.len(), 2);
    for ti in 0..2 {
        assert!(
            has_cached_auto_run(&harness, track(ti)),
            "track {ti} keeps its run until consent"
        );
    }

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - snap automatically")
        .click();
    harness.run_steps(3);

    assert!(harness.state().pending_snap.track_refs.is_empty());
    for ti in 0..2 {
        assert!(
            !has_cached_auto_run(&harness, track(ti)),
            "track {ti} runs once the batch is released"
        );
    }
}

/// The persistence integration end to end, against a real temporary database:
/// a completed run of a history-stored file is written into the
/// recording's snap blob via the worker, and feeding the stored blob back
/// through the response handler seeds a fresh scheduler's stores.
#[test]
fn snap_runs_persist_and_restore_through_the_app() {
    use geotrace_sdk::{Angle, DateTime, Duration as SdkDuration, NavFileBuilder, NavFix};
    use gt_store::{HistoryDatabase, Recordings, StoredSegmentation, TrackRange};

    // One real recording so the blob has a valid group to live in.
    let t0 = DateTime::from_timestamp(1_000, 0).expect("valid timestamp");
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..10i64 {
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t0 + SdkDuration::seconds(i))
                .lat(Angle::degrees(55.68))
                .lon(Angle::degrees(12.56))
                .heading(Angle::degrees(0.0))
                .build(),
        );
    }
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).expect("write bytes");

    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Recordings::open_or_create(&db_path).expect("open");
    let meta = gt_store::extract_meta(&bytes).expect("meta");
    let tracks = [TrackRange {
        start: 0,
        end: meta.nav_point_count,
        hidden: false,
    }];
    let settings = StoredSegmentation {
        track_split_gap_us: 300_000_000,
        detect_clock_discontinuities: false,
        clock_discontinuity_sigmas: 5.0,
    };
    let db_ref = db
        .insert("dev", &meta, &tracks, settings, &bytes)
        .expect("insert");

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().history = crate::app::history_db::HistoryWorker::spawn(
        Recordings::open_or_create(&db_path).expect("reopen"),
        egui::Context::default(),
        gt_pending_writes::PendingWrites::default(),
    );

    // A loaded file associated with the stored recording, with a completed
    // run in the session stores.
    let track = push_file_with(
        &mut harness,
        "ride.gtd",
        None,
        gt_loaded_files::FileHistory::recording("dev".to_owned(), meta, Some(db_ref.clone())),
    );
    inject_completed_run(&mut harness, track);

    // Persist leg: the worker writes the recording's blob.
    let content = {
        let state = harness.state();
        let shared = state.shared.borrow();
        let loaded = track
            .resolve(shared.loaded_files.files())
            .expect("track present");
        crate::app::snap::TrackContentKey::new(loaded)
    };
    harness.state().persist_snap_runs(&[content]);
    let blob = harness
        .step_until_some(|_| {
            Recordings::open_or_create(&db_path)
                .ok()
                .and_then(|db| db.snap_blob(&db_ref).ok())
                .flatten()
        })
        .expect("the history worker stored the snap blob");

    // Restore leg: a fresh scheduler seeded through the response handler.
    harness.state_mut().snap = crate::app::snap::SnapScheduler::new(
        egui::Context::default(),
        gt_fetch::TransportSource::Offline,
        true,
    );
    harness
        .state_mut()
        .handle_history_response(crate::app::history_db::Response::SnapRunsLoaded {
            db_ref,
            blob: Ok(Some(blob)),
        });
    let state = harness.state();
    let shared = state.shared.borrow();
    let loaded = track
        .resolve(shared.loaded_files.files())
        .expect("track present");
    let restored = state
        .snap
        .latest_run_for(loaded)
        .expect("stored run restored into the fresh session");
    assert_eq!(restored.result.points.len(), 60);
}

/// Inject a completed run for `track` straight into the scheduler cache,
/// keyed the way the app's view builders look it up: one snapped segment for
/// the map, and per-point results for the plot - errors for the first sixty
/// points with an unsnapped stretch at indices 20..25, so the snap error
/// series has a line break and markers to show.
fn inject_completed_run(harness: &mut Harness<'_, App>, track: gt_types::TrackRef) {
    use crate::app::snap::{SnapCacheKey, SnapRun};
    use gt_snap::merge::{SnapPoint, SnapResult};
    use gt_snap::snapped_track::{Position, SnappedTrackSegment};
    use gt_snap::wire::{Costing, SnapPointKind};

    let points: Vec<SnapPoint> = (0..60)
        .map(|i| {
            let kind = if (20..25).contains(&i) {
                SnapPointKind::Unsnapped
            } else if i % 2 == 0 {
                SnapPointKind::Snapped
            } else {
                SnapPointKind::Interpolated
            };
            SnapPoint {
                point: gt_types::PointIdx::new(i),
                kind,
                error_m: (kind != SnapPointKind::Unsnapped)
                    .then(|| 2.0 + f64::from(u8::try_from(i % 7).unwrap_or(0))),
                snapped: None,
                edge: None,
                follows_gap: i == 0,
            }
        })
        .collect();
    let result = SnapResult {
        points,
        segments: vec![SnappedTrackSegment {
            positions: vec![
                Position {
                    lat: 55.68,
                    lon: 12.56,
                },
                Position {
                    lat: 55.69,
                    lon: 12.57,
                },
            ],
            edge_spans: Vec::new(),
        }],
        edges: Vec::new(),
        kind_counts: gt_snap::merge::SnapKindCounts::default(),
        confidence_score: None,
        osm_changeset: None,
        params: gt_snap::request_plan::SnapParams::new(Costing::Auto),
        gps_accuracy_sent_m: None,
        partial: false,
    };
    let key = {
        let state = harness.state();
        let shared = state.shared.borrow();
        let loaded_track = track
            .resolve(shared.loaded_files.files())
            .expect("track just pushed");
        SnapCacheKey::new(
            loaded_track,
            gt_snap::request_plan::SnapParams::new(Costing::Auto),
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        )
    };
    harness.state_mut().snap.insert_run(
        key,
        SnapRun::new(
            result,
            Vec::new(),
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        ),
    );
}

/// The map's snapped-track geometry follows the completed run's toggle and
/// the track's tree visibility - hidden either way means no entry.
#[test]
fn snapped_tracks_view_respects_toggle_and_tree_visibility() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    inject_completed_run(&mut harness, track);

    let view = harness.state().snapped_tracks_view();
    let geometry = view
        .get(track)
        .expect("a shown completed run must reach the map");
    assert_eq!(
        geometry.segments.len(),
        1,
        "one snapped segment was injected"
    );
    assert_eq!(
        geometry.segments.first().map(|s| s.points.len()),
        Some(2),
        "both positions must be projected"
    );

    // Toggled hidden: gone from the map view.
    harness.state_mut().hidden_snapped.insert(track);
    assert!(harness.state().snapped_tracks_view().is_empty());
    harness.state_mut().hidden_snapped.remove(&track);

    // Track unchecked in the tree: gone as well.
    harness
        .state()
        .shared
        .borrow_mut()
        .tree
        .toggle_track_check(track);
    assert!(harness.state().snapped_tracks_view().is_empty());
}

/// A recording whose clock offset holds near −234 ms, with one sample carrying
/// a 1 h 09 m recording gap - the `gnss.h5.gtd` case, where the receiver
/// reported its pre-gap GPS epoch for the first fix after resuming.
fn clock_excursion_gtd_bytes() -> Vec<u8> {
    use geotrace_sdk::{Angle, Duration as SdkDuration, NavFileBuilder, NavFix};

    let start = base_time();
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..61i64 {
        let gps = start + SdkDuration::seconds(i);
        let ahead_ms = if i == 10 { 4_127_054 } else { 234 };
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(gps)
                .sys_time(gps + SdkDuration::milliseconds(ahead_ms))
                .lat(Angle::degrees(51.5 + i as f64 * 0.0002))
                .lon(Angle::degrees(-0.1 - i as f64 * 0.00015))
                .heading(Angle::degrees(270.0))
                .eph_m(2.4)
                .build(),
        );
    }
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).expect("write bytes");
    bytes
}

/// The clock offset excursion overlay: the offset line keeps the track's own
/// sub-second scale, and the sample carrying the recording gap is marked with a
/// down-pointing indicator at the bottom edge, on a stub from the baseline.
#[test]
fn snapshot_app_plot_clock_excursion() {
    let gtd_bytes = clock_excursion_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::ClockDeltaMs);
        }
    }
    harness.run_steps(5);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_loose("app_plot_clock_excursion");
}

/// The plot's snap error series from an injected completed run: the mint
/// line breaks over the unsnapped stretch and cross markers sit on the
/// baseline there. Only Eph stays enabled alongside so the snapshot shows
/// the claimed-accuracy vs. observed-deviation overlay the metric exists for.
#[test]
fn snapshot_app_plot_snap_error() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    let track = gt_types::TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
    inject_completed_run(&mut harness, track);
    harness.run_steps(5);

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        use strum::IntoEnumIterator as _;
        for kind in gt_types::MetricKind::iter() {
            vis.set(
                kind,
                matches!(
                    kind,
                    gt_types::MetricKind::Eph | gt_types::MetricKind::SnapError
                ),
            );
        }
    }
    harness.run_steps(5);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_loose("app_plot_snap_error");
}

/// A Kp day archived as the fetch worker leaves one: eight three-hour
/// periods climbing through the storm levels.
fn archive_kp_day(store: &gt_store::SolarStore, day: chrono::NaiveDate) {
    let midnight = day.and_time(chrono::NaiveTime::MIN).and_utc();
    let samples = (0..8_u32)
        .map(|period| gt_solar::series::KpSample {
            period_start: midnight + chrono::TimeDelta::hours(i64::from(period) * 3),
            activity: gt_solar::activity::GeomagneticActivity::from_published_value(
                gt_solar::GeomagneticIndex::Kp,
                2.0 + f64::from(period % 5),
            ),
            status: gt_solar::series::KpStatus::Definitive,
        })
        .collect();
    store
        .insert_or_replace_kp_day(
            day,
            "host",
            Utc::now(),
            &gt_solar::series::KpSeries { samples },
        )
        .expect("insert kp");
    store
        .insert_or_replace_hp30_day(
            day,
            "host",
            Utc::now(),
            &gt_solar::series::Hp30Series { samples: vec![] },
        )
        .expect("insert hp30");
}

/// The Kp line is drawn from the archive across the whole span the plot
/// shows: it runs past both ends of the recording into the margins, and
/// breaks over the day nothing is archived for while the recording runs
/// straight through it.
#[test]
fn snapshot_app_plot_context_line_spans_the_archived_days() {
    let gtd_bytes = synthetic_gtd_bytes(SyntheticGtdSpec {
        start: base_time(),
        point_count: 61,
        step_secs: 3600,
        start_lat_deg: 51.5,
        start_lon_deg: -0.1,
        lat_step_deg: 0.0002,
        lon_step_deg: -0.00015,
        heading_deg: 270.0,
        speed_kmh: 22.0,
        eph_m: 2.4,
        sats_seen: 10,
        sats_in_fix: 8,
    });
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    // The recording spans 22nd to 26th May 2025. The 24th stays unarchived,
    // and the days either side of the recording carry the margins.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_geomagnetic_indices()
        .expect("archive");
    let recorded = base_time().date_naive();
    for offset in [-1_i64, 0, 2, 3, 4] {
        let day = recorded + chrono::TimeDelta::days(offset);
        archive_kp_day(&store, day);
    }
    let ctx = harness.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(store),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::Kp);
        }
    }
    harness.run_steps(5);
    // The archived storm days raise the space weather warning, whose toast
    // has its own snapshot.
    harness.state_mut().toasts.dismiss_all_toasts();
    harness.run_steps(2);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_loose("app_plot_context_line");
}

/// A geomagnetic day archived after the recording was loaded reaches it: the
/// warning line behind the map indicator appears and the load toast is raised
/// once, however many frames follow.
#[test]
fn a_storm_day_archived_after_the_load_warns_on_the_map() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    harness.run_steps(2);
    assert!(
        harness.state().space_weather_warning.lines().is_empty(),
        "no archived day overlaps the recording yet"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_geomagnetic_indices()
        .expect("archive");
    archive_kp_day(&store, base_time().date_naive());
    let ctx = harness.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(store),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(2);

    assert_eq!(
        harness
            .state()
            .space_weather_warning
            .lines()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["Geomagnetic storm: Kp reached 5 (G1)"],
        "the period the recording's fixes fall in is what it carries"
    );
    assert_eq!(harness.state().toasts.len(), 1);

    harness.run_steps(2);
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "a later frame does not raise the toast again"
    );
}

/// Archive one UTC day of TEC maps over the recording's own position, every
/// node standing at `tecu`, two hours apart.
fn archive_tec_day(store: &gt_store::IonexStore, day: chrono::NaiveDate, tecu: f64) {
    let axis = |first_degrees: f64, last_degrees: f64, step_degrees: f64| {
        gt_ionex::grid::GridAxis::new(gt_ionex::grid::AxisDeclaration {
            first_degrees,
            last_degrees,
            step_degrees,
        })
        .expect("axis")
    };
    let grid = gt_ionex::grid::MapGrid {
        latitudes: gt_ionex::grid::LatitudeAxis::new(axis(55.0, 50.0, -2.5)),
        longitudes: gt_ionex::grid::LongitudeAxis::new(axis(-5.0, 5.0, 5.0)),
        shell_height_km: 450.0,
    };
    let midnight = day.and_time(chrono::NaiveTime::MIN).and_utc();
    let maps = (0..=12)
        .map(|step| {
            gt_ionex::maps::TecMap::new(
                midnight + chrono::TimeDelta::hours(step * 2),
                vec![vec![Some(gt_ionex::tec::TotalElectronContent::from_tecu(tecu)); 3]; 3],
            )
        })
        .collect();
    store
        .insert_or_replace_day(
            day,
            "host",
            Utc::now(),
            gt_ionex::IonexProduct::Final,
            &gt_ionex::maps::GlobalIonosphereMaps::new(grid, chrono::TimeDelta::hours(2), maps),
        )
        .expect("insert TEC day");
}

/// The quiet-time window before a loaded recording arrives after it: the map
/// indicator names the deviation and the load toast is raised once, however
/// many frames follow.
#[test]
fn a_tec_window_archived_after_the_load_warns_on_the_map() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    harness.run_steps(2);
    assert!(
        harness.state().space_weather_warning.lines().is_empty(),
        "no archived day overlaps the recording yet"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_tec_maps()
        .expect("archive");
    let recorded = base_time().date_naive();
    archive_tec_day(&store, recorded, 35.0);
    for days_before in 1..=gt_ionex::quiet_time::BACKGROUND_WINDOW_DAYS as i64 {
        archive_tec_day(
            &store,
            recorded - chrono::TimeDelta::days(days_before),
            20.0,
        );
    }
    let ctx = harness.ctx.clone();
    harness.state_mut().tec_maps = crate::app::tec::TecMapScheduler::new(
        ctx,
        Some(store),
        gt_ionex::MirrorList::default(),
        None,
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(2);

    assert_eq!(
        harness
            .state()
            .space_weather_warning
            .lines()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "TEC deviation: +75 % from the 27-day median, moderate ionospheric storm (W = 3)",
            "TEC over the recording: 35 to 35 TECU",
        ],
        "the recording's day stands well above the median of the 27 days before it"
    );
    assert_eq!(harness.state().toasts.len(), 1);

    harness.run_steps(2);
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "a later frame does not raise the toast again"
    );
}

/// The flares of the May 2024 storm, as the fetch worker archives a day:
/// classes spread across the scale so each marker colour is drawn.
fn archive_flare_day(store: &gt_store::FlareStore, day: chrono::NaiveDate, peaks: &[(u32, &str)]) {
    let flares: Vec<gt_flare::SolarFlare> = peaks
        .iter()
        .map(|&(hour, class_type)| {
            let peak = day.and_hms_opt(hour, 13, 0).unwrap_or_default().and_utc();
            gt_flare::SolarFlare {
                id: format!("{peak}-FLR-001"),
                begin: peak - chrono::TimeDelta::minutes(28),
                peak,
                end: Some(peak + chrono::TimeDelta::minutes(23)),
                classification: class_type.parse().expect("a published class"),
                source_location: Some("S20W25".to_owned()),
                active_region: Some(13664),
            }
        })
        .collect();
    store
        .insert_or_replace_day(day, "host", Utc::now(), &flares)
        .expect("archive the day");
}

/// A harness with a recording loaded and an archive of flares behind it,
/// returning the temp directory the archive lives in.
fn harness_with_archived_flares<'a>(
    archived: &[(i64, &[(u32, &str)])],
) -> (Harness<'a, App>, tempfile::TempDir) {
    let gtd_bytes = synthetic_gtd_bytes(SyntheticGtdSpec {
        start: base_time(),
        point_count: 61,
        step_secs: 3600,
        start_lat_deg: 51.5,
        start_lon_deg: -0.1,
        lat_step_deg: 0.0002,
        lon_step_deg: -0.00015,
        heading_deg: 270.0,
        speed_kmh: 22.0,
        eph_m: 2.4,
        sats_seen: 10,
        sats_in_fix: 8,
    });
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_solar_flares()
        .expect("archive");
    let recorded = base_time().date_naive();
    for &(offset, peaks) in archived {
        archive_flare_day(&store, recorded + chrono::TimeDelta::days(offset), peaks);
    }
    let ctx = harness.ctx.clone();
    harness.state_mut().solar_flares = crate::app::flares::SolarFlareScheduler::new(
        ctx,
        Some(store),
        gt_flare::DEFAULT_BASE_URL.to_owned(),
        gt_flare::ApiKey::new("test-key"),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(5);
    // The archived flares raise the space weather warning, whose toast has
    // its own snapshot.
    harness.state_mut().toasts.dismiss_all_toasts();
    harness.run_steps(2);
    (harness, dir)
}

/// The markers are drawn from the archive across the whole span the plot
/// shows, so they reach past both ends of the recording, and each one takes
/// the colour of its class.
#[test]
fn snapshot_app_plot_solar_flare_markers() {
    // The plot shows the middle of the recording, so the day before it
    // carries a flare that is archived and outside the view.
    let (mut harness, _dir) = harness_with_archived_flares(&[
        (-1, &[(6, "C4.5")]),
        (0, &[(20, "C4.5")]),
        (1, &[(6, "M9.0"), (14, "X2.2")]),
        (2, &[(10, "X5.8")]),
    ]);

    {
        let state = harness.state_mut();
        let mut shared = state.shared.borrow_mut();
        let vis = &mut shared.plot_state.metric_vis;
        for kind in gt_types::MetricKind::iter() {
            vis.set(kind, kind == gt_types::MetricKind::Eph);
        }
    }
    harness.run_steps(5);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_loose("app_plot_solar_flare_markers");
}

#[test]
fn snapshot_app_environment_chip_hover() {
    let (mut harness, _archived) = harness_with_archived_flares(&[(0, &[(9, "X2.2")])]);
    // Tall enough for the whole tooltip to fit under the chip row.
    harness.set_size(egui::vec2(800.0, 900.0));
    harness.run_steps(3);
    harness.get_by_label(gt_flare::text::LAYER_LABEL).hover();
    // Tooltips appear after egui's hover delay.
    harness.run_steps(60);

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_loose("app_environment_chip_hover");
}

/// With nothing archived the flare chip renders disabled - visible, not
/// hidden - and an archived day enables it.
#[test]
fn solar_flare_chip_is_disabled_until_a_day_is_archived() {
    let (harness, _empty) = harness_with_archived_flares(&[]);
    let chip = harness.get_by_label(gt_flare::text::LAYER_LABEL);
    assert!(
        chip.accesskit_node().is_disabled(),
        "the chip must render disabled while no flare is archived"
    );

    let (harness, _archived) = harness_with_archived_flares(&[(0, &[(9, "X2.2")])]);
    let chip = harness.get_by_label(gt_flare::text::LAYER_LABEL);
    assert!(
        !chip.accesskit_node().is_disabled(),
        "an archived flare enables the chip"
    );
}

/// Without a completed run the snap error chip renders disabled - visible,
/// not hidden - and enabling runs changes nothing else about the chip row.
#[test]
fn snap_error_chip_is_disabled_until_a_run_completes() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    let track = gt_types::TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
    harness.run_steps(3);

    let chip = harness.get_by_label("Snap error (m)");
    assert!(
        chip.accesskit_node().is_disabled(),
        "the chip must render disabled while no run has completed"
    );

    inject_completed_run(&mut harness, track);
    harness.run_steps(3);
    let chip = harness.get_by_label("Snap error (m)");
    assert!(
        !chip.accesskit_node().is_disabled(),
        "a completed run enables the chip"
    );
}

/// `snap_error_view` resolves each sent point's `PointIdx` to its plot time
/// and mirrors the kind. Unsnapped points keep `error_m: None`.
#[test]
fn snap_error_view_resolves_point_times_and_kinds() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    inject_completed_run(&mut harness, track);

    let view = harness.state_mut().snap_error_view();
    let points = view
        .points_by_track
        .get(&track)
        .expect("the completed run must reach the plot view");
    assert_eq!(points.len(), 60, "one entry per injected sent point");

    let shared = harness.state().shared.borrow();
    let loaded = track
        .resolve(shared.loaded_files.files())
        .expect("track present");
    let first_time = loaded.points[0].tpv.time().as_secs_f64();
    assert!(
        (points[0].x_secs - first_time).abs() < f64::EPSILON,
        "x must be the point's own plot time"
    );
    assert_eq!(points[0].kind, gt_ui_types::SnapErrorKind::Snapped);
    assert_eq!(points[20].kind, gt_ui_types::SnapErrorKind::Unsnapped);
    assert_eq!(points[20].error_m, None);
    assert_eq!(points[21].kind, gt_ui_types::SnapErrorKind::Unsnapped);
    assert_eq!(points[25].kind, gt_ui_types::SnapErrorKind::Interpolated);
    assert!(points[25].error_m.is_some());
}

#[test]
fn snapshot_about_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().about_open = true;
    harness.run();
    // The dialog must render the injected placeholder, never the live crate
    // version - otherwise every release bump would diff the snapshot. If this
    // regresses to `env!`, the snapshot below diffs and this asserts too.
    assert!(
        harness
            .inner
            .query_by_label_contains(super::TEST_APP_VERSION)
            .is_some(),
        "the dialog must render the injected placeholder version"
    );
    assert!(
        harness
            .inner
            .query_by_label_contains(&format!("GeoTrace {}", env!("CARGO_PKG_VERSION")))
            .is_none(),
        "the live crate version must never reach the rendered dialog"
    );
    harness.snapshot("about_dialog");
}

#[test]
fn snapshot_file_menu_open() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(400.0, 300.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.get_by_label("File").click();
    harness.run();
    harness.snapshot("file_menu_open");
}

/// The File menu is the sole route to the About dialog: opening the menu and
/// clicking the entry must raise it.
#[test]
fn file_menu_opens_about_dialog() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    harness.get_by_label("File").click();
    harness.run_steps(2);
    harness.get_by_label("About GeoTrace").click();
    harness.run_steps(2);

    assert!(harness.state().about_open, "the menu entry must open About");
}

#[test]
fn about_dialog_closes_on_escape() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().about_open = true;
    harness.step();

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(!harness.state().about_open, "Escape must close the dialog");
}

#[test]
fn snapshot_recording_details_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state().shared.borrow_mut().metadata_popup =
        Some(gt_side_panel::RecordingDetails {
            metadata: gt_types::FileMetadata {
                filename: "ride_2025-05-23.gtd".to_owned(),
                title: Some("Morning commute".to_owned()),
                device: Some("uBlox ZED-F9P".to_owned()),
                notes: Some("Rooftop antenna, clear sky.".to_owned()),
                travel_mode: Some(gt_types::TravelMode::Bicycle),
                ..gt_test_utils::empty_file_metadata()
            },
            // A long, auto-derived, path-like identity to show the dialog gives
            // it room instead of clipping it.
            identity: Some("auto:/home/user/recordings/2025/05/ride_2025-05-23.gtd".to_owned()),
        });
    harness.run();
    harness.snapshot("recording_details_dialog");
}

/// A journald-shaped log in the ISO form: its lines carry the year, so what the
/// viewer draws does not depend on the year the test runs in.
fn synthetic_log(approx_bytes: usize) -> String {
    synthetic_journald_log(SyntheticLogSpec {
        approx_bytes,
        seed: 7,
        timestamps: SyntheticLogTimestamps::Iso8601Space,
    })
}

/// A recording running from the moment the generated log starts, so the log
/// finds it as an association candidate when it loads after it.
fn recording_alongside_the_log(name: &str, start_lat_deg: f64) -> TestDroppedFile {
    TestDroppedFile::bytes(
        synthetic_gtd_bytes(SyntheticGtdSpec {
            start: synthetic_log_start(),
            point_count: 600,
            step_secs: 1,
            start_lat_deg,
            start_lon_deg: 12.0,
            lat_step_deg: 0.00005,
            lon_step_deg: 0.00008,
            heading_deg: 20.0,
            speed_kmh: 28.0,
            eph_m: 1.8,
            sats_seen: 14,
            sats_in_fix: 11,
        }),
        name,
    )
}

fn drop_log_and_wait_for_load(harness: &mut Harness<App>, text: &str, name: &str) {
    drop_file_and_wait_for_load(harness, TestDroppedFile::bytes(text.as_bytes(), name));
}

/// Drops a log and confirms the association dialog it raises: the log is left
/// associated with the recording the dialog preselected.
fn drop_log_and_associate_it(harness: &mut Harness<App>, text: &str, name: &str) {
    drop_log_and_wait_for_load(harness, text, name);
    harness.run_steps(3);
    harness
        .get_by_label(log_viewer::association_dialog::CONFIRM_LABEL)
        .click();
    harness.run_steps(3);
}

fn app_with_a_log_loaded() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_log_and_wait_for_load(&mut harness, &synthetic_log(64 * 1024), "navsyncd.log");
    harness.run_steps(3);
    harness
}

impl App {
    fn shown_log(&self) -> Option<&LoadedLog> {
        self.logs.get(self.log_viewer.selected_log_index())
    }
}

fn parse_summary_of_the_shown_log(harness: &Harness<App>) -> String {
    harness
        .state()
        .shown_log()
        .map(|log| log.parse_summary_line())
        .unwrap_or_default()
}

#[test]
fn a_log_that_finished_loading_opens_the_viewer_on_its_parse_summary() {
    let harness = app_with_a_log_loaded();

    assert!(
        harness.state().log_viewer.open,
        "the viewer opens by itself"
    );
    assert_eq!(harness.state().logs.len(), 1);
    let summary = parse_summary_of_the_shown_log(&harness);
    assert!(
        summary.starts_with("ISO 8601 · "),
        "the summary names the detected format, got {summary:?}"
    );
    harness.get_by_label(summary.as_str());
}

/// A drag over the app covers it with the hint the empty viewer shows.
#[test]
fn a_drag_over_the_app_shows_the_hint_naming_every_way_a_log_gets_in() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    assert!(
        harness.query_by_label(log_viewer::LOG_LOAD_HINT).is_none(),
        "nothing is being dragged yet"
    );

    harness
        .input_mut()
        .hovered_files
        .push(egui::HoveredFile::default());
    harness.step();

    harness.get_by_label(log_viewer::LOG_LOAD_HINT);
}

/// A log line whose timestamp the pasted-log name is taken from.
const PASTED_LOG: &str = "2026-01-01 14:02:11 navsyncd: uploaded 2 recordings\n";

/// Pastes `text` as Ctrl+V does, and runs until the load it started has
/// finished.
fn paste_and_wait_for_load(harness: &mut Harness<App>, text: &str) {
    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(text.to_owned()));
    harness.step();
    assert!(
        harness.step_until(|harness| harness.state().loader.loading_jobs.is_empty()),
        "the background load did not finish"
    );
}

/// Ctrl+V with nothing focused loads the clipboard text as a log, named after
/// the first entry it anchored.
#[test]
fn pasting_log_text_loads_it_named_after_its_first_entry() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    paste_and_wait_for_load(&mut harness, PASTED_LOG);
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(
        harness
            .state()
            .logs
            .get(0)
            .map(gt_log_view::LoadedLog::name),
        Some("pasted 14:02:11")
    );
    assert!(harness.state().log_viewer.open);
}

/// A paste while a text field holds focus belongs to that field, and loads
/// nothing.
#[test]
fn pasting_into_a_focused_field_reaches_the_field_and_loads_no_log() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().query_window.open = true;
    harness.run_steps(2);
    focus_query_editor_at_end(&harness, "");
    harness.run_steps(2);

    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(PASTED_LOG.to_owned()));
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    assert!(
        harness.state().query_window.text().contains("navsyncd"),
        "the paste reached the editor, got {:?}",
        harness.state().query_window.text()
    );
}

/// An empty clipboard has no log in it.
#[test]
fn pasting_empty_text_loads_no_log() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();

    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(String::new()));
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 0);
    assert!(harness.state().loader.loading_jobs.is_empty());
}

/// A log carrying a byte that is not UTF-8 loads, and its summary states what
/// reading it as text cost.
#[test]
fn dropping_a_log_that_is_not_utf8_states_the_replacement_in_its_summary() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(
            b"2026-01-01 14:02:11 navsyncd: caf\xe9 open\n".as_slice(),
            "navsyncd.log",
        ),
    );
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    let summary = parse_summary_of_the_shown_log(&harness);
    assert!(
        summary.ends_with("1 byte replaced"),
        "the summary states the lossy decode, got {summary:?}"
    );
    harness.get_by_label(summary.as_str());
}

/// With no recording loaded a log is still fully readable: nothing asks which
/// recording it belongs to, and it puts nothing on the map.
#[test]
fn a_log_loaded_without_a_recording_stays_untargeted_and_raises_no_dialog() {
    let mut harness = app_with_a_log_loaded();

    assert!(harness.state().association_dialog.is_none());
    assert_eq!(
        harness
            .state()
            .logs
            .get(0)
            .and_then(gt_log_view::LoadedLog::association_target),
        None
    );
    assert!(harness.state_mut().logs.map_matches().is_empty());
    harness.get_by_label(parse_summary_of_the_shown_log(&harness).as_str());
}

#[test]
fn the_menu_bar_icon_closes_and_reopens_the_viewer() {
    let mut harness = app_with_a_log_loaded();

    harness.get_by_label(ICON_ARTICLE).click();
    harness.run_steps(2);
    assert!(!harness.state().log_viewer.open);

    harness.get_by_label(ICON_ARTICLE).click();
    harness.run_steps(2);
    assert!(harness.state().log_viewer.open);
}

#[test]
fn clicking_the_parse_summary_unfolds_the_boots_and_the_service_table() {
    let mut harness = app_with_a_log_loaded();
    assert!(
        harness.query_by_label("Boots").is_none(),
        "the summary panel starts folded away"
    );

    let summary = parse_summary_of_the_shown_log(&harness);
    harness.get_by_label(summary.as_str()).click();
    harness.run_steps(3);

    harness.get_by_label("Boots");
    harness.get_by_label("Service summary");
    // What the fixture's exporter summary block states about the device.
    harness.get_by_label("nav-devkit-mk2");
    harness.get_by_label("hal-powerd");
}

/// The viewer over a journald-shaped log: the selector row with its parse
/// summary, the summary panel unfolded onto the boot timeline and the
/// exporter's service table, the line table, and the footer's association
/// controls.
#[test]
fn snapshot_app_log_viewer() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(
        &mut harness.inner,
        &synthetic_log(64 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    let summary = parse_summary_of_the_shown_log(&harness.inner);
    harness.inner.get_by_label(summary.as_str()).click();
    harness.inner.run_steps(8);

    harness.snapshot_loose("app_log_viewer");
}

/// Types `text` into the log viewer's live filter and runs until the scan it
/// starts has landed. The field is focused by its own id: the app renders text
/// fields of its own behind the window.
fn type_into_log_filter(harness: &mut TestHarness<'_, App>, text: &str) {
    type_into_log_filter_of(&mut harness.inner, text);
}

fn type_into_log_filter_of(harness: &mut Harness<'_, App>, text: &str) {
    focus_the_live_log_filter(harness);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    run_until_the_log_filter_scans_land(harness);
}

fn focus_the_live_log_filter(harness: &mut Harness<'_, App>) {
    harness.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(log_viewer::filters::LIVE_FILTER_FIELD_ID));
    });
    harness.run_steps(2);
}

/// Runs until every scan the shown log's filters started has landed. The scans
/// run on worker threads, and the filter row draws
/// [`log_viewer::filters::PENDING_NOTE`] until they do.
fn run_until_the_log_filter_scans_land(harness: &mut Harness<'_, App>) {
    // The frame that reads the keystroke or the click is the one that starts
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

/// Writes `text` into the live filter and keeps it as a chip.
fn add_log_filter(harness: &mut TestHarness<'_, App>, text: &str) {
    add_log_filter_in(&mut harness.inner, text);
}

fn add_log_filter_in(harness: &mut Harness<'_, App>, text: &str) {
    type_into_log_filter_of(harness, text);
    harness
        .get_by_label(log_viewer::filters::ADD_FILTER_LABEL)
        .click();
    run_until_the_log_filter_scans_land(harness);
}

const PASSES_THE_POOL_HOLDS_A_FILTER_SCAN_QUEUED: u64 = 20;

/// Holds every thread of [`gt_logfile::log_worker_pool`] busy, leaving the
/// filter scans spawned onto it queued until the pool is released.
struct OccupiedLogWorkerPool {
    release: Arc<Barrier>,
}

impl OccupiedLogWorkerPool {
    fn occupy() -> Self {
        let pool = gt_logfile::log_worker_pool().expect("the log worker pool builds");
        let workers = pool.current_num_threads();
        let occupied = Arc::new(Barrier::new(workers + 1));
        let release = Arc::new(Barrier::new(workers + 1));
        for _ in 0..workers {
            let occupied = Arc::clone(&occupied);
            let release = Arc::clone(&release);
            pool.spawn(move || {
                occupied.wait();
                release.wait();
            });
        }
        occupied.wait();
        Self { release }
    }

    /// Releases the pool once `ctx` has run `passes` further passes, holding a
    /// queued scan for the same number of frames on every machine.
    fn release_after_passes(self, ctx: &egui::Context, passes: u64) {
        let ctx = ctx.clone();
        let release_at = ctx.cumulative_pass_nr().saturating_add(passes);
        thread::Builder::new()
            .name("release-log-worker-pool".to_owned())
            .spawn(move || {
                while ctx.cumulative_pass_nr() < release_at {
                    thread::sleep(StdDuration::from_millis(1));
                }
                self.release.wait();
            })
            .expect("the releasing thread spawns");
    }
}

/// The scan a keystroke starts stays pending until the worker pool runs it,
/// and the filter row draws [`log_viewer::filters::PENDING_NOTE`] until it
/// lands.
#[test]
fn the_log_filter_wait_runs_until_the_scan_the_keystroke_started_lands() {
    let mut harness = app_with_a_log_loaded();
    OccupiedLogWorkerPool::occupy()
        .release_after_passes(&harness.ctx, PASSES_THE_POOL_HOLDS_A_FILTER_SCAN_QUEUED);

    focus_the_live_log_filter(&mut harness);
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("kernel".to_owned()));
    harness.run_steps(1);
    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| log.filters().is_query_pending()),
        "the keystroke frame started a scan"
    );
    harness.run_steps(2);
    harness.get_by_label(log_viewer::filters::PENDING_NOTE);

    run_until_the_log_filter_scans_land(&mut harness);

    assert!(
        harness
            .state()
            .shown_log()
            .is_some_and(|log| !log.filters().is_query_pending()),
        "the wait ran until the scan landed"
    );
    assert!(
        harness
            .query_by_label(log_viewer::filters::PENDING_NOTE)
            .is_none(),
        "the note goes with the scan"
    );
}

/// The viewer filtering a journald-shaped log: the live filter with its match
/// count and the term it highlights in the table, a layer chip with its colour
/// swatch and gutter bars, and a refine chip narrowing the table to what it
/// matched.
#[test]
fn snapshot_app_log_viewer_filters() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(
        &mut harness.inner,
        &synthetic_log(64 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    add_log_filter(&mut harness, "kernel");
    add_log_filter(&mut harness, "rotated");
    add_log_filter(&mut harness, "rc=-110");
    // The last chip added is the one furthest right in the chip row.
    harness
        .inner
        .nth_matching(By::new().label(ICON_PLUS_CIRCLE), 2)
        .click();
    run_until_the_log_filter_scans_land(&mut harness.inner);
    type_into_log_filter(&mut harness, "retries");

    harness.snapshot_loose("app_log_viewer_filters");
}

/// The association dialog over a freshly loaded log: the loaded recordings
/// ranked by how much of the log each ran alongside, the one that missed it
/// grayed, and the attach tickbox live for a recording the history database
/// holds.
#[test]
fn snapshot_log_association_dialog() {
    let dir = tempfile::tempdir().expect("temp dir");
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().history = crate::app::history_db::HistoryWorker::spawn(
        open_temporary_history_database(&dir.path().join("geotrace.h5")),
        egui::Context::default(),
        gt_pending_writes::PendingWrites::default(),
    );
    harness.inner.state_mut().sync_db_path();
    harness.inner.state_mut().history.hide_path();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_a_day_after_the_log("drive.gtd"),
    );
    drop_log_and_wait_for_load(
        &mut harness.inner,
        &synthetic_log(64 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_label(log_viewer::association_dialog::ATTACH_LABEL)
        .click();
    harness.inner.run_steps(5);

    harness.snapshot_loose("log_association_dialog");
}

/// A recordings database of this run's own, so a test can store a log with a
/// recording without touching the user's history.
fn open_temporary_history_database(path: &std::path::Path) -> gt_store::Recordings {
    use gt_store::HistoryDatabase as _;
    gt_store::Recordings::open_or_create(path).expect("the temporary database opens")
}

/// A recording from a day the log does not cover, which the dialog lists as a
/// choice that would leave every line unassociated.
fn recording_a_day_after_the_log(name: &str) -> TestDroppedFile {
    TestDroppedFile::bytes(
        synthetic_gtd_bytes(SyntheticGtdSpec {
            start: synthetic_log_start() + chrono::Duration::days(1),
            point_count: 300,
            step_secs: 1,
            start_lat_deg: 48.2,
            start_lon_deg: 11.6,
            lat_step_deg: 0.00004,
            lon_step_deg: 0.00009,
            heading_deg: 75.0,
            speed_kmh: 42.0,
            eph_m: 2.2,
            sats_seen: 12,
            sats_in_fix: 9,
        }),
        name,
    )
}

/// The map draws what a filter selected, and stops when the log that owns the
/// filter is hidden.
#[test]
fn a_layer_chip_puts_the_lines_it_matched_on_the_map() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();
    drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(&mut harness.inner, &synthetic_log(8 * 1024), "navsyncd.log");
    harness.inner.run_steps(5);
    assert!(
        harness.inner.state_mut().logs.map_matches().is_empty(),
        "a loaded log draws nothing until a filter selects lines"
    );

    add_log_filter(&mut harness, "gnss");

    let matched = harness.inner.state_mut().logs.map_matches().match_count();
    assert!(matched > 0, "the chip's lines reach the map");

    if let Some(log) = harness.inner.state_mut().logs.get_mut(0) {
        log.set_visible(false);
    }
    harness.inner.run_steps(2);
    assert!(
        harness.inner.state_mut().logs.map_matches().is_empty(),
        "hiding the log takes its layer off the map"
    );
}

/// The map under a filtered log: a layer chip's hexagons along the recording,
/// clustered where the lines are dense, with the live filter's own colour over
/// them. The viewer is closed so the map it draws on is visible.
#[test]
fn snapshot_app_log_map_hexagons() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(build_app);
    harness.inner.step();

    drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    // A log long enough to span the whole recording, so its hexagons run the
    // length of the track the map frames.
    drop_log_and_associate_it(
        &mut harness.inner,
        &synthetic_log(384 * 1024),
        "navsyncd.log",
    );
    harness.inner.run_steps(5);

    add_log_filter(&mut harness, "kernel");
    type_into_log_filter(&mut harness, "bus-off");
    harness.inner.get_by_label(ICON_ARTICLE).click();
    harness.inner.run_steps(5);

    harness.snapshot_loose("app_log_map_hexagons");
}

#[test]
fn choosing_a_target_in_the_footer_associates_the_log_against_it() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_file_and_wait_for_load(
        &mut harness,
        recording_alongside_the_log("walk_a.gtd", 55.0),
    );
    drop_file_and_wait_for_load(
        &mut harness,
        recording_alongside_the_log("walk_b.gtd", 60.0),
    );
    drop_log_and_wait_for_load(&mut harness, &synthetic_log(8 * 1024), "navsyncd.log");
    harness.run_steps(3);
    // The footer is the fallback for a log left untargeted, which cancelling
    // the association dialog is one way to reach.
    harness.get_by_label("Cancel").click();
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .logs
            .get(0)
            .and_then(|log| log.association_target()),
        None,
        "two overlapping recordings leave the choice to the user"
    );

    harness.get_by_label("Associated with");
    harness.get(By::new().value(gt_ui_theme::EM_DASH)).click();
    harness.run_steps(2);
    // The side panel lists the same recording, so take the row the combo
    // popup opened at the bottom of the viewer.
    harness
        .bottommost_matching(By::new().label("walk_b.gtd"))
        .click();
    harness.run_steps(3);

    assert!(
        harness
            .state()
            .logs
            .get(0)
            .is_some_and(|log| log.associated_entry_count() > 0),
        "picking a target associates the log against it right away"
    );
}

/// The dialog every loaded log raises while a recording is open, and what
/// confirming it does.
mod log_association {
    use gt_store::{HistoryDatabase as _, LogAttachmentEntry, Recordings, StoredLogFilterMode};

    use crate::app::history_db::HistoryWorker;
    use crate::app::log_viewer::association_dialog;

    use super::*;

    /// An app whose history worker owns a database of its own, so a log can be
    /// stored with a recording and read back.
    fn app_over_a_history_database(db_path: &std::path::Path) -> Harness<'static, App> {
        let mut harness = Harness::builder()
            .with_wait_for_pending_images(false)
            .build_eframe(transient_app);
        harness.step();
        harness.state_mut().history = HistoryWorker::spawn(
            open_temporary_history_database(db_path),
            egui::Context::default(),
            gt_pending_writes::PendingWrites::default(),
        );
        harness.state_mut().sync_db_path();
        harness
    }

    fn drop_the_log(harness: &mut Harness<App>) {
        drop_log_and_wait_for_load(harness, &synthetic_log(8 * 1024), "navsyncd.log");
        harness.run_steps(3);
    }

    fn dialog_is_open(harness: &Harness<App>) -> bool {
        harness.state().association_dialog.is_some()
    }

    fn confirm(harness: &mut Harness<App>) {
        harness
            .get_by_label(association_dialog::CONFIRM_LABEL)
            .click();
        harness.run_steps(3);
    }

    fn cancel(harness: &mut Harness<App>) {
        harness.get_by_label("Cancel").click();
        harness.run_steps(3);
    }

    fn shown_log_target(harness: &Harness<App>) -> Option<gt_loaded_files::LoadedFileId> {
        harness
            .state()
            .logs
            .get(0)
            .and_then(gt_log_view::LoadedLog::association_target)
    }

    /// Every attachment the recording carries, as the database holds it.
    fn stored_attachments(
        db_path: &std::path::Path,
        db_ref: &gt_store::DatabaseRef,
    ) -> Vec<LogAttachmentEntry> {
        Recordings::open_or_create(db_path)
            .ok()
            .and_then(|db| db.log_attachments(db_ref).ok())
            .unwrap_or_default()
    }

    /// An app over a database of its own, holding the fixture recording and
    /// the fixture log with its association dialog open. Returns the recording
    /// the database stored.
    fn harness_over_a_recording_and_its_log(
        db_path: &std::path::Path,
    ) -> (Harness<'static, App>, gt_store::DatabaseRef) {
        let mut harness = app_over_a_history_database(db_path);
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));
        let db_ref = stored_recording(&harness);
        drop_the_log(&mut harness);
        (harness, db_ref)
    }

    /// Ticks the dialog's attach box and confirms it, leaving the log stored
    /// with the recording it was associated against.
    fn attach_the_log(
        harness: &mut Harness<App>,
        db_path: &std::path::Path,
        db_ref: &gt_store::DatabaseRef,
    ) {
        harness
            .get_by_label(association_dialog::ATTACH_LABEL)
            .click();
        harness.run_steps(2);
        confirm(harness);
        assert!(
            harness.step_until(|_| !stored_attachments(db_path, db_ref).is_empty()),
            "the worker stored the log with the recording"
        );
        assert!(
            harness.step_until(|harness| harness
                .state()
                .logs
                .get(0)
                .is_some_and(|log| log.attachment().is_some())),
            "the viewer noted the attachment the worker stored"
        );
    }

    /// The recording the app stored when the fixture recording was dropped.
    fn stored_recording(harness: &Harness<App>) -> gt_store::DatabaseRef {
        let state = harness.state();
        let shared = state.shared.borrow();
        let entry = shared
            .loaded_files
            .view()
            .get(0)
            .expect("the recording is loaded");
        entry
            .history()
            .db_ref()
            .cloned()
            .expect("the dropped recording was stored in history")
    }

    /// The one recording the log overlaps is preselected, and confirming takes
    /// the log's positions from it.
    #[test]
    fn confirming_the_dialog_associates_the_log_with_the_preselected_recording() {
        let mut harness = Harness::builder()
            .with_wait_for_pending_images(false)
            .build_eframe(transient_app);
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));
        drop_the_log(&mut harness);

        assert!(dialog_is_open(&harness), "a loaded log raises the dialog");
        harness.get_by_label(association_dialog::TITLE);
        assert_eq!(
            shown_log_target(&harness),
            None,
            "the log takes no position until the choice is made"
        );

        confirm(&mut harness);

        assert!(!dialog_is_open(&harness));
        assert!(
            harness
                .state()
                .logs
                .get(0)
                .is_some_and(|log| log.associated_entry_count() > 0),
            "the preselected recording is what the log associates against"
        );
    }

    /// Several overlapping recordings leave the choice to the user: confirming
    /// without making one leaves the log untargeted.
    #[test]
    fn several_overlapping_recordings_leave_the_dialog_without_a_preselection() {
        let mut harness = Harness::builder()
            .with_wait_for_pending_images(false)
            .build_eframe(transient_app);
        drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk_a.gtd", 55.0),
        );
        drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk_b.gtd", 60.0),
        );
        drop_the_log(&mut harness);
        assert!(dialog_is_open(&harness));

        confirm(&mut harness);

        assert_eq!(shown_log_target(&harness), None);
        assert_eq!(
            harness
                .state()
                .logs
                .get(0)
                .map(gt_log_view::LoadedLog::associated_entry_count),
            Some(0)
        );
    }

    /// Cancelling loads the log as text: no target, and the viewer's footer
    /// left as the way to pick one.
    #[test]
    fn cancelling_the_dialog_loads_the_log_untargeted() {
        let mut harness = Harness::builder()
            .with_wait_for_pending_images(false)
            .build_eframe(transient_app);
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));
        drop_the_log(&mut harness);

        cancel(&mut harness);

        assert!(!dialog_is_open(&harness));
        assert_eq!(shown_log_target(&harness), None);
        assert_eq!(
            harness.state().logs.len(),
            1,
            "the log is loaded either way"
        );
        assert!(harness.state().log_viewer.open);
    }

    /// Escape belongs to the dialog while it is open: the viewer it stands over
    /// stays open.
    #[test]
    fn escape_cancels_the_dialog_and_leaves_the_viewer_open() {
        let mut harness = Harness::builder()
            .with_wait_for_pending_images(false)
            .build_eframe(transient_app);
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));
        drop_the_log(&mut harness);
        assert!(dialog_is_open(&harness));

        press_escape(&mut harness);
        harness.run_steps(3);

        assert!(!dialog_is_open(&harness));
        assert!(harness.state().log_viewer.open, "the viewer stays open");
        assert_eq!(shown_log_target(&harness), None);

        press_escape(&mut harness);
        harness.run_steps(3);

        assert!(
            !harness.state().log_viewer.open,
            "with the dialog gone, Escape closes the viewer"
        );
    }

    /// A stored log that is not the log the attribute names it as: the same
    /// warning path as one that went missing.
    #[test]
    fn an_attachment_whose_stored_log_changed_is_reported_in_the_viewer() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);

        // The attribute now names a log the stored file is not.
        let stored = stored_attachments(&db_path, &db_ref);
        let entry = stored.first().expect("the attachment was stored");
        let mut db = open_temporary_history_database(&db_path);
        db.write_log_attachment_attribute(
            &db_ref,
            entry.id,
            &gt_store::LogAttachment::new(
                entry.attachment.name.clone(),
                gt_store::LogContentHash::of_log_bytes(b"a different log"),
                Vec::new(),
            ),
        )
        .expect("the attribute is writable");
        drop(db);

        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));

        assert!(
            harness.step_until(|harness| harness
                .query_by_label_contains("is not the log it was stored as")
                .is_some()),
            "the viewer says the stored log is not the one that was attached"
        );
        assert_eq!(
            harness.state().logs.len(),
            1,
            "nothing was restored from the attachment"
        );
    }

    /// Once the dialog is switched off, only the unambiguous case associates by
    /// itself. The settings page switches it back on.
    #[test]
    fn dont_show_this_again_leaves_the_unambiguous_case_to_associate_by_itself() {
        let mut harness = Harness::builder()
            .with_wait_for_pending_images(false)
            .build_eframe(transient_app);
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));
        drop_the_log(&mut harness);

        harness
            .get_by_label(association_dialog::DONT_SHOW_AGAIN_LABEL)
            .click();
        harness.run_steps(2);
        cancel(&mut harness);
        assert!(!harness.state().ask_log_association_target);

        drop_the_log(&mut harness);

        assert!(!dialog_is_open(&harness), "the dialog stays away");
        assert!(
            harness
                .state()
                .logs
                .get(1)
                .is_some_and(|log| log.associated_entry_count() > 0),
            "the one overlapping recording is taken without asking"
        );

        harness.state_mut().settings_open = true;
        harness.state_mut().settings_page = settings_ui::SettingsPage::Processing;
        harness.run_steps(2);
        click_settings_row_tickbox(
            &mut harness,
            settings_ui::processing::ASK_LOG_ASSOCIATION_TARGET_LABEL,
        );
        harness.run_steps(2);
        harness.state_mut().settings_open = false;
        harness.run_steps(2);
        assert!(harness.state().ask_log_association_target);

        drop_the_log(&mut harness);

        assert!(
            dialog_is_open(&harness),
            "the setting brings the dialog back"
        );
    }

    /// The whole attachment path over a real database: the dialog stores the
    /// log with the recording, chip edits follow it, and opening that recording
    /// again brings the log back with its filter stack.
    #[test]
    fn an_attached_log_comes_back_with_its_filters_when_the_recording_opens_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);

        attach_the_log(&mut harness, &db_path, &db_ref);

        let attachments = stored_attachments(&db_path, &db_ref);
        assert_eq!(
            attachments
                .iter()
                .map(|entry| entry.attachment.name.as_str())
                .collect::<Vec<_>>(),
            ["navsyncd.log"]
        );

        // A chip added after the attachment was stored is written to it.
        add_log_filter_in(&mut harness, "kernel");
        assert!(
            harness.step_until(|_| {
                !stored_attachments(&db_path, &db_ref)
                    .first()
                    .is_none_or(|entry| entry.attachment.filters.is_empty())
            }),
            "the chip reached the stored attachment"
        );

        // Opening the recording again restores the log it carries.
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));
        assert!(
            harness.step_until(|harness| harness.state().logs.len() == 2),
            "the attached log came back with the recording"
        );
        let restored = harness
            .state()
            .logs
            .get(1)
            .map(|log| {
                log.filters()
                    .chips()
                    .iter()
                    .map(|chip| (chip.pattern().text.clone(), chip.mode()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(
            restored,
            [("kernel".to_owned(), gt_log_view::FilterChipMode::Layer)],
            "the restored log carries the stack it was stored with"
        );
        assert!(
            harness
                .state()
                .logs
                .get(1)
                .is_some_and(|log| log.association_target().is_some()),
            "a restored log is associated with the recording that carried it"
        );
        assert!(harness.state().log_viewer.open);
    }

    /// The stored log is gone from the store, and the recording still opens.
    #[test]
    fn an_attachment_whose_log_file_is_gone_is_reported_in_the_viewer() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);

        for entry in std::fs::read_dir(dir.path().join(gt_store::LOGS_DIRECTORY))
            .expect("the logs directory exists")
            .flatten()
        {
            std::fs::remove_file(entry.path()).expect("the stored log is removable");
        }
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));

        assert!(
            harness.step_until(|harness| harness
                .query_by_label_contains("attachment missing")
                .is_some()),
            "the viewer says the attachment did not come back"
        );
        assert_eq!(
            harness.state().shared.borrow().loaded_files.len(),
            2,
            "the recording loads either way"
        );
    }

    /// Removing the attachment takes it out of the database and leaves the log
    /// loaded.
    #[test]
    fn removing_an_attachment_leaves_the_log_loaded() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);

        harness.get_by_label(log_viewer::DETACH_LABEL).click();
        harness.run_steps(3);

        assert!(
            harness.step_until(|_| stored_attachments(&db_path, &db_ref).is_empty()),
            "the database no longer holds the log"
        );
        assert!(
            harness.step_until(|harness| harness
                .state()
                .logs
                .get(0)
                .is_some_and(|log| log.attachment().is_none())),
            "the viewer noted the attachment the worker removed"
        );
        assert_eq!(harness.state().logs.len(), 1, "the session copy stays");
    }

    /// The recording's database entry is gone by the time the attach runs: the
    /// failure is reported and the loaded log is untouched.
    #[test]
    fn attaching_to_a_recording_deleted_mid_session_reports_the_failure() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let mut harness = app_over_a_history_database(&db_path);
        drop_file_and_wait_for_load(&mut harness, recording_alongside_the_log("walk.gtd", 55.0));
        let db_ref = stored_recording(&harness);
        drop_the_log(&mut harness);

        let mut db = Recordings::open_or_create(&db_path).expect("the database opens");
        db.delete_batch(std::slice::from_ref(&db_ref))
            .expect("the recording is deletable");
        drop(db);

        harness
            .get_by_label(association_dialog::ATTACH_LABEL)
            .click();
        harness.run_steps(2);
        confirm(&mut harness);

        assert!(
            harness.step_until(|harness| harness
                .query_by_label_contains("Could not attach navsyncd.log")
                .is_some()),
            "the viewer reports what the database refused"
        );
        assert_eq!(harness.state().logs.len(), 1);
        assert!(
            harness
                .state()
                .logs
                .get(0)
                .is_some_and(|log| log.attachment().is_none()),
            "nothing about the loaded log changed"
        );
    }

    /// A log attached twice to the same recording is a duplicate the dialog
    /// warns about before it happens.
    #[test]
    fn a_recording_that_already_holds_the_log_warns_in_the_dialog() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);

        harness.get_by_label(log_viewer::ATTACH_LABEL).click();
        harness.run_steps(3);

        assert!(
            harness.step_until(|harness| harness
                .query_by_label_contains("already holds this log")
                .is_some()),
            "the dialog warns before the same log is attached twice"
        );
    }

    /// The stored stack is the chips, in the modes and colours they were in.
    #[test]
    fn the_stored_stack_holds_every_chips_mode_and_colour() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);

        add_log_filter_in(&mut harness, "kernel");
        add_log_filter_in(&mut harness, "rotated");
        assert!(
            harness.step_until(|_| stored_attachments(&db_path, &db_ref)
                .first()
                .is_some_and(|entry| entry.attachment.filters.len() == 2)),
            "both chips reached the stored attachment"
        );

        let stored = stored_attachments(&db_path, &db_ref);
        let filters = stored
            .first()
            .map(|entry| entry.attachment.filters.clone())
            .unwrap_or_default();
        assert_eq!(
            filters
                .iter()
                .map(|filter| (filter.text.as_str(), filter.enabled, filter.mode))
                .collect::<Vec<_>>(),
            [
                ("kernel", true, StoredLogFilterMode::Layer { color_slot: 0 }),
                (
                    "rotated",
                    true,
                    StoredLogFilterMode::Layer { color_slot: 1 }
                ),
            ]
        );
    }
}
