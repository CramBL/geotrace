use std::path::PathBuf;
use std::{
    sync::Arc,
    thread,
    time::{Duration as StdDuration, Instant},
};

use egui_kittest::{Harness, kittest::Queryable as _};
use geotrace_sdk::{Channel, ChannelUnit, DateTime, Duration, Unit, Utc};
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

/// Step the harness repeatedly until the query worker's result has landed.
fn step_until_query_result(harness: &mut Harness<App>) {
    for _ in 0..200 {
        if harness.state().query_window.matches().is_some() {
            return;
        }
        thread::sleep(StdDuration::from_millis(5));
        harness.step();
    }
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

/// Query results gray out when the data they were computed from changes -
/// here via a global-filter edit - and recover when it changes back.
#[test]
fn query_results_go_stale_when_the_filter_changes() {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        bytes: Some(Arc::from(gtd_bytes.as_slice())),
        name: "test.gtd".to_owned(),
        ..Default::default()
    });
    harness.step();
    step_until_loaded(&mut harness);

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
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        bytes: Some(Arc::from(gtd_bytes.as_slice())),
        name: "test.gtd".to_owned(),
        ..Default::default()
    });
    harness.step();
    step_until_loaded(&mut harness);

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

/// The same loaded-file view under the light theme, so the side panel, chip row,
/// and plot are all exercised on a light background - the general light-mode
/// baseline alongside the plot- and badge-specific ones.
#[test]
fn snapshot_app_with_file_loaded_light() {
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
    // The run executes on a worker thread; step until its results land.
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
    app.query_window
        .set_text("points | window 10 | where avg(velocity) > 25 km/h".to_owned());
    harness.inner.run_steps(5);

    harness
        .inner
        .get_by_role_and_label(egui::accesskit::Role::Button, "Run")
        .click();
    step_until_query_result(&mut harness.inner);
    harness.inner.run_steps(60);

    // Hover the larger match's header; the cross-highlight lands on the map
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
            *vis.field_mut(kind) = kind == gt_types::MetricKind::Velocity;
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
/// invisible; this is the baseline that guards the theme-aware `metric_color`
/// light variants so a regression there fails CI instead of only being caught
/// by eye in dark mode.
#[test]
fn snapshot_app_plot_light() {
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
            *vis.field_mut(kind) = shown.contains(&kind);
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

    harness
        .inner
        .input_mut()
        .dropped_files
        .push(egui::DroppedFile {
            bytes: Some(Arc::from(accel_channel_gtd_bytes())),
            name: "accel_demo.gtd".to_owned(),
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

    harness
        .inner
        .input_mut()
        .dropped_files
        .push(egui::DroppedFile {
            bytes: Some(Arc::from(accel_channel_gtd_bytes())),
            name: "accel_demo.gtd".to_owned(),
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
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        bytes: Some(Arc::from(gtd_bytes.as_slice())),
        name: "test.gtd".to_owned(),
        ..Default::default()
    });
    harness.step();
    step_until_loaded(&mut harness);

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

    // A config from before the display mask existed loads as all-visible.
    let old_config: crate::settings::Settings =
        toml::from_str("[map]\nsync_to_map = false\n").expect("old config parses");
    assert!(!old_config.map.display_mask.any_hidden());
}

/// Build an app with one loaded file and the query window open. Shared setup
/// for the interactive query-history tests.
fn app_with_query_window_open() -> Harness<'static, App> {
    let gtd_bytes = minimal_gtd_bytes();
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(transient_app);
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        bytes: Some(Arc::from(gtd_bytes.as_slice())),
        name: "test.gtd".to_owned(),
        ..Default::default()
    });
    harness.step();
    step_until_loaded(&mut harness);
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
    harness
        .get_by_label(egui_phosphor::regular::PUSH_PIN)
        .click();
    harness.run_steps(3);
    {
        let window = &harness.state().query_window;
        assert!(window.history()[0].pinned, "clicking pin pins the entry");
        assert!(
            window.history_revision() > revision_before,
            "pinning bumps the revision so settings flush"
        );
    }

    harness.get_by_label(egui_phosphor::regular::X).click();
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

/// Enter accepts the highlighted candidate, replacing the partial word; a
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

/// With the editor focused, Esc first unfocuses it; only a second Esc closes
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

/// The fixture stretches whose `accel` x-component exceeds 1 g, shared by the
/// value generation and the expected-match assertion so they cannot drift.
const ACCEL_HIGH_RANGES: [std::ops::Range<usize>; 2] = [60..120, 180..200];

/// Synthetic `.gtd` bytes whose track carries an aligned 3-component `accel`
/// channel in g, one sample per nav fix. The [`ACCEL_HIGH_RANGES`] stretches
/// exceed 1 g on x, so an `@accel.x` filter has multi-sample matches to table
/// on the window and halo on the map.
fn accel_channel_gtd_bytes() -> Vec<u8> {
    let spec = SyntheticGtdSpec {
        start: base_time(),
        point_count: 240,
        step_secs: 1,
        start_lat_deg: 55.0,
        start_lon_deg: 12.0,
        lat_step_deg: 0.00005,
        lon_step_deg: 0.00008,
        heading_deg: 20.0,
        speed_kmh: 28.0,
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
    use gt_types::{
        Channel, FileMetadata, FileSource, LoadedFile, LoadedTrack, TrackLod, TrackMetadata,
    };
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
        metadata: FileMetadata::default(),
        tracks: vec![LoadedTrack {
            metadata: TrackMetadata::default(),
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

/// A channel-source query mixed with a points query cannot run; the editor
/// says why instead of leaving a silently dead Run button.
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
        .get_by_label_contains(egui_phosphor::regular::TERMINAL_WINDOW)
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
    let mut state = egui::TextEdit::load_state(&harness.ctx, editor_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(text.chars().count()),
        )));
    egui::TextEdit::store_state(&harness.ctx, editor_id, state);
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

    harness.snapshot("query_autocomplete_popup");
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
    harness.snapshot("query_error");
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

    // Hover the `window` token. It starts after "points | " on the first line;
    // the editor's text begins near the top-left of the window content.
    let editor = harness
        .inner
        .get_by_role(egui::accesskit::Role::MultilineTextInput);
    let rect = editor.rect();
    let hover = egui::pos2(rect.left() + 96.0, rect.top() + 10.0);
    harness.inner.run_steps(2);
    harness.inner.hover_at(hover);
    // The hover doc appears only after the pointer has rested; step past the
    // delay (steps advance the mock clock a frame at a time).
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
    harness.inner.state_mut().pending_history_unlock = Some(PathBuf::from("geotrace.h5"));
    harness.run();
    harness.snapshot("history_locked_dialog");
}

#[test]
fn snapshot_history_corrupt_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(build_app);
    harness.inner.step();
    harness.inner.state_mut().pending_db_corruption = Some(PathBuf::from("geotrace.h5"));
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
                ..gt_types::FileMetadata::default()
            },
            // A long, auto-derived, path-like identity to show the dialog gives
            // it room instead of clipping it.
            identity: Some("auto:/home/user/recordings/2025/05/ride_2025-05-23.gtd".to_owned()),
        });
    harness.run();
    harness.snapshot("recording_details_dialog");
}
