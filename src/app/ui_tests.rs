use std::{
    sync::Arc,
    thread,
    time::{Duration as StdDuration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable as _};
use geotrace_sdk::{DateTime, Utc};
use gt_test_utils::{DEMO_BYTES, GOLD_BYTES, SyntheticGtdSpec, TestHarness, synthetic_gtd_bytes};
use gt_types::{FileIdx, LoadWarning, TrackIdx, TrackRef};

use super::App;

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
        },
    )
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

/// Step the harness repeatedly until all background load jobs have finished.
///
/// Background threads send a `Completed` message when done. `drain_load_channel`
/// (called at the start of every `ui()` frame) picks it up and removes the job.
/// We sleep briefly between steps to let the threads make progress.
fn step_until_loaded(harness: &mut Harness<App>) {
    for _ in 0..200 {
        if harness.state().loader.loading_jobs.is_empty() {
            return;
        }
        thread::sleep(StdDuration::from_millis(5));
        harness.step();
    }
    // One final step in case the last drain hadn't run yet.
    harness.step();
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
        harness.input_mut().dropped_files.push(egui::DroppedFile {
            bytes: Some(Arc::from(bytes)),
            name: name.to_owned(),
            ..Default::default()
        });
        harness.step();
        step_until_loaded(harness);
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
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        path: Some(tmp_path),
        ..Default::default()
    });
    harness.step(); // processes the drop, spawns load thread
    step_until_loaded(&mut harness); // waits for thread + drains channel

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

#[test]
fn drag_drop_gtd_bytes_loads_file() {
    let gtd_bytes = minimal_gtd_bytes();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        bytes: Some(Arc::from(gtd_bytes.as_slice())),
        name: "test.gtd".to_owned(),
        ..Default::default()
    });
    harness.step(); // processes the drop, spawns load thread
    step_until_loaded(&mut harness); // waits for thread + drains channel

    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

#[test]
fn drag_drop_unknown_bytes_sets_error() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    // \xff is not valid UTF-8 and doesn't match the HDF5 magic, so the error
    // is detected synchronously without spawning a background thread.
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        bytes: Some(Arc::from(b"\xff\xfe\x00binary_junk".as_slice())),
        name: "mystery.bin".to_owned(),
        ..Default::default()
    });
    harness.step();

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
/// `egui::Window` renders the detached panel as a floating overlay inside the
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

#[test]
fn settings_window_closes_on_esc() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.step();
    harness.state_mut().settings_open = true;
    harness.step(); // window open
    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
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

