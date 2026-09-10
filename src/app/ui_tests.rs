use egui::TextEdit;
use egui_phosphor::regular::ARROW_LINE_UP_LEFT as ICON_ARROW_LINE_UP_LEFT;
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use egui_phosphor::regular::ARTICLE as ICON_ARTICLE;
use egui_phosphor::regular::DOTS_SIX as ICON_DOTS_SIX;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use std::collections::BTreeSet;
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::{
    sync::{Arc, Barrier},
    time::{Duration as StdDuration, Instant},
};

use egui_kittest::{Harness, Node, kittest::NodeT as _, kittest::Queryable as _};
use geotrace_sdk::{Channel, DateTime, Duration, Unit, Utc};
use gt_instance_lock::{DataDirectoryLock, DataDirectoryOwnership};
use gt_log_view::LoadedLog;
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_store::{
    FlareStore, HistoryDatabase as _, IonexStore, JamStore, ReadOnlyHistoryDatabase as _,
    Recordings, RecordingsHandle, SolarStore,
};
use gt_test_utils::{
    By, ControlLabel, DEMO_BYTES, GOLD_BYTES, HarnessInteraction as _, SyntheticGtdSpec,
    SyntheticLogSpec, SyntheticLogTimestamps, TestHarness, WindowFitAssertions as _,
    synthetic_gtd_bytes, synthetic_journald_log, synthetic_log_start,
};
use gt_types::{FileIdx, LoadWarning, TrackIdx, TrackRef};
use rstest::rstest;
use strum::IntoEnumIterator as _;

use super::App;
use super::archive_recovery::UnavailableArchives;
use super::frame::{LOADING_OVERLAY_MOST_LISTED_JOBS, LOADING_OVERLAY_WINDOW_ID};
use super::history_open::{
    AUTO_PRUNE_RECORDINGS_MOST_LINES, AUTO_PRUNE_TITLE, CLEAR_LOCK_BUTTON_LABEL,
    HISTORY_DATABASE_CORRUPTED_TITLE, HISTORY_DATABASE_IN_USE_TITLE, HISTORY_DATABASE_LOCKED_TITLE,
    TRACK_SETTINGS_DIFFER_TITLE,
};
use super::instance_wait::TakenOverInstance;
use super::log_viewer;
use super::modals::{DELETE_PERMANENTLY_BUTTON_LABEL, SHELVE_BUTTON_LABEL};
use super::query;
use super::settings_ui::{self, SettingsPage};
use super::storage::OpenStorage;
use super::storage_controls::AUTO_STORE_LABEL;
use crate::app::test_util;
use crate::app::test_util::harness::{TEST_APP_VERSION, TestDroppedFile};

mod archive_recovery;
mod instance_wait;
mod query_editor;
mod query_results;
mod recording_from_disk;
mod settings_window;
mod shutdown;
mod snap;
mod storage;

/// Fails listing every captured tile the map requested while drawing the frame
/// about to be snapshotted and did not get, so no fixture-backed snapshot is
/// recorded over blank ground. The record starts fresh and one more frame is
/// drawn, so the check covers only that frame.
fn assert_the_capture_covers_the_map(harness: &mut TestHarness<'_, App>, snapshot_name: &str) {
    harness
        .inner
        .state_mut()
        .map
        .forget_missing_captured_tiles();
    harness.inner.step();
    let missing = harness
        .inner
        .state()
        .map
        .missing_captured_tiles()
        .expect("the map draws the captured tiles");
    gt_test_utils::assert_map_tile_capture_is_complete(snapshot_name, missing);
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

/// The second node labelled `label`, in render order. The side panel renders
/// first and its Visible section lists every recording drawn on the map: the
/// first node is the panel's, the second the surface under test.
fn node_outside_the_side_panel<'h>(harness: &'h Harness<'_, App>, label: &'h str) -> Node<'h> {
    harness.nth_matching(By::new().label(label), 1)
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
        test_util::harness::drop_file_and_wait_for_load(
            harness,
            TestDroppedFile::bytes(bytes, name),
        );
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(&mut harness, TestDroppedFile::path(tmp_path));

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

#[test]
fn drag_drop_gtd_bytes_loads_file() {
    let gtd_bytes = minimal_gtd_bytes();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "test.gtd"),
    );

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

/// With Sync to map off, no frame scans the loaded fixes for a range: the plot
/// takes a range from the map viewport only while the toggle is on.
#[rstest]
#[case::synced_to_the_map(true)]
#[case::not_synced_to_the_map(false)]
fn the_plot_takes_a_range_from_the_map_only_while_sync_to_map_is_on(#[case] sync_to_map: bool) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.state().shared.borrow_mut().plot_state.sync_to_map = sync_to_map;
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(minimal_gtd_bytes(), "test.gtd"),
    );
    harness.run_steps(3);

    let scanned = harness
        .state()
        .shared
        .borrow()
        .map_synced_plot_range
        .scanned_range();
    assert_eq!(scanned.is_some(), sync_to_map, "scanned range {scanned:?}");
}

/// A demo-trip query with more matches than the matches table lists at once,
/// each of them holding rows for the points table below it.
const MANY_MATCH_QUERY: &str = "points | where accel < -0.2 m/s2";

/// The query window's title, which both its area and its accesskit node are
/// addressed by.
const QUERY_WINDOW_TITLE: &str = "Query";

/// The query window's button moving the matches list into a window of its own.
/// The side panel offers the same icon, so this looks only in the query window.
fn pop_out_button<'h>(harness: &'h Harness<'_, App>) -> egui_kittest::Node<'h> {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, QUERY_WINDOW_TITLE)
        .get_by_label(ICON_ARROW_SQUARE_OUT)
}

/// Anything that is not a recording goes to the log parser, so binary junk
/// fails as a log: nothing in it contains a timestamp.
#[test]
fn drag_drop_binary_junk_reports_it_is_not_a_recognised_log() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
        node_outside_the_side_panel(&harness, name);
    }
    assert!(
        harness.query_by_label("overlap_a.gtd").is_none(),
        "the legend must not fall back to the raw filename"
    );
}

/// The shelve confirmation labels the tracks it is about to shelve the same
/// way every other surface does.
#[test]
fn shelve_confirmation_follows_the_recording_name_template() {
    let mut harness = harness_with_three_files_loaded();
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.recording_name_template = "Rec: {filename}".to_owned();
        shared.tree.shelve_confirm = Some(gt_side_panel::ShelveConfirmState {
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

/// Only the snap can dock the legend from this release point: it is 21 points
/// from the dock, inside the snap radius and past the tolerance
/// `gt_plot::legend_is_docked` allows.
#[test]
fn a_legend_drag_released_short_of_the_dock_snaps_to_it() {
    let mut harness = harness_with_three_files_loaded();
    detach_legend(&mut harness, egui::vec2(220.0, 120.0));

    let start = harness.get_by_label(ICON_DOTS_SIX).rect().center();
    harness.press_drag_release(start, egui::vec2(-195.0, -95.0), 1);

    let offset = harness
        .state()
        .shared
        .borrow()
        .plot_state
        .file_legend_offset;
    assert!(
        gt_plot::legend_is_docked(offset),
        "expected the legend released near the dock to snap to {:?}, got ({:.2},{:.2})",
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
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
/// for a recording the archives place a disturbance in, stating the recording
/// and what each metric reached over it.
#[test]
fn snapshot_space_weather_warning_toast() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    harness.inner.run_steps(60);

    let recorded = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .files()
        .first()
        .and_then(|file| file.metadata.time_range)
        .map(|range| range.start.date_naive())
        .expect("the gold recording is loaded");
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    // Every period of the archived day is at storm level, so the recording is
    // disturbed wherever in the day its first track falls.
    test_util::day_archive::archive_kp_day(&store, recorded, 5.0);
    let ctx = harness.inner.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(store),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    // Two frames put the toast the archived day raises past its slide-in
    // without reaching its expiry.
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
        .eframe(test_util::harness::build_app);
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );
    harness.inner.ctx.set_theme(egui::ThemePreference::Light);
    harness.inner.run_steps(60);

    harness.snapshot_with_tolerance("app_with_file_loaded_light", 2.5, 4);
}

/// Snapshot of the point window pinned on a fix the receiver wrote a latitude
/// of 91° for: the window marks the recorded value and states the position the
/// map draws the fix at.
#[test]
fn snapshot_app_point_window_coordinate_out_of_range() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    let out_of_range = gt_types::PointIdx::new(2);
    let points = gt_test_utils::fixtures::nav_points_with_a_latitude_out_of_range(6, out_of_range);
    let fi = push_points_as(
        &mut harness.inner,
        "out_of_range.gtd",
        &points,
        None,
        gt_loaded_files::FileHistory::None,
    );

    {
        let mut shared = harness.inner.state().shared.borrow_mut();
        shared.highlight.toggle_sticky(gt_ui_types::DataPointRef {
            track: TrackRef::new(fi, TrackIdx::new(0)),
            category: gt_types::DataCategory::Tpv,
            point_index: out_of_range,
        });
        shared.zoom_to_visible_request = true;
    }
    harness.inner.run_steps(30);

    harness.snapshot_loose("app_point_window_coordinate_out_of_range");
}

/// Snapshot of the app zoomed into the cluster of Sahara desert tracks from
/// the gold dataset. All other tracks (antimeridian, southern hemisphere, etc.)
/// are hidden so only the closely-spaced Sahara tracks fill the map.
#[test]
fn snapshot_app_sahara_tracks() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(GOLD_BYTES, "gold.gtd"),
    );

    // Identify the Sahara tracks by latitude: they are all centred around
    // 23°N 13°E.
    let sahara_tracks: Vec<TrackRef> = {
        let state = harness.inner.state().shared.borrow();
        state.loaded_files[0]
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.geometry
                    .measured()
                    .is_some_and(|geometry| geometry.bounding_box.lat.south().as_degrees() > 20.0)
            })
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

    assert_the_capture_covers_the_map(&mut harness, "app_sahara_tracks");
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
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
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

    assert_the_capture_covers_the_map(&mut harness, "app_demo_trip");
    harness.snapshot_loose("app_demo_trip");
}

/// Channels plot as their own toggleable category: the Channels toggle
/// reveals the accel chip and its component lines, the chip hides them
/// again, and both toggles round-trip. The demo trip carries a 25 Hz accel
/// channel, so the snapshot shows real IMU-shaped lines beneath the metrics.
#[test]
fn snapshot_app_plot_channels() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );

    // Force the light theme (startup settings default to system/dark in tests).
    harness.inner.ctx.set_theme(egui::ThemePreference::Light);

    // Enable a spread of seen/fix series across constellations so the snapshot
    // exercises the light palette's constellation coding and the seen-vs-fix
    // depth separation. (Util/slip families are advanced-gated and covered by
    // the contrast test.)
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

/// The query history survives the settings flush/load roundtrip: a run is
/// captured by `collect_settings_for_flush` and restored by
/// `apply_startup_settings`.
#[test]
fn query_history_persists_across_settings_roundtrip() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
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
    test_util::harness::step_until_query_result(&mut harness);
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

/// Build an app with one loaded file and the query window open. Shared setup
/// for the interactive query-history tests.
fn app_with_a_recording() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(minimal_gtd_bytes(), "test.gtd"),
    );
    harness
}

/// [`app_with_a_recording`] with the query window open over it.
fn app_with_query_window_open() -> Harness<'static, App> {
    let mut harness = app_with_a_recording();
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
    test_util::harness::step_until_query_result(harness);
    harness.run_steps(3);
}

