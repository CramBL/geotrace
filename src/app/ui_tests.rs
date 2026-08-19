use egui::TextEdit;
use egui_phosphor::regular::ARROW_LINE_UP_LEFT as ICON_ARROW_LINE_UP_LEFT;
use egui_phosphor::regular::ARTICLE as ICON_ARTICLE;
use egui_phosphor::regular::DOTS_SIX as ICON_DOTS_SIX;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use egui_phosphor::regular::PUSH_PIN as ICON_PUSH_PIN;
use egui_phosphor::regular::TERMINAL_WINDOW as ICON_TERMINAL_WINDOW;
use egui_phosphor::regular::X as ICON_X;
use std::path::PathBuf;
use std::{
    sync::Arc,
    time::{Duration as StdDuration, Instant},
};

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use geotrace_sdk::{Channel, ChannelUnit, DateTime, Duration, Unit, Utc};
use gt_test_utils::{
    By, DEMO_BYTES, GOLD_BYTES, HarnessInteraction as _, SyntheticGtdSpec, SyntheticLogSpec,
    SyntheticLogTimestamps, TestHarness, synthetic_gtd_bytes, synthetic_journald_log,
    synthetic_log_start,
};
use gt_types::{FileIdx, LoadWarning, TrackIdx, TrackRef};
use strum::IntoEnumIterator as _;

use super::App;
use super::log_viewer;
use super::settings_ui::{self, SettingsPage};

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
    App::new_with_config(
        cc,
        &[],
        Some(config_path.to_path_buf()),
        super::StartupOptions {
            fading_enabled: fading,
            offline: true,
            storage: crate::app::Storage::Disabled,
            app_version: super::TEST_APP_VERSION,
        },
    )
}

/// Fixes every download control's date range, or a snapshot of the settings
/// window would redate every day.
fn pin_backfill_ranges(app: &mut App) {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 2).unwrap_or_default();
    app.interference_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
    app.geomagnetic_index_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
    app.tec_map_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
    app.solar_flare_backfill_ui = crate::app::backfill_ui::BackfillUi::with_today(today);
}

/// App constructor for the functional (non-snapshot) tests that don't touch a
/// config file. Fading stays off so frame counts are deterministic.
fn transient_app(cc: &mut eframe::CreationContext<'_>) -> App {
    App::new_with_config(
        cc,
        &[],
        None,
        super::StartupOptions {
            fading_enabled: false,
            offline: true,
            storage: crate::app::Storage::Disabled,
            app_version: super::TEST_APP_VERSION,
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

/// Hovering a match header in the query results table cross-highlights the
/// whole match: its range lands in `hover_match` (the map halo band and the
/// plot time band read it) and the match's track gets hover focus.
#[test]
fn query_match_header_hover_highlights_the_match() {
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

    // The match header reads "test.gtd #0 — <time> — <count> points".
    let header_pos = harness.get_by_label_contains("test.gtd #0").rect().center();
    harness.hover_at(header_pos);
    harness.run_steps(2);

    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let highlight = harness.state().shared.borrow().highlight;
    let hover_match = highlight.hover_match.expect("header hover sets the match");
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

    // Pointer off the header: the cross-highlight clears the next frame.
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
        "the highlight clears when the pointer leaves the header"
    );
}

#[test]
fn drag_drop_unknown_bytes_sets_error() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    // \xff is not valid UTF-8 and doesn't match the HDF5 magic, so the error
    // is detected synchronously without spawning a background thread.
    drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(b"\xff\xfe\x00binary_junk".as_slice(), "mystery.bin"),
    );

    assert!(harness.state().load_error.is_some());
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 0);
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
/// editor text, a run whose matches draw as halos on the map, the run
/// summary, and an expanded match table.
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

    // Expand the second match (the smaller one) so the snapshot covers the
    // point table with the query's columns.
    harness.inner.get_by_label_contains("12 points").click();
    harness.inner.run_steps(10);

    // Expand the query-history and examples lists so the snapshot documents
    // them: history now holds the run above, examples lists the built-ins.
    // Re-render between clicks so the second targets the reflowed layout.
    harness.inner.get_by_label("Query history").click();
    harness.inner.run_steps(5);
    harness.inner.get_by_label("Examples").click();
    harness.inner.run_steps(10);

    let history_len = harness.inner.state().query_window.history().len();
    assert_eq!(history_len, 1, "the run above is recorded in history");

    harness.snapshot_loose("app_query_window");
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

    // Hover the larger match's header. The cross-highlight lands on the map
    // and plot a frame later.
    let header_pos = harness
        .inner
        .get_by_label_contains("62 points")
        .rect()
        .center();
    harness.inner.hover_at(header_pos);
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
/// component (`@accel.x`) runs standalone, the results list per-track sample
/// tables (time plus each component), and the map halos the track segments
/// the matched samples cover.
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

    // The crafted stretches match: the per-track sample table lists them
    // and the map draws halos over the covered segments.
    let matched: usize = ACCEL_HIGH_RANGES.iter().map(|r| r.len()).sum();
    harness
        .inner
        .get_by_label_contains(&format!("{matched} samples"));

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

    // Expand the first match so the snapshot shows the point table with the
    // query's `table time, velocity` columns.
    let first_match = harness
        .inner
        .query_all_by_label_contains("accel_demo.gtd #0")
        .next()
        .expect("the run lists match headers");
    first_match.click();
    harness.inner.run_steps(10);
    // Park the pointer off the header so the hovered-match cross-highlight
    // (its own snapshot) does not blend into this one.
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
            #[cfg(feature = "self-update")]
            Self::Application => "Confirm before pruning",
        }
    }

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
    pin_backfill_ranges(harness.inner.state_mut());
    (harness, config_path)
}

