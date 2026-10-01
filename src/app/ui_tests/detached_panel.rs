use std::time::{Duration as StdDuration, Instant};

use egui_kittest::Harness;

use crate::app::test_util;

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

#[test]
fn visible_section_fraction_and_geometry_survive_dock_detach_dock() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 900.0))
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .set_visible_section_fraction(0.5);
    harness.run_steps(4);
    let docked_id = egui::Id::new(("visible_tracks_section", false));
    let detached_id = egui::Id::new(("visible_tracks_section", true));
    let docked = harness
        .ctx
        .read_response(docked_id)
        .expect("docked section")
        .rect;
    harness.state_mut().shared.borrow_mut().tree.detached = true;
    harness.run_steps(4);
    let detached = harness
        .ctx
        .read_response(detached_id)
        .expect("detached section")
        .rect;
    assert!(detached.height() < docked.height());
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .tree
            .visible_section_fraction()
            .to_bits(),
        0.5_f32.to_bits()
    );
    harness.set_size(egui::vec2(900.0, 600.0));
    harness.run_steps(4);
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .tree
            .visible_section_fraction()
            .to_bits(),
        0.5_f32.to_bits()
    );
    harness.state_mut().shared.borrow_mut().tree.detached = false;
    harness.run_steps(4);
    let restored = harness
        .ctx
        .read_response(docked_id)
        .expect("restored section")
        .rect;
    assert!((docked.height() - restored.height() - 150.0).abs() < 1.0);
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .tree
            .visible_section_fraction()
            .to_bits(),
        0.5_f32.to_bits()
    );
    harness.set_size(egui::vec2(900.0, 900.0));
    harness.run_steps(4);
    let enlarged = harness
        .ctx
        .read_response(docked_id)
        .expect("enlarged section")
        .rect;
    assert!((enlarged.height() - docked.height()).abs() < 1.0);
}
