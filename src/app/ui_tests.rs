use std::{
    env, fs,
    sync::Arc,
    thread,
    time::{Duration as StdDuration, Instant},
};

use egui_kittest::Harness;
use naview_sdk::{Angle, DateTime, Duration, NavFileBuilder, NavFix, Utc, degree};

use super::App;

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("fixed timestamp is valid")
}

fn minimal_nvd_bytes() -> Vec<u8> {
    let mut builder = NavFileBuilder::new();
    let t0 = base_time();
    let t1 = t0 + Duration::seconds(60);
    builder.add_nav_fix(
        NavFix::builder()
            .gps_time(t0)
            .lat(Angle::new::<degree>(51.5))
            .lon(Angle::new::<degree>(-0.1))
            .heading(Angle::new::<degree>(270.0))
            .build(),
    );
    builder.add_nav_fix(
        NavFix::builder()
            .gps_time(t1)
            .lat(Angle::new::<degree>(51.6))
            .lon(Angle::new::<degree>(-0.2))
            .heading(Angle::new::<degree>(90.0))
            .build(),
    );
    let nav_file = builder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).expect("write succeeds");
    bytes
}

/// Step the harness repeatedly until all background load jobs have finished.
///
/// Background threads send a `Completed` message when done; `drain_load_channel`
/// (called at the start of every `ui()` frame) picks it up and removes the job.
/// We sleep briefly between steps to let the threads make progress.
fn step_until_loaded(harness: &mut Harness<App>) {
    for _ in 0..200 {
        if harness.state().loading_jobs.is_empty() {
            return;
        }
        thread::sleep(StdDuration::from_millis(5));
        harness.step();
    }
    // One final step in case the last drain hadn't run yet.
    harness.step();
}

#[test]
fn drag_drop_nvd_path_loads_file() {
    let nvd_bytes = minimal_nvd_bytes();
    let tmp = env::temp_dir().join("naview_test_drag_drop_path.nvd");
    fs::write(&tmp, &nvd_bytes).expect("write temp nvd");

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(|cc| App::new(cc));
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        path: Some(tmp.clone()),
        ..Default::default()
    });
    harness.step(); // processes the drop, spawns load thread
    step_until_loaded(&mut harness); // waits for thread + drains channel

    fs::remove_file(&tmp).ok();
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
}

#[test]
fn drag_drop_nvd_bytes_loads_file() {
    let nvd_bytes = minimal_nvd_bytes();

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(|cc| App::new(cc));
    harness.input_mut().dropped_files.push(egui::DroppedFile {
        bytes: Some(Arc::from(nvd_bytes.as_slice())),
        name: "test.nvd".to_owned(),
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
        .build_eframe(|cc| App::new(cc));
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
        .build_eframe(|cc| App::new(cc));
    harness.step();
    assert!(!harness.state().shared.borrow().panel.detached);

    harness.state_mut().shared.borrow_mut().panel.detached = true;
    harness.step();
    assert!(harness.state().shared.borrow().panel.detached);
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
/// *same* OS window, so there is only one Wayland surface — the compositor
/// cannot suspend it independently of the main window.
///
/// # What this test checks
///
/// `egui_kittest` is headless; it cannot trigger the real Wayland deadlock.
/// What it *can* do is verify that the detached panel code path completes
/// each frame quickly and does not introduce any O(n²) loops or accidentally
/// blocking operations that would manifest even in a headless runner.
/// If a future change re-introduces a blocking call, this test will time out.
#[test]
fn detached_panel_steps_complete_within_time_budget() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(|cc| App::new(cc));
    harness.step();

    harness.state_mut().shared.borrow_mut().panel.detached = true;

    // 50 consecutive steps must all finish within 10 seconds total.
    // In a healthy headless runner each step takes well under 1 ms; the
    // budget is generous to survive slow CI machines.
    let deadline = Instant::now() + StdDuration::from_secs(10);
    for _ in 0..50 {
        assert!(
            Instant::now() < deadline,
            "step deadline exceeded — likely a blocking call in the detached panel render path"
        );
        harness.step();
    }

    // Docking must also work cleanly after repeated detached rendering.
    harness.state_mut().shared.borrow_mut().panel.detached = false;
    harness.step();
    assert!(!harness.state().shared.borrow().panel.detached);
}