/// A key-press event for `key` with no modifiers.
fn key_press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// The vector channel's per-component hues and the chip's hover legend, on
/// a fixture whose y-scale keeps the three accel lines visibly apart (the
/// demo-trip snapshot squeezes them into one line against velocity's
/// scale). The tooltip maps each component color square to its name.
#[test]
fn snapshot_app_plot_channel_components() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
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
/// the override colour.
#[test]
fn snapshot_app_plot_channel_color_override() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
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

#[test]
fn snapshot_app_three_overlapping_files() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1280.0, 800.0))
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();
    load_three_overlapping_files(&mut harness.inner);
    assert_eq!(harness.inner.state().shared.borrow().loaded_files.len(), 3);

    {
        let mut state = harness.inner.state().shared.borrow_mut();
        state.zoom_to_visible_request = true;
    }
    harness.inner.run_steps(70);
    assert_the_capture_covers_the_map(&mut harness, "app_three_overlapping_files");
    // Full-app map+plot render, so it carries the same GPU/anti-aliasing
    // nondeterminism as the other map snapshots and uses the shared loose
    // tolerance: a tighter hand-picked count drifts on the macOS runner.
    harness.snapshot_loose("app_three_overlapping_files");
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

/// The update prompt as a user installed via the shell/PowerShell installer
/// sees it: a prominent "Update and restart" plus lower-key Later / Skip.
#[cfg(feature = "self-update")]
#[test]
fn snapshot_update_prompt_self_update() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 400.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().update_checker =
        super::update::UpdateChecker::available_for_test("0.2.0", true);
    harness.run();
    harness.snapshot_loose("update_prompt_self_update");
}

/// What a failed install reports in the cases below.
#[cfg(feature = "self-update")]
const UPDATE_INSTALL_FAILURE: &str = "the release asset could not be downloaded";

/// The prompt as it opens, with an update offered and no install started yet.
#[cfg(feature = "self-update")]
fn app_showing_the_update_prompt() -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 400.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().update_checker =
        super::update::UpdateChecker::available_for_test("0.2.0", true);
    harness.inner.run_steps(4);
    harness
}

/// What the prompt shows once the install it started has failed: the reason
/// in the body, and a manual download beside the dismissal.
#[cfg(feature = "self-update")]
#[test]
fn snapshot_update_prompt_install_failed() {
    let mut harness = app_showing_the_update_prompt();
    harness
        .inner
        .state()
        .update_checker
        .report_a_failed_install_for_test(UPDATE_INSTALL_FAILURE);
    harness.inner.run_steps(4);

    harness.snapshot_loose("update_prompt_install_failed");
}

/// The install reports its outcome after the prompt has already opened.
#[cfg(feature = "self-update")]
#[test]
fn the_update_prompt_shows_the_reason_a_failed_install_gives_without_growing() {
    let mut harness = app_showing_the_update_prompt();

    harness
        .inner
        .state()
        .update_checker
        .report_a_failed_install_for_test(UPDATE_INSTALL_FAILURE);
    harness.inner.run_steps(4);

    let reason = harness
        .inner
        .get_by_label_contains(UPDATE_INSTALL_FAILURE)
        .rect();
    let dismissal = harness.inner.get_by_label("Later").rect();
    assert!(
        reason.bottom() <= dismissal.top(),
        "the reason the failed install gave is at {reason:?}, over the action row at \
         {dismissal:?}"
    );
}

/// A non-self-updatable build (Homebrew / MSI / manual download) exposes the
/// available version for the subtle menu-bar badge. A self-updatable build
/// has no badge version and gets the dialog.
#[cfg(feature = "self-update")]
#[test]
fn non_self_update_uses_badge_not_dialog() {
    let badge = super::update::UpdateChecker::available_for_test("0.2.0", false);
    assert_eq!(badge.badge_version().as_deref(), Some("0.2.0"));

    let self_updatable = super::update::UpdateChecker::available_for_test("0.2.0", true);
    assert_eq!(self_updatable.badge_version(), None);
}

#[derive(Clone, Copy)]
struct WindowTitle<'a>(&'a str);

const SETTINGS_WINDOW: WindowTitle<'static> = WindowTitle("Settings");
const HISTORY_WINDOW: WindowTitle<'static> = WindowTitle("History");

/// Runs frames until the window titled `title` shows the control labelled
/// `label`, settles the pointer on it and clicks it, then runs the frames the
/// click's effect needs to reach the app state.
///
/// The wait searches the window's own subtree because both windows draw the
/// same storage controls, and
/// [`gt_test_utils::HarnessInteraction::step_until`] tests its predicate on
/// the accessibility tree of the previous frame. A search of the whole tree
/// also matches the control in the window that the test closed one frame
/// earlier, and ends the wait before the window named here has drawn.
fn click_the_control_once_the_window_shows_it(
    harness: &mut Harness<'_, App>,
    WindowTitle(title): WindowTitle<'_>,
    ControlLabel(label): ControlLabel<'_>,
) {
    assert!(
        harness.step_until(|h| {
            h.query_by_role_and_label(egui::accesskit::Role::Window, title)
                .and_then(|window| window.query_by_label(label))
                .is_some()
        }),
        "the {title} window shows the control labelled {label:?}"
    );
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .get_by_label(label)
        .hover();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Window, title)
        .get_by_label(label)
        .click();
    harness.run_steps(3);
}

/// The storage controls appear in the History window and on the settings
/// window's Application page, both driving the one setting: what one window
/// writes, the other reads.
#[test]
fn storage_controls_drive_one_setting_from_both_windows() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let ctx = harness.ctx.clone();
    harness
        .state_mut()
        .reopen_history_database(&dir.path().join("recordings.h5"), &ctx);

    // The Application page turns auto-pruning on.
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    click_the_control_once_the_window_shows_it(
        &mut harness,
        SETTINGS_WINDOW,
        ControlLabel("Auto-prune when over"),
    );
    assert!(
        harness.state().storage_settings.auto_prune_enabled,
        "the Application page's auto-prune switch writes the setting"
    );

    // Clicking the History window's confirmation toggle proves it reads the
    // Application page's write and writes the same setting back: the toggle
    // only takes a click while auto-pruning is on.
    harness.state_mut().settings_open = false;
    harness.state_mut().history_window.open = true;
    click_the_control_once_the_window_shows_it(
        &mut harness,
        HISTORY_WINDOW,
        ControlLabel("Confirm before pruning"),
    );
    assert!(
        !harness.state().storage_settings.auto_prune_confirm,
        "the History window's confirmation toggle writes the setting"
    );

    // Auto-storing off in the History window empties the loader's database
    // path, the same live effect the Application page has.
    click_the_control_once_the_window_shows_it(
        &mut harness,
        HISTORY_WINDOW,
        ControlLabel(AUTO_STORE_LABEL),
    );
    assert!(
        !harness.state().storage_settings.enabled,
        "the History window's auto-store checkbox writes the setting"
    );
    assert_eq!(harness.state().loader.db_path, None);

    // The Application page reads the History window's write: its auto-store
    // checkbox turns storing back on, and the loader's path returns.
    harness.state_mut().history_window.open = false;
    harness.state_mut().settings_open = true;
    click_the_control_once_the_window_shows_it(
        &mut harness,
        SETTINGS_WINDOW,
        ControlLabel(AUTO_STORE_LABEL),
    );
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(crate::app::storage::HistoryFailure::Locked(
        PathBuf::from("geotrace.h5"),
    ));
    harness.run();
    harness.snapshot_loose("history_locked_dialog");
}

#[test]
fn snapshot_history_corrupt_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(
        crate::app::storage::HistoryFailure::Unreadable(PathBuf::from("geotrace.h5")),
    );
    harness.run();
    harness.snapshot_loose("history_corrupt_dialog");
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
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    assert!(
        harness.state().history.path().is_none(),
        "the harness starts with storage disabled"
    );

    let opened = crate::app::storage::OpenStorage {
        history: crate::app::history_db::HistoryWorker::spawn(
            RecordingsHandle::Owner(store.open_recordings().expect("recordings")),
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
        .build_eframe(test_util::harness::transient_app);
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

/// [`app_with_the_databases_still_opening`] for a session with `write_access`,
/// which controls whether anything it loads is stored.
fn app_with_the_databases_still_opening_for<'a>(
    paths: &[PathBuf],
    write_access: WriteAccess,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    let paths = paths.to_vec();
    app_with_the_databases_still_opening_built_by(move |cc| {
        test_util::harness::transient_app_with_the_instance_lock(
            cc,
            &paths,
            DataDirectoryLock::marking_nothing(),
            PendingWrites::new(write_access),
        )
    })
}

/// [`app_with_the_databases_still_opening`] for a session that reads and
/// writes the settings file at `config_path`.
fn app_with_the_databases_still_opening_reading<'a>(
    config_path: PathBuf,
    write_access: WriteAccess,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>) {
    app_with_the_databases_still_opening_built_by(move |cc| {
        test_util::harness::transient_app_with_the_settings_file(
            cc,
            &[],
            Some(config_path.clone()),
            DataDirectoryLock::marking_nothing(),
            PendingWrites::new(write_access),
        )
    })
}

/// The app `build` returns, holding back the databases it opens: the returned
/// sender lands them, as [`land_the_databases`] does.
fn app_with_the_databases_still_opening_built_by<'a, F>(
    build: F,
) -> (Harness<'a, App>, mpsc::Sender<OpenStorage>)
where
    F: FnOnce(&eframe::CreationContext<'_>) -> App + 'a,
{
    let (sender_tx, sender_rx) = mpsc::channel();
    let harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(|cc| {
            let mut app = build(cc);
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
            RecordingsHandle::Owner(
                store
                    .open_recordings()
                    .expect("open the recordings database"),
            ),
            ctx.clone(),
            pending_writes.clone(),
        ),
        history_failure: None,
        archive: store.open_or_create_archive::<JamStore>().ok(),
        geomagnetic_indices: store.open_or_create_archive::<SolarStore>().ok(),
        tec_maps: store.open_or_create_archive::<IonexStore>().ok(),
        solar_flares: store.open_or_create_archive::<FlareStore>().ok(),
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

/// Lands the databases of a run that opened no recording history, as a run
/// whose database another process holds does.
fn land_the_databases_without_a_recording_history(
    harness: &mut Harness<'_, App>,
    databases: &mpsc::Sender<OpenStorage>,
) {
    let opened = OpenStorage {
        history: crate::app::history_db::HistoryWorker::disabled(),
        history_failure: None,
        archive: None,
        geomagnetic_indices: None,
        tec_maps: None,
        solar_flares: None,
        unavailable_archives: UnavailableArchives::default(),
    };
    databases.send(opened).expect("the app holds the receiver");
    harness.step();
}

/// The app started on `data_directory`, which the caller's own
/// [`DataDirectoryLock`] holds, with the files a command line named.
///
/// The app takes its own lock on that very directory, which is rejected for as
/// long as the caller keeps its lock - the same rejection a second GeoTrace
/// gets from the first.
fn lock_on_a_directory_another_instance_holds(data_directory: &Path) -> DataDirectoryLock {
    let instance_lock = DataDirectoryLock::acquire(Some(data_directory));
    assert_eq!(
        instance_lock.ownership(),
        DataDirectoryOwnership::HeldByAnotherInstance,
        "the app is meant to start out waiting"
    );
    instance_lock
}

fn app_waiting_for_the_data_directory<'a>(
    paths: &[PathBuf],
    data_directory: &Path,
) -> Harness<'a, App> {
    let instance_lock = lock_on_a_directory_another_instance_holds(data_directory);
    let paths = paths.to_vec();
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            test_util::harness::transient_app_with_the_instance_lock(
                cc,
                &paths,
                instance_lock,
                PendingWrites::default(),
            )
        })
}