// The settings window renders a `self-update`-only page ("Application"), so its
// appearance depends on that feature. Gating the snapshot on the feature means
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
    click_settled(&mut harness, "Auto-store recordings");
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
        harness.step_until(|h| h.query_by_label("Auto-store recordings").is_some()),
        "the Application page shows the auto-store checkbox"
    );
    click_settled(&mut harness, "Auto-store recordings");
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
        .by_track
        .get(&track)
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

    let mut harness = gt_test_utils::TestHarness::from_harness(harness);
    harness.snapshot_loose("app_plot_context_line");
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
    );
    harness.run_steps(5);
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

fn app_with_a_log_loaded() -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    drop_log_and_wait_for_load(&mut harness, &synthetic_log(64 * 1024), "navsyncd.log");
    harness.run_steps(3);
    harness
}

fn parse_summary_of_the_shown_log(harness: &Harness<App>) -> String {
    harness
        .state()
        .logs
        .get(harness.state().log_viewer.selected_log_index())
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
    drop_log_and_wait_for_load(
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
    harness.inner.ctx.memory_mut(|memory| {
        memory.request_focus(egui::Id::new(log_viewer::filters::LIVE_FILTER_FIELD_ID));
    });
    harness.inner.run_steps(2);
    harness
        .inner
        .input_mut()
        .events
        .push(egui::Event::Text(text.to_owned()));
    harness.inner.run_steps(6);
}

/// Writes `text` into the live filter and keeps it as a chip.
fn add_log_filter(harness: &mut TestHarness<'_, App>, text: &str) {
    type_into_log_filter(harness, text);
    harness
        .inner
        .get_by_label(log_viewer::filters::ADD_FILTER_LABEL)
        .click();
    harness.inner.run_steps(5);
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
    drop_log_and_wait_for_load(
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
    harness.inner.run_steps(5);
    type_into_log_filter(&mut harness, "retries");

    harness.snapshot_loose("app_log_viewer_filters");
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
    drop_log_and_wait_for_load(&mut harness.inner, &synthetic_log(8 * 1024), "navsyncd.log");
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
    drop_log_and_wait_for_load(
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