#[test]
fn legend_redock_icon_resets_offset_to_default() {
    let mut harness = harness_with_three_files_loaded();
    detach_legend(&mut harness, egui::vec2(220.0, 120.0));

    harness
        .get_by_label(egui_phosphor::regular::ARROW_LINE_UP_LEFT)
        .click();
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
    let drag_handle = harness.get_by_label(egui_phosphor::regular::DOTS_SIX);
    let start = drag_handle.rect().center();
    let end = start + egui::vec2(120.0, 70.0);
    harness.drag_at(start);
    harness.hover_at(end);
    harness.drop_at(end);
    harness.step();

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

    let drag_handle = harness.get_by_label(egui_phosphor::regular::DOTS_SIX);
    let start = drag_handle.rect().center();
    harness.drag_at(start);
    harness.step();

    let mut pos = start;
    for _ in 0..10 {
        pos += egui::vec2(20.0, 15.0);
        harness.hover_at(pos);
        harness.step();
    }

    harness.drop_at(pos);
    harness.step();

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

    let drag_handle = harness.get_by_label(egui_phosphor::regular::DOTS_SIX);
    let start = drag_handle.rect().center();
    let end = start - egui::vec2(210.0, 110.0);
    harness.drag_at(start);
    harness.hover_at(end);
    harness.drop_at(end);
    harness.step();

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

    harness
        .inner
        .input_mut()
        .dropped_files
        .push(egui::DroppedFile {
            bytes: Some(Arc::from(GOLD_BYTES)),
            name: "gold.gtd".to_owned(),
            ..Default::default()
        });
    harness.inner.step();
    step_until_loaded(&mut harness.inner);
    // The app repaints continuously (map + background jobs). Run many frames
    // so the map zoom and plot layout converge before we snapshot.
    harness.inner.run_steps(60);

    // Use per-test tolerance: this snapshot includes live map/plot rendering,
    // so allow tiny pixel-level variance across runs and platforms.
    harness.snapshot_with_tolerance("app_with_file_loaded", 2.5, 4);
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

    harness
        .inner
        .input_mut()
        .dropped_files
        .push(egui::DroppedFile {
            bytes: Some(Arc::from(GOLD_BYTES)),
            name: "gold.gtd".to_owned(),
            ..Default::default()
        });
    harness.inner.step();
    step_until_loaded(&mut harness.inner);

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

    harness
        .inner
        .input_mut()
        .dropped_files
        .push(egui::DroppedFile {
            bytes: Some(Arc::from(DEMO_BYTES)),
            name: "demo_trip.gtd".to_owned(),
            ..Default::default()
        });
    harness.inner.step();
    step_until_loaded(&mut harness.inner);

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

    harness
        .inner
        .input_mut()
        .dropped_files
        .push(egui::DroppedFile {
            bytes: Some(Arc::from(DEMO_BYTES)),
            name: "demo_trip.gtd".to_owned(),
            ..Default::default()
        });
    harness.inner.step();
    step_until_loaded(&mut harness.inner);

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
    harness.inner.run_steps(60);

    let match_count = {
        let app = harness.inner.state();
        app.query_window
            .matches()
            .map_or(0, |m| m.ranges.values().map(Vec::len).sum())
    };
    assert!(match_count > 0, "the demo trip has stretches above 25 km/h");

    // Expand the second match (the smaller one) so the snapshot covers the
    // point table with the query's columns.
    harness.inner.get_by_label_contains("12 points").click();
    harness.inner.run_steps(10);

    harness.snapshot_loose("app_query_window");
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

// The settings window renders a `self-update`-only row ("Check for updates on
// startup"), so its appearance depends on that feature. Gating the snapshot on
// the feature means the reference image can only ever be generated and compared
// in the same configuration CI uses (`just test` / `just test-snapshots` both
// enable it). Without this, regenerating snapshots in a build that lacks the
// feature would silently drop that row and break macOS CI. Any future
// feature-dependent snapshot must be gated the same way.
#[cfg(feature = "self-update")]
#[test]
fn snapshot_settings_window() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(600.0, 400.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().settings_open = true;
    harness.run();
    harness.snapshot("settings_window");
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

/// A non-self-updatable build (Homebrew / MSI / manual download) shows no dialog;
/// it exposes the available version for the subtle menu-bar badge instead.
#[cfg(feature = "self-update")]
#[test]
fn non_self_update_uses_badge_not_dialog() {
    let badge = super::update::UpdateChecker::available_for_test("0.2.0", false);
    assert_eq!(badge.badge_version().as_deref(), Some("0.2.0"));

    let self_updatable = super::update::UpdateChecker::available_for_test("0.2.0", true);
    assert_eq!(self_updatable.badge_version(), None);
}

#[test]
fn snapshot_history_locked_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_history_unlock =
        Some(std::path::PathBuf::from("geotrace.h5"));
    harness.run();
    harness.snapshot("history_locked_dialog");
}

#[test]
fn snapshot_history_corrupt_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_db_corruption = Some(std::path::PathBuf::from("geotrace.h5"));
    harness.run();
    harness.snapshot("history_corrupt_dialog");
}

#[test]
fn snapshot_history_resegment_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_resegment = Some(super::ResegmentPrompt {
        db_ref: gt_history::DatabaseRef {
            identity: "auto:ride.gtd".to_owned(),
            group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
        },
        filename: "ride.gtd".to_owned(),
        bytes: std::sync::Arc::from(Vec::<u8>::new()),
        stored: gt_history::StoredSegmentation {
            track_split_gap_us: 60_000_000,
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 4.0,
        },
        hidden_positions: Vec::new(),
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