/// "Try again" on a database that still will not open puts the prompt back.
/// Uses an unreadable file, since holding a real lock needs a second process.
#[test]
fn a_failed_retry_restores_the_prompt() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("recordings.h5");
    std::fs::write(&path, b"not a database").expect("write");

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
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

/// A shutdown that has begun rejects all three recovery paths: each writes to
/// the recordings database. `recreate_history_database` renames the file before
/// it reopens it, and a quit in between would leave the directory without one.
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
fn a_history_database_recovery_is_rejected_once_shutdown_has_begun(
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
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state().pending_writes.begin_shutdown();
    let ctx = harness.ctx.clone();

    recover(harness.state_mut(), &path, &ctx);

    assert!(
        harness.state().history.path().is_none(),
        "the rejected recovery opened the recordings database"
    );
    assert_eq!(
        files_in_the_data_directory(),
        before,
        "the rejected recovery renamed, removed or created a file"
    );
}

/// An app that took write access from another instance, with `failure` set as
/// the recordings database's open would have set it.
fn app_after_a_take_over_with<'a>(
    failure: crate::app::storage::HistoryFailure,
) -> Harness<'a, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().instance_taken_over_from = Some(TakenOverInstance {
        process_id: Some(4210),
    });
    harness.state_mut().history_failure = Some(failure);
    harness.run_steps(2);
    harness
}

/// The busy prompt states the GeoTrace the user took write access from: they
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history_failure = Some(crate::app::storage::HistoryFailure::Busy(
        PathBuf::from("geotrace.h5"),
    ));
    harness.run();
    harness.snapshot_loose("history_busy_dialog");
}

/// The button that dismisses a history database prompt.
const CANCEL_LABEL: &str = "Cancel";

/// A re-segment prompt for the recording named `filename`, stored with a split
/// rule and a placement rule that both differ from the current ones.
fn resegment_prompt_named(filename: &str) -> super::ResegmentPrompt {
    super::ResegmentPrompt {
        db_ref: gt_store::DatabaseRef {
            identity: format!("auto:{filename}"),
            group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
        },
        filename: filename.to_owned(),
        bytes: std::sync::Arc::from(Vec::<u8>::new()),
        stored: gt_store::StoredSegmentation {
            track_split_gap_us: 60_000_000,
            track_split_rule: gt_store::StoredTrackSplitRule::ForwardGapOnly,
            fix_placement_rule: gt_store::StoredFixPlacementRule::MissingHeading,
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 4.0,
        },
        stored_tracks: Vec::new(),
        marker_settings_changed: false,
    }
}

fn app_showing_the_resegment_prompt(filename: &str) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_resegment = Some(resegment_prompt_named(filename));
    harness.run();
    harness
}

/// The marks the user set by hand on the recording of a re-segment prompt.
#[derive(Clone, Copy)]
struct MarksOnThePromptedRecording {
    shelved_tracks: usize,
    hidden_tracks: usize,
}

/// Nav points per track of the stored track table the prompt is built with.
const STORED_TRACK_NAV_POINTS: u64 = 100;

/// A stored track table of `shelved_tracks` shelved tracks and one live one.
fn stored_track_table_with_shelved_tracks(shelved_tracks: usize) -> Vec<gt_store::TrackRange> {
    let mut table = Vec::new();
    let mut start = 0;
    for state in std::iter::repeat_n(gt_store::TrackState::Shelved, shelved_tracks)
        .chain(std::iter::once(gt_store::TrackState::Live))
    {
        table.push(gt_store::TrackRange {
            start,
            end: start + STORED_TRACK_NAV_POINTS,
            state,
        });
        start += STORED_TRACK_NAV_POINTS;
    }
    table
}

fn app_showing_the_resegment_prompt_with(
    marks: MarksOnThePromptedRecording,
) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    let mut prompt = resegment_prompt_named("ride.gtd");
    prompt.stored_tracks = stored_track_table_with_shelved_tracks(marks.shelved_tracks);
    harness
        .inner
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .set_hidden_tracks_of_recording(prompt.db_ref.clone(), (1..=marks.hidden_tracks).collect());
    harness.inner.state_mut().pending_resegment = Some(prompt);
    harness.run();
    harness
}

/// What every warning about the marks a recalculation drops opens with.
const RECALCULATION_WARNING_OPENING: &str = "Recalculating ";

#[rstest]
#[case::shelved_tracks(
    MarksOnThePromptedRecording {
        shelved_tracks: 2,
        hidden_tracks: 0,
    },
    Some("Recalculating puts the 2 shelved tracks of this recording back in the working set.")
)]
#[case::hidden_tracks(
    MarksOnThePromptedRecording {
        shelved_tracks: 0,
        hidden_tracks: 1,
    },
    Some("Recalculating shows the 1 hidden track of this recording.")
)]
#[case::shelved_and_hidden_tracks(
    MarksOnThePromptedRecording {
        shelved_tracks: 1,
        hidden_tracks: 3,
    },
    Some(
        "Recalculating puts the 1 shelved track of this recording back in the working set and \
         shows its 3 hidden tracks."
    )
)]
#[case::neither(
    MarksOnThePromptedRecording {
        shelved_tracks: 0,
        hidden_tracks: 0,
    },
    None
)]
fn the_resegment_prompt_states_the_marks_a_recalculation_drops(
    #[case] marks: MarksOnThePromptedRecording,
    #[case] expected_warning: Option<&str>,
) {
    let harness = app_showing_the_resegment_prompt_with(marks);
    match expected_warning {
        Some(warning) => {
            harness.inner.get_by_label(warning);
        }
        None => assert!(
            harness
                .inner
                .query_by_label_contains(RECALCULATION_WARNING_OPENING)
                .is_none(),
            "the re-segment prompt warns about marks on a recording that has none"
        ),
    }
}

#[test]
fn snapshot_history_resegment_dialog_with_shelved_and_hidden_tracks() {
    let mut harness = app_showing_the_resegment_prompt_with(MarksOnThePromptedRecording {
        shelved_tracks: 1,
        hidden_tracks: 3,
    });
    harness.snapshot_loose("history_resegment_dialog_with_shelved_and_hidden_tracks");
}

/// A recording name of words, long enough for the prompt's intro to run past
/// the room it caps at.
fn recording_name_past_the_capped_room() -> String {
    format!("{}.gtd", ["ride"; 60].join(" "))
}

#[test]
fn snapshot_history_resegment_dialog() {
    let mut harness = app_showing_the_resegment_prompt("ride.gtd");
    harness.snapshot_loose("history_resegment_dialog");
}

#[test]
fn snapshot_history_resegment_dialog_past_the_capped_room() {
    let mut harness = app_showing_the_resegment_prompt(&recording_name_past_the_capped_room());
    harness.snapshot_loose("history_resegment_dialog_past_the_capped_room");
}

/// Opening a second recording from history replaces the prompt with one
/// stating that recording.
#[test]
fn the_resegment_prompt_keeps_its_buttons_in_place_while_a_longer_name_arrives() {
    let mut harness = app_showing_the_resegment_prompt("ride.gtd");
    let before = harness.inner.get_by_label(CANCEL_LABEL).rect();

    harness.inner.state_mut().pending_resegment = Some(resegment_prompt_named(
        &recording_name_past_the_capped_room(),
    ));
    harness.inner.run_steps(4);

    assert_eq!(
        harness.inner.get_by_label(CANCEL_LABEL).rect(),
        before,
        "the Cancel button of the re-segment prompt moved: a press where the user aimed misses it"
    );
}

/// The recordings a prune under the storage limit deletes, each named by its
/// identity and the group it is stored under.
fn auto_prune_candidates(count: usize) -> Vec<gt_store::DatabaseRef> {
    (0..count)
        .map(|index| gt_store::DatabaseRef {
            identity: format!("auto:ride-{index}.gtd"),
            group_name: format!("2025-05-2{index}T10:00:00Z_a1b2"),
        })
        .collect()
}

fn app_showing_the_auto_prune_confirmation(count: usize) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_auto_prune = Some(auto_prune_candidates(count));
    harness.run();
    harness
}

/// Candidates enough to fill the room the list caps at
/// [`AUTO_PRUNE_RECORDINGS_MOST_LINES`].
const AUTO_PRUNE_CANDIDATES_PAST_THE_CAPPED_ROOM: usize = 12;

const AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM: usize = 40;

#[test]
fn snapshot_auto_prune_dialog() {
    let mut harness = app_showing_the_auto_prune_confirmation(3);
    harness.snapshot_loose("auto_prune_dialog");
}

#[test]
fn snapshot_auto_prune_dialog_past_the_capped_room() {
    let mut harness =
        app_showing_the_auto_prune_confirmation(AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM);
    harness.snapshot_loose("auto_prune_dialog_past_the_capped_room");
}

#[test]
fn the_auto_prune_confirmation_opens_at_one_height_for_every_list_past_the_capped_room() {
    let past = app_showing_the_auto_prune_confirmation(AUTO_PRUNE_CANDIDATES_PAST_THE_CAPPED_ROOM);
    let far_past =
        app_showing_the_auto_prune_confirmation(AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM);

    assert_eq!(
        far_past
            .inner
            .window_rect(AUTO_PRUNE_TITLE)
            .expect("the auto-prune confirmation is shown")
            .size(),
        past.inner
            .window_rect(AUTO_PRUNE_TITLE)
            .expect("the auto-prune confirmation is shown")
            .size(),
        "{AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM} candidates made the auto-prune \
         confirmation taller than {AUTO_PRUNE_CANDIDATES_PAST_THE_CAPPED_ROOM} did: a list past \
         the room it caps at {AUTO_PRUNE_RECORDINGS_MOST_LINES} lines has to scroll inside that \
         room"
    );
}

/// A recording stored while the confirmation is open runs the auto-prune check
/// again, and the candidates it comes back with replace the list.
#[test]
fn the_auto_prune_confirmation_keeps_its_buttons_in_place_while_more_candidates_arrive() {
    let mut harness = app_showing_the_auto_prune_confirmation(3);
    let before = harness.inner.get_by_label(CANCEL_LABEL).rect();

    harness.inner.state_mut().pending_auto_prune = Some(auto_prune_candidates(
        AUTO_PRUNE_CANDIDATES_FAR_PAST_THE_CAPPED_ROOM,
    ));
    harness.inner.run_steps(4);

    assert_eq!(
        harness.inner.get_by_label(CANCEL_LABEL).rect(),
        before,
        "the Cancel button of the auto-prune confirmation moved: a press where the user aimed \
         misses it"
    );
}

/// A recording that segments into two tracks under the default split gap: ten
/// fixes a second apart, an hour of nothing, then ten more.
fn two_track_gtd_bytes() -> Vec<u8> {
    use geotrace_sdk::{Angle, Duration as SdkDuration, NavFileBuilder, NavFix, NavFixTime};

    let mut recorder = NavFileBuilder::new().open();
    for fix in 0..20i64 {
        let seconds = if fix < 10 { fix } else { fix + 3_600 };
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(
                    base_time() + SdkDuration::seconds(seconds),
                ))
                .lat(Angle::degrees(51.5 + 0.0002 * fix as f64))
                .lon(Angle::degrees(-0.1))
                .heading(Angle::degrees(270.0))
                .build(),
        );
    }
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).expect("write bytes");
    bytes
}

/// The stored track table of [`two_track_gtd_bytes`], both tracks live.
fn two_live_track_ranges() -> [gt_store::TrackRange; 2] {
    use gt_store::{TrackRange, TrackState};

    [
        TrackRange {
            start: 0,
            end: 10,
            state: TrackState::Live,
        },
        TrackRange {
            start: 10,
            end: 20,
            state: TrackState::Live,
        },
    ]
}

/// Store `bytes` in the history database under `store`, cut into `tracks` and
/// segmented under `settings`, and return the reference it is stored under.
fn insert_recording_into(
    store: &gt_store::Store,
    bytes: &[u8],
    tracks: &[gt_store::TrackRange],
    settings: gt_store::StoredSegmentation,
) -> gt_store::DatabaseRef {
    let mut db =
        Recordings::open_or_create(&store.recordings_path()).expect("open the history database");
    let meta = gt_store::extract_meta(bytes).expect("the recording has metadata");
    db.insert("dev", &meta, tracks, settings, bytes)
        .expect("store the recording")
}

/// The app before its databases land, reading a settings file that lists the
/// second track of the stored recording `db_ref` as hidden.
struct AppReadingHiddenTracksFromTheSettingsFile {
    harness: Harness<'static, App>,
    dir: tempfile::TempDir,
    store: gt_store::Store,
    db_ref: gt_store::DatabaseRef,
    databases: mpsc::Sender<OpenStorage>,
}

/// One recording of two live tracks in the history database under `store`.
fn seed_a_two_track_recording(store: &gt_store::Store) -> gt_store::DatabaseRef {
    let settings = crate::app::loader::stored_segmentation_from_config(
        &gt_track_builder::SegmentationConfig::default(),
    );
    insert_recording_into(
        store,
        &two_track_gtd_bytes(),
        &two_live_track_ranges(),
        settings,
    )
}

fn app_reading_hidden_tracks_from_the_settings_file(
    write_access: WriteAccess,
) -> AppReadingHiddenTracksFromTheSettingsFile {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let db_ref = seed_a_two_track_recording(&store);

    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        format!(
            "[[ui.hidden_tracks]]\nidentity = {:?}\ngroup_name = {:?}\ntrack_numbers = [2]\n",
            db_ref.identity, db_ref.group_name
        ),
    )
    .expect("write the settings file");

    let (mut harness, databases) =
        app_with_the_databases_still_opening_reading(config_path, write_access);
    harness.step();
    AppReadingHiddenTracksFromTheSettingsFile {
        harness,
        dir,
        store,
        db_ref,
        databases,
    }
}

/// The hidden tracks of `db_ref` as this session holds them, and [`None`]
/// where it has not read that recording's UI state.
fn hidden_track_numbers_of(
    harness: &Harness<'_, App>,
    db_ref: &gt_store::DatabaseRef,
) -> Option<BTreeSet<usize>> {
    let state = harness.state();
    let shared = state.shared.borrow();
    shared.tree.hidden_track_numbers(db_ref).cloned()
}

/// The hidden tracks a settings file written before the history database held
/// them are stored with their recording at startup, and the settings file this
/// version writes lists none.
#[test]
fn the_hidden_tracks_the_settings_file_lists_are_stored_with_their_recording() {
    let AppReadingHiddenTracksFromTheSettingsFile {
        mut harness,
        dir,
        store,
        db_ref,
        databases,
    } = app_reading_hidden_tracks_from_the_settings_file(WriteAccess::Owner);
    land_the_databases(&mut harness, &databases, &store);

    harness.state().history.open(db_ref.clone());

    assert!(
        harness.step_until(
            |harness| hidden_track_numbers_of(harness, &db_ref) == Some(BTreeSet::from([2]))
        ),
        "the recording opened without the hidden track the settings file lists"
    );
    harness.state().flush_settings();
    let written = std::fs::read_to_string(dir.path().join("config.toml"))
        .expect("read the settings file back");
    assert!(
        !written.contains("hidden_tracks"),
        "the settings file still lists hidden tracks: {written}"
    );
}

/// A session that opened no recording history keeps the hidden tracks the
/// settings file lists, and stores them with their recording once the user has
/// a database open again.
#[test]
fn the_hidden_tracks_the_settings_file_lists_wait_for_a_recording_history() {
    let AppReadingHiddenTracksFromTheSettingsFile {
        mut harness,
        dir: _dir,
        store,
        db_ref,
        databases,
    } = app_reading_hidden_tracks_from_the_settings_file(WriteAccess::Owner);
    land_the_databases_without_a_recording_history(&mut harness, &databases);

    harness
        .state_mut()
        .install_history_worker(test_util::recordings::worker_on(&store.recordings_path()));
    harness.state().history.open(db_ref.clone());

    assert!(
        harness.step_until(
            |harness| hidden_track_numbers_of(harness, &db_ref) == Some(BTreeSet::from([2]))
        ),
        "the session that opened no recording history dropped the hidden tracks"
    );
}

/// A read-only session stores none of them: the recording keeps the UI state
/// the database holds, and the settings file keeps its list for a session with
/// write access.
#[test]
fn a_read_only_session_stores_none_of_the_hidden_tracks_the_settings_file_lists() {
    let AppReadingHiddenTracksFromTheSettingsFile {
        mut harness,
        dir,
        store,
        db_ref,
        databases,
    } = app_reading_hidden_tracks_from_the_settings_file(WriteAccess::ReadOnly);
    land_the_databases(&mut harness, &databases, &store);

    harness.state().history.open(db_ref.clone());

    assert!(
        harness.step_until(
            |harness| hidden_track_numbers_of(harness, &db_ref) == Some(BTreeSet::new())
        ),
        "the read-only session stored the hidden tracks the settings file lists"
    );
    harness.state().flush_settings();
    let written = std::fs::read_to_string(dir.path().join("config.toml"))
        .expect("read the settings file back");
    assert!(
        written.contains("hidden_tracks"),
        "the read-only session rewrote the settings file: {written}"
    );
}

/// The version report covers the whole session: two recordings a newer version
/// of GeoTrace stored UI state for raise one message between them.
#[test]
fn ui_state_a_newer_version_stored_raises_one_message_for_two_recordings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) = app_with_the_databases_still_opening(&[]);
    harness.step();
    land_the_databases(&mut harness, &databases, &store);

    assert_eq!(
        harness.state().toasts.len(),
        0,
        "the session opened with a message about its UI state"
    );

    for group_name in ["2026-01-01T00:00:00Z_ride", "2026-01-02T00:00:00Z_ride"] {
        harness.state().history.ui_state_versions().report_too_new(
            &gt_store::DatabaseRef {
                identity: "dev".to_owned(),
                group_name: group_name.to_owned(),
            },
            2,
        );
    }
    harness.run_steps(2);

    assert_eq!(
        harness.state().toasts.len(),
        1,
        "each recording raised a message of its own"
    );

    harness.run_steps(2);
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "a later frame raised the message again"
    );
}

#[test]
fn snapshot_load_warnings_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(test_util::harness::build_app);
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
    harness.snapshot_loose("load_warnings_dialog");
}

#[test]
fn snapshot_snap_consent_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().snap_consent_prompt = true;
    harness.run();
    harness.snapshot_loose("snap_to_road_consent_dialog");
}

/// The service link only shows for the default FOSSGIS host - its terms do
/// not apply to a self-hosted server.
#[test]
fn consent_service_link_gates_on_the_default_host() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(test_util::harness::build_app);
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().snap_settings.auto_snap = Some(false);
    harness.inner.state_mut().snap_consent_prompt = true;
    harness.run();
    harness.snapshot_loose("snap_to_road_consent_dialog_mode_chosen");
}

/// The one-time auto prompt for uploads acknowledged before auto mode
/// existed.
#[test]
fn snapshot_snap_auto_prompt() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(test_util::harness::build_app);
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
    harness.snapshot_loose("snap_to_road_auto_prompt");
}

#[test]
fn snap_consent_agree_persists_the_server_host() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
    // `snap_consent_agree_persists_the_server_host`). The second only fires
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
) -> (tempfile::TempDir, gt_store::InterferenceArchive) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<JamStore>()
        .expect("archive");
    for day in days {
        test_util::day_archive::archive_an_empty_interference_day(&store, *day);
    }
    install_interference_scheduler(harness, &store);
    (dir, store)
}

/// Point the app at `store` for interference, with nothing to fetch from.
fn install_interference_scheduler(
    harness: &mut Harness<'_, App>,
    store: &gt_store::InterferenceArchive,
) {
    let ctx = harness.ctx.clone();
    harness.state_mut().jamming = crate::app::jamming::JammingScheduler::new(
        ctx,
        Some(store.clone()),
        gt_jam::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
}

fn app_with_interference_days<'a>(
    days: &[chrono::NaiveDate],
) -> (
    Harness<'a, App>,
    tempfile::TempDir,
    gt_store::InterferenceArchive,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let (dir, store) = install_interference_archive(&mut harness, days);
    (harness, dir, store)
}

fn enable_environment_auto_prune(harness: &mut Harness<'_, App>, max_age_months: u32) {
    let settings = &mut harness.state_mut().environment_storage_settings;
    settings.auto_prune_enabled = true;
    settings.auto_prune_max_age_months = max_age_months;
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

/// Push a file built with `travel_mode` into the app's loaded files, returning
/// the ref of its single track.
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
    shared.sync_tree_from_loaded_files();
    fi
}

/// A recording whose clock offset holds near −234 ms, with one sample carrying
/// a 1 h 09 m recording gap - the `gnss.h5.gtd` case, where the receiver
/// reported its pre-gap GPS epoch for the first fix after resuming.
fn clock_excursion_gtd_bytes() -> Vec<u8> {
    use geotrace_sdk::{Angle, Duration as SdkDuration, NavFileBuilder, NavFix, NavFixTime};

    let start = base_time();
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..61i64 {
        let gps = start + SdkDuration::seconds(i);
        let ahead_ms = if i == 10 { 4_127_054 } else { 234 };
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Both {
                    gps,
                    sys: gps + SdkDuration::milliseconds(ahead_ms),
                })
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    let track = gt_types::TrackRef::new(gt_types::FileIdx::new(0), gt_types::TrackIdx::new(0));
    test_util::snap::inject_completed_run(&mut harness, track);
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

/// Every affected track the map indicator lists, as its label and its lines.
fn listed_track_warnings(harness: &Harness<'_, App>) -> Vec<(String, Vec<String>)> {
    harness
        .state()
        .space_weather_warning
        .track_warnings()
        .iter()
        .map(|warning| (warning.track_label.clone(), warning.lines.clone()))
        .collect()
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    // The recording spans 22nd to 26th May 2025. The 24th stays unarchived,
    // and the days either side of the recording carry the margins.
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    let recorded = base_time().date_naive();
    for offset in [-1_i64, 0, 2, 3, 4] {
        let day = recorded + chrono::TimeDelta::days(offset);
        test_util::day_archive::archive_kp_day(&store, day, 2.0);
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .space_weather_warning
            .track_warnings()
            .is_empty(),
        "no archived day overlaps the recording yet"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    test_util::day_archive::archive_kp_day(&store, base_time().date_naive(), 2.0);
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
        listed_track_warnings(&harness),
        [(
            "ride.gtd".to_owned(),
            vec!["Geomagnetic storm (≥5): Kp 5, G1".to_owned()]
        )],
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

/// The quiet-time window before a loaded recording arrives after it: the map
/// indicator states the deviation and the load toast is raised once, however
/// many frames follow.
#[test]
fn a_tec_window_archived_after_the_load_warns_on_the_map() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .space_weather_warning
            .track_warnings()
            .is_empty(),
        "no archived day overlaps the recording yet"
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<IonexStore>()
        .expect("archive");
    let recorded = base_time().date_naive();
    test_util::day_archive::archive_tec_day(&store, recorded, 35.0);
    for days_before in 1..=gt_ionex::quiet_time::BACKGROUND_WINDOW_DAYS as i64 {
        test_util::day_archive::archive_tec_day(
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
        listed_track_warnings(&harness),
        [(
            "ride.gtd".to_owned(),
            vec![
                "ΔTEC (> +43%): +75% from the 27-day median, moderate ionospheric storm \
                 (W = 3), 24h"
                    .to_owned(),
                "TEC over track: 35–35 TECU".to_owned(),
            ]
        )],
        "every epoch of the recording's day stands well above the median of the 27 days before it"
    );
    assert_eq!(harness.state().toasts.len(), 1);

    harness.run_steps(2);
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "a later frame does not raise the toast again"
    );

    let indices_dir = tempfile::tempdir().expect("temp dir");
    let indices = gt_store::Store::open_in(indices_dir.path())
        .open_or_create_archive::<SolarStore>()
        .expect("archive");
    test_util::day_archive::archive_kp_day(&indices, recorded - chrono::TimeDelta::days(1), 5.0);
    let ctx = harness.ctx.clone();
    harness.state_mut().geomagnetic_indices = crate::app::solar::GeomagneticIndexScheduler::new(
        ctx,
        Some(indices),
        gt_solar::DEFAULT_BASE_URL.to_owned(),
        gt_fetch::TransportSource::Offline,
        gt_pending_writes::PendingWrites::default(),
    );
    harness.run_steps(2);

    assert_eq!(
        listed_track_warnings(&harness),
        [(
            "ride.gtd".to_owned(),
            vec![
                "ΔTEC (> +43%): +75% from the 27-day median, moderate ionospheric storm \
                 (W = 3), 24h, after a G5 storm 21h before"
                    .to_owned(),
                "TEC over track: 35–35 TECU".to_owned(),
            ]
        )],
        "the day of indices archived after the load qualifies the deviation"
    );
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(gtd_bytes.as_slice(), "ride.gtd"),
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path())
        .open_or_create_archive::<FlareStore>()
        .expect("archive");
    let recorded = base_time().date_naive();
    for &(offset, peaks) in archived {
        test_util::day_archive::archive_flare_day(
            &store,
            recorded + chrono::TimeDelta::days(offset),
            peaks,
        );
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
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

    test_util::snap::inject_completed_run(&mut harness, track);
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
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let track = push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    test_util::snap::inject_completed_run(&mut harness, track);

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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().about_open = true;
    harness.run();
    // The dialog must render the injected placeholder, never the live crate
    // version - otherwise every release bump would diff the snapshot. If this
    // regresses to `env!`, the snapshot below diffs and this asserts too.
    assert!(
        harness
            .inner
            .query_by_label_contains(TEST_APP_VERSION)
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
    harness.snapshot_loose("about_dialog");
}

#[test]
fn snapshot_file_menu_open() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(400.0, 300.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.get_by_label("File").click();
    harness.run();
    harness.snapshot_loose("file_menu_open");
}

/// The File menu is the sole route to the About dialog: opening the menu and
/// clicking the entry must raise it.
#[test]
fn file_menu_opens_about_dialog() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();

    harness.get_by_label("File").click();
    harness.run_steps(2);
    harness.get_by_label("About GeoTrace").click();
    harness.run_steps(2);

    assert!(harness.state().about_open, "the menu entry must open About");
}

/// A line that only states something shows the ordinary cursor: labels do not
/// select anywhere in the app. Text a reader copies out opts back in, and
/// shows the I-beam over it.
#[rstest::rstest]
#[case::tagline("GPS/GNSS navigation data visualizer", egui::CursorIcon::Default)]
#[case::version(TEST_APP_VERSION, egui::CursorIcon::Text)]
fn the_about_dialog_shows_the_text_cursor_only_over_the_version(
    #[case] label: &str,
    #[case] expected: egui::CursorIcon,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().about_open = true;
    harness.run_steps(2);

    let line = harness.get_by_label_contains(label).rect().center();
    harness.hover_at_and_settle(line, 5);

    assert_eq!(
        harness.output().platform_output.cursor_icon,
        expected,
        "hovering {label:?} should request {expected:?}"
    );
}

#[test]
fn about_dialog_closes_on_escape() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    // The recorded time reads well short of the range it covers: this
    // recording idled between its tracks.
    let day = chrono::NaiveDate::from_ymd_opt(2025, 5, 23).unwrap_or_default();
    let morning = day.and_hms_opt(7, 12, 4).unwrap_or_default().and_utc();
    let noon = day.and_hms_opt(11, 48, 30).unwrap_or_default().and_utc();
    harness.inner.state().shared.borrow_mut().metadata_popup =
        Some(gt_side_panel::RecordingDetails {
            metadata: gt_types::FileMetadata {
                filename: "ride_2025-05-23.gtd".to_owned(),
                title: Some("Morning commute".to_owned()),
                device: Some("uBlox ZED-F9P".to_owned()),
                notes: Some("Rooftop antenna, clear sky.".to_owned()),
                travel_mode: Some(gt_types::TravelMode::Bicycle),
                time_range: Some(gt_types::TimeRange::new(morning, noon)),
                total_duration: chrono::TimeDelta::minutes(88),
                ..gt_test_utils::empty_file_metadata()
            },
            // A long, auto-derived, path-like identity to show the dialog gives
            // it room.
            identity: Some("auto:/home/user/recordings/2025/05/ride_2025-05-23.gtd".to_owned()),
        });
    harness.run();
    harness.snapshot_loose("recording_details_dialog");
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
    TestDroppedFile::bytes(recording_bytes_alongside_the_log(start_lat_deg), name)
}

/// The GTD bytes [`recording_alongside_the_log`] drops as a file, for a test
/// that stores the same recording in a database of its own instead.
fn recording_bytes_alongside_the_log(start_lat_deg: f64) -> Vec<u8> {
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
    })
}

fn drop_log_and_wait_for_load(harness: &mut Harness<App>, text: &str, name: &str) {
    test_util::harness::drop_file_and_wait_for_load(
        harness,
        TestDroppedFile::bytes(text.as_bytes(), name),
    );
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
        .build_eframe(test_util::harness::transient_app);
    drop_log_and_wait_for_load(&mut harness, &synthetic_log(64 * 1024), "navsyncd.log");
    harness.run_steps(3);
    harness
}

impl App {
    fn shown_log(&self) -> Option<&LoadedLog> {
        self.log_viewer
            .selected_log()
            .and_then(|id| self.logs.get_by_id(id))
    }

    /// The log that loaded first, which is the only one in every fixture that
    /// loads one.
    fn first_log(&self) -> Option<&LoadedLog> {
        self.logs.iter().next()
    }

    /// The log that loaded last, for the fixtures loading a second one.
    fn last_log(&self) -> Option<&LoadedLog> {
        self.logs.iter().last()
    }

    /// How many lines the loaded logs put on the map, resolved against the
    /// loaded recordings the way the frame does.
    fn log_map_match_count(&mut self) -> usize {
        let shared = self.shared.borrow();
        let names = gt_loaded_files::RecordingNames::resolve(
            shared.loaded_files.view(),
            &shared.recording_name_template,
        );
        self.logs
            .map_matches(shared.loaded_files.view(), &names)
            .match_count()
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

/// One log per content: dropping a text the session already holds opens the
/// loaded log and keeps the one copy.
#[test]
fn dropping_a_log_the_session_already_holds_shows_the_loaded_one() {
    let mut harness = app_with_a_log_loaded();
    let loaded = harness.state().logs.first_id();
    harness.state_mut().log_viewer.open = false;

    drop_log_and_wait_for_load(
        &mut harness,
        &synthetic_log(64 * 1024),
        "copy-of-navsyncd.log",
    );
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(
        harness.state().logs.first_id(),
        loaded,
        "the loaded log kept its identity"
    );
    assert_eq!(
        harness
            .state()
            .shown_log()
            .map(gt_log_view::LoadedLog::name),
        Some("navsyncd.log"),
        "the viewer shows the log the session holds, under the name it loaded with"
    );
    assert!(harness.state().log_viewer.open, "the viewer opens on it");
    assert_eq!(
        harness.state().toasts.len(),
        1,
        "the drop raises a toast naming the loaded log"
    );
}

/// A drag over the app covers it with the hint the empty viewer shows.
#[test]
fn a_drag_over_the_app_shows_the_hint_naming_every_way_a_log_gets_in() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
    harness.step();

    paste_and_wait_for_load(&mut harness, PASTED_LOG);
    harness.run_steps(3);

    assert_eq!(harness.state().logs.len(), 1);
    assert_eq!(
        harness
            .state()
            .first_log()
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
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
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
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

/// With no recording loaded a log is still fully readable: nothing asks the
/// user which recording it belongs to, and it puts nothing on the map.
#[test]
fn a_log_loaded_without_a_recording_stays_untargeted_and_raises_no_dialog() {
    let mut harness = app_with_a_log_loaded();

    assert!(harness.state().association_dialog.is_none());
    assert_eq!(
        harness
            .state()
            .first_log()
            .and_then(gt_log_view::LoadedLog::associated_recording),
        None
    );
    assert_eq!(harness.state_mut().log_map_match_count(), 0);
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().history = crate::app::history_db::HistoryWorker::spawn(
        RecordingsHandle::Owner(open_temporary_history_database(
            &dir.path().join("geotrace.h5"),
        )),
        egui::Context::default(),
        gt_pending_writes::PendingWrites::default(),
    );
    harness.inner.state_mut().sync_db_path();
    harness.inner.state_mut().history.hide_path();

    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    test_util::harness::drop_file_and_wait_for_load(
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
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness.inner,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(&mut harness.inner, &synthetic_log(8 * 1024), "navsyncd.log");
    harness.inner.run_steps(5);
    assert_eq!(
        harness.inner.state_mut().log_map_match_count(),
        0,
        "a loaded log draws nothing until a filter selects lines"
    );

    add_log_filter(&mut harness, "gnss");

    let matched = harness.inner.state_mut().log_map_match_count();
    assert!(matched > 0, "the chip's lines reach the map");

    let loaded = harness.inner.state().logs.first_id();
    if let Some(log) = loaded.and_then(|id| harness.inner.state_mut().logs.get_mut_by_id(id)) {
        log.set_visible(false);
    }
    harness.inner.run_steps(2);
    assert_eq!(
        harness.inner.state_mut().log_map_match_count(),
        0,
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
        .eframe(test_util::harness::build_app_on_captured_tiles);
    harness.inner.step();

    test_util::harness::drop_file_and_wait_for_load(
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

    assert_the_capture_covers_the_map(&mut harness, "app_log_map_hexagons");
    harness.snapshot_loose("app_log_map_hexagons");
}

/// A recording the history database does not hold is identified by the session
/// identity it keeps for as long as it stays loaded: unloading it takes the logs
/// anchored to it with it.
#[test]
fn unloading_a_recording_outside_history_unloads_the_logs_anchored_to_it() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        recording_alongside_the_log("walk.gtd", 55.0),
    );
    drop_log_and_associate_it(&mut harness, &synthetic_log(8 * 1024), "navsyncd.log");
    harness.run_steps(3);
    assert!(
        matches!(
            harness.state().first_log().and_then(LoadedLog::anchor_key),
            Some(gt_log_view::RecordingKey::Session(_))
        ),
        "the recording is in no history database"
    );

    harness.state_mut().shared.borrow_mut().tree.pending_unload =
        Some(vec![gt_side_panel::NodeKey::File(FileIdx::new(0))]);
    harness.run_steps(3);

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);
    assert_eq!(harness.state().logs.len(), 0);
}

#[test]
fn choosing_a_target_in_the_footer_associates_the_log_against_it() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        recording_alongside_the_log("walk_a.gtd", 55.0),
    );
    test_util::harness::drop_file_and_wait_for_load(
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
            .first_log()
            .and_then(|log| log.associated_recording()),
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
            .first_log()
            .is_some_and(|log| log.associated_entry_count() > 0),
        "picking a target associates the log against it right away"
    );
}

/// The dialog every loaded log raises while a recording is open, and what
/// confirming it does.
mod log_association {
    use gt_store::{LogAttachmentEntry, Recordings, StoredLogFilter, StoredLogFilterMode};

    use gt_log_view::LogAttachmentRef;

    use crate::app::history_db::HistoryWorker;
    use crate::app::loader::AttachedLogRestore;
    use crate::app::log_viewer::association_dialog;

    use super::*;

    /// An app whose history worker owns a database of its own, so a log can be
    /// stored with a recording and read back.
    fn app_over_a_history_database(db_path: &std::path::Path) -> Harness<'static, App> {
        let mut harness = Harness::builder()
            .with_wait_for_pending_images(false)
            .build_eframe(test_util::harness::transient_app);
        harness.step();
        harness.state_mut().history = HistoryWorker::spawn(
            RecordingsHandle::Owner(open_temporary_history_database(db_path)),
            egui::Context::default(),
            gt_pending_writes::PendingWrites::default(),
        );
        harness.state_mut().sync_db_path();
        harness
    }

    fn drop_the_log(harness: &mut Harness<App>) {
        drop_a_log(harness, FIXTURE_LOG_SEED);
    }

    /// The seed [`drop_the_log`] writes its lines from.
    const FIXTURE_LOG_SEED: u64 = 7;

    /// Drops a log generated from `seed`. Two drops in one test need two
    /// seeds: the session holds one log per content.
    fn drop_a_log(harness: &mut Harness<App>, seed: u64) {
        drop_log_and_wait_for_load(harness, &fixture_log_text(seed), FIXTURE_LOG_NAME);
        harness.run_steps(3);
    }

    /// The log text of `seed`, which is one log however often it is generated:
    /// the session holds one log per content.
    fn fixture_log_text(seed: u64) -> String {
        synthetic_journald_log(SyntheticLogSpec {
            approx_bytes: 8 * 1024,
            seed,
            timestamps: SyntheticLogTimestamps::Iso8601Space,
        })
    }

    /// The name the fixture log is loaded and stored under.
    const FIXTURE_LOG_NAME: &str = "navsyncd.log";

    fn dialog_is_open(harness: &Harness<App>) -> bool {
        harness.state().association_dialog.is_some()
    }

    fn confirm(harness: &mut Harness<App>) {
        harness
            .get(By::new().label(association_dialog::CONFIRM_LABEL))
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
            .first_log()
            .and_then(gt_log_view::LoadedLog::associated_recording)
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
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );
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
                .first_log()
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
            .build_eframe(test_util::harness::transient_app);
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );
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
                .first_log()
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
            .build_eframe(test_util::harness::transient_app);
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk_a.gtd", 55.0),
        );
        test_util::harness::drop_file_and_wait_for_load(
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
                .first_log()
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
            .build_eframe(test_util::harness::transient_app);
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );
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
            .build_eframe(test_util::harness::transient_app);
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );
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

        // The attribute now holds the content hash of a different log.
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

        test_util::harness::drop_a_stored_recording_and_load_it_from_disk(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );

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
            .build_eframe(test_util::harness::transient_app);
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );
        drop_the_log(&mut harness);

        harness
            .get_by_label(association_dialog::DONT_SHOW_AGAIN_LABEL)
            .click();
        harness.run_steps(2);
        cancel(&mut harness);
        assert!(!harness.state().ask_log_association_target);

        drop_a_log(&mut harness, FIXTURE_LOG_SEED + 1);

        assert!(!dialog_is_open(&harness), "the dialog stays away");
        assert!(
            harness
                .state()
                .last_log()
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

        drop_a_log(&mut harness, FIXTURE_LOG_SEED + 2);

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

        // With the session copy unloaded, the attachment is what brings the
        // log back.
        unload_the_log(&mut harness);
        assert_eq!(harness.state().logs.len(), 0);

        // Opening the recording again restores the log it carries.
        test_util::harness::drop_a_stored_recording_and_load_it_from_disk(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );
        test_util::harness::step_until_a_log_is_loaded(&mut harness);
        let restored = harness
            .state()
            .first_log()
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
                .first_log()
                .is_some_and(|log| log.associated_recording().is_some()),
            "a restored log is associated with the recording that carried it"
        );
    }

    /// A log the recording holds is listed under it as soon as it is unloaded,
    /// and its load button reads it back with its anchor, its attachment and
    /// the filters it was stored with.
    #[test]
    fn an_unloaded_attachment_is_listed_under_its_recording_and_loads_back() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);
        add_log_filter_in(&mut harness, "kernel");
        assert!(
            harness.step_until(|_| stored_attachments(&db_path, &db_ref)
                .first()
                .is_some_and(|entry| entry.attachment.filters.len() == 1)),
            "the chip reached the stored attachment"
        );
        let stored = stored_attachments(&db_path, &db_ref)
            .first()
            .map(|entry| entry.id)
            .expect("the attachment was stored");
        assert!(
            harness
                .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
                .is_none(),
            "a log that is loaded is listed as the loaded log it is"
        );

        unload_the_log(&mut harness);
        assert_eq!(harness.state().logs.len(), 0);

        harness
            .get_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
            .click();
        test_util::harness::step_until_a_log_is_loaded(&mut harness);

        assert_eq!(
            harness
                .state()
                .first_log()
                .and_then(LoadedLog::attachment)
                .map(|attachment| attachment.id),
            Some(stored),
            "the log that came back is the one the recording holds"
        );
        assert!(
            harness
                .state()
                .first_log()
                .is_some_and(|log| log.associated_recording().is_some()),
            "the log takes its positions from the recording that holds it"
        );
        assert_eq!(
            harness
                .state()
                .first_log()
                .map(|log| log
                    .filters()
                    .chips()
                    .iter()
                    .map(|chip| chip.pattern().text.clone())
                    .collect::<Vec<_>>())
                .unwrap_or_default(),
            ["kernel".to_owned()],
            "the log came back under the stack it was stored with"
        );
        assert!(
            harness
                .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
                .is_none(),
            "the attachment is the loaded log's now"
        );
    }

    /// A recording leaving the session takes the logs it holds off the list
    /// with it.
    #[test]
    fn the_unloaded_attachments_of_a_removed_recording_leave_the_list() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);
        unload_the_log(&mut harness);
        assert!(
            harness
                .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
                .is_some()
        );

        open_the_shelve_confirmation(&mut harness, false);
        harness.get_by_label(SHELVE_BUTTON_LABEL).click();
        harness.run_steps(3);

        assert!(
            harness
                .query_by_label(log_viewer::log_list::LOAD_ATTACHMENT_LABEL)
                .is_none(),
            "the recording that held the log is gone from the session"
        );
        assert!(
            harness
                .state()
                .log_attachments
                .of_recording(&db_ref)
                .is_empty()
        );
    }

    /// Opens the shelve confirmation on the recording the session loaded
    /// first, with the permanent-delete box ticked when `permanently`.
    fn open_the_shelve_confirmation(harness: &mut Harness<App>, permanently: bool) {
        harness.state_mut().shared.borrow_mut().tree.shelve_confirm =
            Some(gt_side_panel::ShelveConfirmState {
                items: vec![gt_side_panel::NodeKey::File(FileIdx::new(0))],
                delete_permanently: permanently,
            });
        harness.run_steps(3);
    }

    /// A recording leaving the session takes its attached logs with it, and the
    /// dialog states that before the user confirms.
    #[test]
    fn shelving_every_track_of_a_recording_unloads_the_logs_it_holds() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);

        open_the_shelve_confirmation(&mut harness, false);
        harness.get_by_label_contains("Unloads 1 attached log");

        harness.get_by_label(SHELVE_BUTTON_LABEL).click();
        harness.run_steps(3);

        assert_eq!(harness.state().logs.len(), 0);
        assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);
    }

    /// Deleting the recording permanently takes its attached logs out of the
    /// session with it, and the dialog counts them before the user confirms.
    #[test]
    fn permanently_deleting_a_recording_states_the_attached_logs_it_deletes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);

        open_the_shelve_confirmation(&mut harness, true);
        harness.get_by_label_contains("Deletes 1 attached log with them");

        harness
            .get_by_label(DELETE_PERMANENTLY_BUTTON_LABEL)
            .click();
        harness.run_steps(3);

        assert_eq!(harness.state().logs.len(), 0);
        assert!(
            harness.step_until(|_| stored_attachments(&db_path, &db_ref).is_empty()),
            "the deleted recording took its attachment with it"
        );
    }

    /// The recording is opened again while its log is still loaded: the
    /// session keeps the one log, which keeps the attachment it holds.
    #[test]
    fn a_recording_opened_again_restores_no_attachment_that_is_already_loaded() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);
        let loaded = harness.state().logs.first_id();

        restore_the_stored_attachment(&mut harness, &db_path, &db_ref);

        assert_eq!(harness.state().logs.len(), 1, "no second copy was loaded");
        assert_eq!(
            harness.state().logs.first_id(),
            loaded,
            "the loaded log kept its identity"
        );
    }

    /// A log loaded on its own and anchored to the recording takes the
    /// attachment that recording carries, and no second copy is loaded.
    #[test]
    fn a_restored_attachment_reaches_the_loaded_log_anchored_to_that_recording() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);
        let stored = stored_attachments(&db_path, &db_ref)
            .first()
            .map(|entry| entry.id)
            .expect("the attachment was stored");

        // The same text, loaded by itself and anchored to the recording that
        // holds the attachment in the database.
        unload_the_log(&mut harness);
        drop_the_log(&mut harness);
        confirm(&mut harness);
        let loaded = harness.state().logs.first_id();
        assert_eq!(
            harness.state().first_log().and_then(LoadedLog::attachment),
            None,
            "a log the user loaded is stored nowhere"
        );

        restore_the_stored_attachment(&mut harness, &db_path, &db_ref);

        assert_eq!(harness.state().logs.len(), 1, "no second copy was loaded");
        assert_eq!(harness.state().logs.first_id(), loaded);
        assert_eq!(
            harness
                .state()
                .first_log()
                .and_then(LoadedLog::attachment)
                .map(|attachment| attachment.id),
            Some(stored),
            "the loaded log is now the log the recording carries"
        );
    }

    /// The toolbar's log button counts a log that comes back with a recording,
    /// and the viewer stays closed on the log it was showing.
    #[test]
    fn the_toolbar_counts_a_log_restored_with_a_recording_until_the_viewer_opens() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);
        unload_the_log(&mut harness);
        drop_a_log(&mut harness, FIXTURE_LOG_SEED + 1);
        cancel(&mut harness);
        let shown = harness.state().log_viewer.selected_log();
        harness.state_mut().log_viewer.open = false;
        harness.run_steps(2);

        restore_the_stored_attachment(&mut harness, &db_path, &db_ref);

        assert_eq!(harness.state().logs.len(), 2, "the attachment came back");
        assert!(!harness.state().log_viewer.open, "the viewer stays closed");
        assert_eq!(
            harness.state().log_viewer.selected_log(),
            shown,
            "the viewer still shows the log the user was reading"
        );
        assert_eq!(harness.state().log_viewer.restored_logs.count(), 1);
        assert!(harness.state().log_viewer.restored_logs.is_pulsing());
        harness.get_by_label(format!("{ICON_ARTICLE} 1").as_str());

        // The harness clock ticks a quarter second per frame, past the two
        // seconds the pulse runs for.
        harness.run_steps(12);

        assert!(
            !harness.state().log_viewer.restored_logs.is_pulsing(),
            "the pulse ends by itself"
        );
        assert_eq!(
            harness.state().log_viewer.restored_logs.count(),
            1,
            "the count stands until the viewer is opened"
        );

        harness
            .get_by_label(format!("{ICON_ARTICLE} 1").as_str())
            .click();
        harness.run_steps(3);

        assert!(harness.state().log_viewer.open);
        assert_eq!(harness.state().log_viewer.restored_logs.count(), 0);
        harness.get_by_label(ICON_ARTICLE);
    }

    /// Attaching a log the recording already holds reuses the stored
    /// attachment: the loaded log takes it, and the app writes its filter
    /// stack there.
    #[test]
    fn attaching_a_log_the_recording_already_holds_reuses_the_stored_attachment() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (mut harness, db_ref) = harness_over_a_recording_and_its_log(&db_path);
        attach_the_log(&mut harness, &db_path, &db_ref);
        add_log_filter_in(&mut harness, "kernel");
        assert!(
            harness.step_until(|_| stored_attachments(&db_path, &db_ref)
                .first()
                .is_some_and(|entry| entry.attachment.filters.len() == 1)),
            "the chip reached the stored attachment"
        );
        let stored = stored_attachments(&db_path, &db_ref)
            .first()
            .map(|entry| entry.id)
            .expect("the attachment was stored");

        // The same text loaded again, filtered by none of the chips the stored
        // attachment holds.
        unload_the_log(&mut harness);
        drop_the_log(&mut harness);
        assert!(
            harness.step_until(|harness| harness
                .query_by_label_contains("Attaching reuses that attachment")
                .is_some()),
            "the dialog states what the recording already holds"
        );
        harness
            .get(By::new().label(association_dialog::ATTACH_LABEL))
            .click();
        harness.state_mut().toasts.dismiss_all_toasts();
        confirm(&mut harness);

        assert!(
            harness.step_until(|_| stored_attachments(&db_path, &db_ref)
                .first()
                .is_some_and(|entry| entry.attachment.filters.is_empty())),
            "the stack of the loaded log reached the attachment it took"
        );
        assert_eq!(
            stored_attachments(&db_path, &db_ref)
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [stored],
            "the recording holds the one attachment it held before"
        );
        assert_eq!(
            harness
                .state()
                .first_log()
                .and_then(LoadedLog::attachment)
                .map(|attachment| attachment.id),
            Some(stored)
        );
        assert_eq!(harness.state().toasts.len(), 1, "the reuse raises a toast");
    }

    /// A database holding the fixture recording and the fixture log stored
    /// with it, as a session that attached the log left them.
    fn seed_a_recording_and_the_log_stored_with_it(
        db_path: &std::path::Path,
    ) -> (gt_store::DatabaseRef, gt_store::LogAttachmentId) {
        use gt_store::{LogAttachments as _, LogToAttach, TrackRange, TrackState};

        let bytes = recording_bytes_alongside_the_log(55.0);
        let meta = gt_store::extract_meta(&bytes).expect("the fixture recording carries metadata");
        let tracks = [TrackRange {
            start: 0,
            end: meta.nav_point_count,
            state: TrackState::Live,
        }];
        let mut db = open_temporary_history_database(db_path);
        let db_ref = db
            .insert(
                "walk.gtd",
                &meta,
                &tracks,
                crate::app::loader::stored_segmentation_from_config(
                    &gt_track_builder::SegmentationConfig::default(),
                ),
                &bytes,
            )
            .expect("the fixture recording is stored");
        let stored = db
            .attach_log(
                &db_ref,
                &LogToAttach {
                    name: FIXTURE_LOG_NAME,
                    text: &fixture_log_text(FIXTURE_LOG_SEED),
                    filters: vec![StoredLogFilter {
                        text: "kernel".to_owned(),
                        regex: false,
                        enabled: true,
                        mode: StoredLogFilterMode::Layer { color_slot: 0 },
                    }],
                },
            )
            .expect("the fixture log is stored with the recording");
        (db_ref, stored.id)
    }

    /// Opens one of a recording's stored logs, as the history window's
    /// "Open log" does.
    fn open_the_stored_log(
        harness: &Harness<App>,
        db_ref: &gt_store::DatabaseRef,
        id: gt_store::LogAttachmentId,
    ) {
        harness.state().history.load_attached_log(
            LogAttachmentRef {
                recording: db_ref.clone(),
                id,
            },
            FIXTURE_LOG_NAME.to_owned(),
        );
    }

    /// The chips the shown log is filtered by, in stack order.
    fn shown_log_chips(harness: &Harness<App>) -> Vec<String> {
        harness
            .state()
            .first_log()
            .map(|log| {
                log.filters()
                    .chips()
                    .iter()
                    .map(|chip| chip.pattern().text.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A log opened out of history while its recording is not loaded: it comes
    /// back attached, anchored to that recording, under the stack it was
    /// stored with, and no line of it has a position.
    #[test]
    fn opening_a_stored_log_alone_loads_it_anchored_and_without_positions() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (db_ref, stored) = seed_a_recording_and_the_log_stored_with_it(&db_path);
        let mut harness = app_over_a_history_database(&db_path);

        open_the_stored_log(&harness, &db_ref, stored);

        test_util::harness::step_until_a_log_is_loaded(&mut harness);
        assert_eq!(
            harness
                .state()
                .first_log()
                .and_then(LoadedLog::attachment)
                .map(|attachment| attachment.id),
            Some(stored)
        );
        assert_eq!(
            harness.state().first_log().and_then(LoadedLog::anchor_key),
            Some(&gt_log_view::RecordingKey::Stored(db_ref)),
            "the log is anchored to the recording that holds it"
        );
        assert_eq!(shown_log_chips(&harness), ["kernel".to_owned()]);
        assert_eq!(
            harness
                .state()
                .first_log()
                .map(|log| (log.associated_recording(), log.associated_entry_count())),
            Some((None, 0)),
            "no line has a position while the anchored recording is not loaded"
        );
        assert!(harness.state().log_viewer.open);
        assert_eq!(
            harness.state().log_viewer.selected_log(),
            harness.state().logs.first_id(),
            "the viewer opens on the log the user asked for"
        );
    }

    /// The same stored log opened twice: the session holds one log per
    /// content, whichever way that content arrives.
    #[test]
    fn opening_a_stored_log_that_is_already_loaded_loads_no_second_copy() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (db_ref, stored) = seed_a_recording_and_the_log_stored_with_it(&db_path);
        let mut harness = app_over_a_history_database(&db_path);
        open_the_stored_log(&harness, &db_ref, stored);
        test_util::harness::step_until_a_log_is_loaded(&mut harness);
        let loaded = harness.state().logs.first_id();
        harness.state_mut().toasts.dismiss_all_toasts();

        open_the_stored_log(&harness, &db_ref, stored);

        assert!(
            harness.step_until(|harness| harness.state().toasts.len() == 1),
            "the second copy raises a toast"
        );
        assert_eq!(harness.state().logs.len(), 1);
        assert_eq!(harness.state().logs.first_id(), loaded);
        assert_eq!(harness.state().log_viewer.selected_log(), loaded);
    }

    /// The footer's "Load recording" opens the anchored recording from
    /// history, and the log takes its positions from it as soon as it loads.
    #[test]
    fn loading_the_recording_of_a_stored_log_gives_its_lines_positions() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("geotrace.h5");
        let (db_ref, stored) = seed_a_recording_and_the_log_stored_with_it(&db_path);
        let mut harness = app_over_a_history_database(&db_path);
        open_the_stored_log(&harness, &db_ref, stored);
        test_util::harness::step_until_a_log_is_loaded(&mut harness);
        harness.run_steps(3);
        harness.get_by_label("Anchored to walk.gtd (not loaded)");

        harness
            .get_by_label(log_viewer::LOAD_RECORDING_LABEL)
            .click();

        test_util::harness::step_until_a_recording_is_loaded(&mut harness);
        assert!(
            harness.step_until(|harness| harness
                .state()
                .first_log()
                .is_some_and(|log| log.associated_entry_count() > 0)),
            "the lines take their positions from the recording that loaded"
        );
        assert!(
            harness
                .state()
                .first_log()
                .is_some_and(|log| log.associated_recording().is_some())
        );
        assert!(
            harness
                .query_by_label(log_viewer::LOAD_RECORDING_LABEL)
                .is_none(),
            "the footer states the loaded recording, with nothing left to open"
        );
    }

    /// Hands the app the attachment a recording load reads back, parsed as the
    /// loader's worker parses it.
    fn restore_the_stored_attachment(
        harness: &mut Harness<App>,
        db_path: &std::path::Path,
        db_ref: &gt_store::DatabaseRef,
    ) {
        let stored = stored_attachments(db_path, db_ref);
        let entry = stored.first().expect("the attachment was stored");
        let text = fixture_log_text(FIXTURE_LOG_SEED);
        let parsed = gt_logfile::parse_log(text.as_str().into(), synthetic_log_start())
            .expect("the fixture log parses");
        let restore = AttachedLogRestore {
            attachment: LogAttachmentRef {
                recording: db_ref.clone(),
                id: entry.id,
            },
            filters: entry.attachment.filters.clone(),
            requested_by: crate::app::loader::AttachedLogRequester::RecordingLoad,
        };
        harness.state_mut().load_parsed_log(
            Some(entry.attachment.name.clone()),
            parsed,
            Some(restore),
        );
        harness.run_steps(2);
    }

    /// Unloads the shown log through the viewer's own button, which the
    /// selector row draws before any filter chip's.
    fn unload_the_log(harness: &mut Harness<App>) {
        harness
            .nth_matching(By::new().label(egui_phosphor::regular::X), 0)
            .click();
        harness.run_steps(3);
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
        test_util::harness::drop_a_stored_recording_and_load_it_from_disk(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );

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
                .first_log()
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
        test_util::harness::drop_file_and_wait_for_load(
            &mut harness,
            recording_alongside_the_log("walk.gtd", 55.0),
        );
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
            "the viewer reports what the database rejected"
        );
        assert_eq!(harness.state().logs.len(), 1);
        assert!(
            harness
                .state()
                .first_log()
                .is_some_and(|log| log.attachment().is_none()),
            "nothing about the loaded log changed"
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

/// A window the app owns, opened with content far larger than any of the audit
/// viewports.
#[derive(Debug, Clone, Copy)]
enum OversizedAppWindow {
    HistoryDatabaseInUse,
    HistoryDatabaseLocked,
    HistoryDatabaseCorrupted,
    TrackSettingsDiffer,
    AutoPrune,
    Settings,
    Query,
    TrackData,
    LoadingProgress,
}

impl OversizedAppWindow {
    fn audited(self) -> gt_test_utils::AuditedWindow<'static> {
        match self {
            Self::HistoryDatabaseInUse => {
                gt_test_utils::AuditedWindow::titled(HISTORY_DATABASE_IN_USE_TITLE)
            }
            Self::HistoryDatabaseLocked => {
                gt_test_utils::AuditedWindow::titled(HISTORY_DATABASE_LOCKED_TITLE)
            }
            Self::HistoryDatabaseCorrupted => {
                gt_test_utils::AuditedWindow::titled(HISTORY_DATABASE_CORRUPTED_TITLE)
            }
            Self::TrackSettingsDiffer => {
                gt_test_utils::AuditedWindow::titled(TRACK_SETTINGS_DIFFER_TITLE)
            }
            Self::AutoPrune => gt_test_utils::AuditedWindow::titled(AUTO_PRUNE_TITLE),
            Self::Settings => gt_test_utils::AuditedWindow::identified(
                "Settings",
                egui::Id::new(settings_ui::WINDOW_ID),
            ),
            Self::Query => gt_test_utils::AuditedWindow::titled("Query"),
            Self::TrackData => gt_test_utils::AuditedWindow::identified(
                "Track data",
                egui::Id::new("detached_panel"),
            ),
            Self::LoadingProgress => {
                gt_test_utils::AuditedWindow::titled(LOADING_OVERLAY_WINDOW_ID)
            }
        }
    }

    /// The control the user must still be able to reach. The windows a user
    /// only reads have none of their own.
    fn reachable_control(self) -> Option<&'static str> {
        match self {
            Self::HistoryDatabaseInUse => Some("Try again"),
            Self::HistoryDatabaseLocked
            | Self::HistoryDatabaseCorrupted
            | Self::TrackSettingsDiffer
            | Self::AutoPrune => Some("Cancel"),
            Self::Settings | Self::Query | Self::TrackData | Self::LoadingProgress => None,
        }
    }

    fn open_on(self, app: &mut App) {
        let long = gt_test_utils::oversized_text('a');
        match self {
            Self::HistoryDatabaseInUse => {
                app.history_failure = Some(crate::app::storage::HistoryFailure::Busy(
                    PathBuf::from(long),
                ));
            }
            Self::HistoryDatabaseLocked => {
                app.history_failure = Some(crate::app::storage::HistoryFailure::Locked(
                    PathBuf::from(long),
                ));
            }
            Self::HistoryDatabaseCorrupted => {
                app.history_failure = Some(crate::app::storage::HistoryFailure::Unreadable(
                    PathBuf::from(long),
                ));
            }
            Self::TrackSettingsDiffer => {
                app.pending_resegment = Some(super::ResegmentPrompt {
                    db_ref: gt_store::DatabaseRef {
                        identity: long.clone(),
                        group_name: long.clone(),
                    },
                    filename: long,
                    bytes: std::sync::Arc::from(Vec::<u8>::new()),
                    stored: gt_store::StoredSegmentation {
                        track_split_gap_us: 60_000_000,
                        track_split_rule: gt_store::StoredTrackSplitRule::StepInEitherDirection,
                        fix_placement_rule:
                            gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
                        detect_clock_discontinuities: false,
                        clock_discontinuity_sigmas: 4.0,
                    },
                    stored_tracks: Vec::new(),
                    marker_settings_changed: false,
                });
            }
            Self::AutoPrune => {
                app.pending_auto_prune = Some(
                    (0..gt_test_utils::window_fit::OVERSIZED_ROW_COUNT)
                        .map(|index| gt_store::DatabaseRef {
                            identity: format!("{long}/{index}"),
                            group_name: long.clone(),
                        })
                        .collect(),
                );
            }
            Self::Settings => app.settings_open = true,
            Self::Query => app.query_window.open = true,
            Self::TrackData => app.shared.borrow_mut().tree.detached = true,
            Self::LoadingProgress => {
                app.loader.loading_jobs = (0..gt_test_utils::window_fit::OVERSIZED_ROW_COUNT)
                    .map(|index| crate::app::loader::LoadingJob {
                        id: index as u64,
                        filename: format!("{long}/{index}"),
                        progress: 0.5,
                        stage: "reading",
                        started_at: 0.0,
                    })
                    .collect();
            }
        }
    }
}

/// Every window the app owns stays inside the screen and keeps its action
/// reachable, however much the state behind it holds.
#[rstest]
fn every_app_window_fits_the_audit_viewports(
    #[values(
        OversizedAppWindow::HistoryDatabaseInUse,
        OversizedAppWindow::HistoryDatabaseLocked,
        OversizedAppWindow::HistoryDatabaseCorrupted,
        OversizedAppWindow::TrackSettingsDiffer,
        OversizedAppWindow::AutoPrune,
        OversizedAppWindow::Settings,
        OversizedAppWindow::Query,
        OversizedAppWindow::TrackData,
        OversizedAppWindow::LoadingProgress
    )]
    window: OversizedAppWindow,
    #[values(
        gt_test_utils::window_fit::CRAMPED_VIEWPORT,
        gt_test_utils::window_fit::NARROW_VIEWPORT,
        gt_test_utils::window_fit::SHORT_VIEWPORT
    )]
    viewport: egui::Vec2,
) {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(viewport)
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    window.open_on(harness.inner.state_mut());
    harness.inner.run_steps(8);

    harness
        .inner
        .assert_window_fits_the_viewport(window.audited());
    if let Some(control) = window.reachable_control() {
        harness
            .inner
            .assert_control_is_reachable(window.audited(), ControlLabel(control));
    }
}

/// The self-update prompt stays inside the screen and keeps its dismissals
/// reachable, however long the offered version reads.
#[cfg(feature = "self-update")]
#[rstest]
fn the_update_prompt_fits_the_audit_viewports(
    #[values(
        gt_test_utils::window_fit::CRAMPED_VIEWPORT,
        gt_test_utils::window_fit::NARROW_VIEWPORT,
        gt_test_utils::window_fit::SHORT_VIEWPORT
    )]
    viewport: egui::Vec2,
) {
    let window = gt_test_utils::AuditedWindow::titled(super::update::UPDATE_DIALOG_TITLE);
    let (mut harness, _config_path) = TestHarness::builder()
        .size(viewport)
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().update_checker =
        super::update::UpdateChecker::available_for_test(&gt_test_utils::oversized_text('a'), true);
    harness.inner.run_steps(8);

    harness.inner.assert_window_fits_the_viewport(window);
    harness
        .inner
        .assert_control_is_reachable(window, ControlLabel("Skip this version"));
}

/// The popped-out match list stays inside the screen at any viewport: its rows
/// scroll inside it.
#[rstest]
fn the_match_list_window_fits_the_audit_viewports(
    #[values(
        gt_test_utils::window_fit::CRAMPED_VIEWPORT,
        gt_test_utils::window_fit::NARROW_VIEWPORT,
        gt_test_utils::window_fit::SHORT_VIEWPORT
    )]
    viewport: egui::Vec2,
) {
    let mut harness = Harness::builder()
        .with_size(viewport)
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(DEMO_BYTES, "demo_trip.gtd"),
    );
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    run_query(&mut harness, MANY_MATCH_QUERY);
    pop_out_button(&harness).click();
    harness.run_steps(8);

    harness.assert_window_fits_the_viewport(gt_test_utils::AuditedWindow::titled(
        query::results::MATCH_LIST_WINDOW_TITLE,
    ));
}

/// Recordings dropped in one batch, one more than the overlay lists at once.
const BATCH_PAST_THE_LISTED_JOBS: usize = LOADING_OVERLAY_MOST_LISTED_JOBS + 1;

/// Recordings dropped in one batch, far more than the overlay lists at once.
const BATCH_FAR_PAST_THE_LISTED_JOBS: usize = LOADING_OVERLAY_MOST_LISTED_JOBS + 40;

/// 1280x800, where the map fills the top two thirds of the window and the plot
/// the bottom third.
const DESKTOP_VIEWPORT: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Loads still running, which the overlay lists with a progress bar each.
#[derive(Clone, Copy)]
struct RunningJobCount(usize);

/// Loads that have finished, which the overlay lists under the running ones
/// while they fade.
#[derive(Clone, Copy)]
struct FinishedJobCount(usize);

/// The app with the progress overlay in the bottom-right corner, where the
/// map's layer and display toggles also sit.
fn app_with_a_batch_of_load_jobs(
    viewport: egui::Vec2,
    running: RunningJobCount,
    finished: FinishedJobCount,
) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(viewport)
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    let now = harness.inner.ctx.input(|input| input.time);
    let app = harness.state_mut();
    app.loader.loading_jobs = (0..running.0)
        .map(|index| crate::app::loader::LoadingJob {
            id: index as u64,
            filename: format!("ride-2026-05-{:02}.gtd", index + 1),
            progress: 0.2 + 0.15 * (index % 5) as f32,
            stage: crate::app::loader::STAGE_READING,
            started_at: now,
        })
        .collect();
    app.loader.finishing_jobs = (0..finished.0)
        .map(|index| crate::app::loader::FinishedJob {
            filename: format!("walk-2026-04-{:02}.gtd", index + 1),
            elapsed_secs: 1.4,
            completed_at: now,
        })
        .collect();
    harness.inner.run_steps(4);
    harness
}

#[test]
fn snapshot_loading_overlay_past_the_listed_jobs() {
    let mut harness = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(BATCH_FAR_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );
    harness.snapshot_loose("loading_overlay_past_the_listed_jobs");
}

/// The overlay lists [`LOADING_OVERLAY_MOST_LISTED_JOBS`] jobs whatever the
/// batch behind it holds.
#[test]
fn the_loading_overlay_opens_at_one_height_for_every_batch_past_the_listed_jobs() {
    let past = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(BATCH_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );
    let far_past = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(BATCH_FAR_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );

    assert_eq!(
        far_past
            .inner
            .window_rect(LOADING_OVERLAY_WINDOW_ID)
            .expect("the overlay lists the batch")
            .size(),
        past.inner
            .window_rect(LOADING_OVERLAY_WINDOW_ID)
            .expect("the overlay lists the batch")
            .size(),
        "{BATCH_FAR_PAST_THE_LISTED_JOBS} loads made the overlay taller than \
         {BATCH_PAST_THE_LISTED_JOBS} did: it has to list \
         {LOADING_OVERLAY_MOST_LISTED_JOBS} of them and count the rest"
    );
}

/// The line under the listed jobs counts the ones left over, whether those are
/// still running, finished, or both.
#[rstest]
#[case::all_running(BATCH_FAR_PAST_THE_LISTED_JOBS, 0)]
#[case::all_finished(0, BATCH_FAR_PAST_THE_LISTED_JOBS)]
#[case::one_running_over_a_finished_batch(1, BATCH_FAR_PAST_THE_LISTED_JOBS)]
fn the_loading_overlay_counts_the_jobs_it_does_not_list(
    #[case] running: usize,
    #[case] finished: usize,
) {
    let harness = app_with_a_batch_of_load_jobs(
        DESKTOP_VIEWPORT,
        RunningJobCount(running),
        FinishedJobCount(finished),
    );
    let unlisted = running + finished - LOADING_OVERLAY_MOST_LISTED_JOBS;

    assert!(
        harness
            .inner
            .query_by_label(format!("{unlisted} more").as_str())
            .is_some(),
        "the overlay listed {LOADING_OVERLAY_MOST_LISTED_JOBS} of {running} running and \
         {finished} finished jobs without counting the {unlisted} it left out"
    );
}

/// The overlay covers the map's display toggle on a short screen. A press
/// there has to reach the toggle: the overlay has no controls and egui leaves
/// it out of the hit-test.
#[test]
fn the_map_display_toggle_opens_on_a_press_under_the_loading_overlay() {
    let mut harness = app_with_a_batch_of_load_jobs(
        gt_test_utils::window_fit::SHORT_VIEWPORT,
        RunningJobCount(BATCH_FAR_PAST_THE_LISTED_JOBS),
        FinishedJobCount(0),
    );
    let toggle = harness
        .inner
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::new(gt_map::DISPLAY_TOGGLE_BUTTON_AREA_ID)))
        .expect("the map draws its display toggle");
    let overlay = harness
        .inner
        .window_rect(LOADING_OVERLAY_WINDOW_ID)
        .expect("the overlay lists the batch");
    assert!(
        overlay.intersects(toggle),
        "the overlay at {overlay:?} does not cover the display toggle at {toggle:?}: the press \
         below would not cross the overlay"
    );

    harness.inner.click_at(toggle.center());
    harness.inner.run_steps(2);

    assert!(
        harness
            .inner
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new(gt_map::DISPLAY_TOGGLE_POPUP_AREA_ID)))
            .is_some(),
        "the press on the display toggle did not open its popup: egui routed it to the loading \
         overlay instead"
    );
}
