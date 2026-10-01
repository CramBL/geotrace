use std::time::{Duration as StdDuration, Instant};

use egui_kittest::{Harness, kittest::Queryable as _};
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use gt_test_utils::HarnessInteraction as _;
use rstest::rstest;

use crate::app::frame::TRACK_DATA_WINDOW_SIZING;
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

#[rstest]
#[case::center(egui::pos2(450.0, 50.0), 1.0, false)]
#[case::scaled_center(egui::pos2(450.0, 50.0), 1.5, false)]
#[case::top_left(egui::pos2(0.0, 0.0), 1.0, false)]
#[case::bottom_right(egui::pos2(899.0, 699.0), 1.5, false)]
#[case::released_before_floating_frame(egui::pos2(450.0, 50.0), 1.0, true)]
fn drag_detach_applies_the_latest_pointer_and_clamps_the_first_visible_rectangle(
    #[case] target: egui::Pos2,
    #[case] scale: f32,
    #[case] released_before_floating_frame: bool,
) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .with_pixels_per_point(scale)
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .set_visible_section_fraction(0.42);
    harness.run_steps(4);
    let grip = harness
        .ctx
        .read_response(egui::Id::new((gt_side_panel::TRACK_DATA_GRIP_ID, false)))
        .expect("docked grip")
        .rect
        .center();
    harness.hover_at(grip);
    harness.step();
    harness.drag_at(grip);
    harness.step();
    harness.hover_at(egui::pos2(500.0, grip.y));
    harness.step();
    assert!(harness.state().shared.borrow().tree.detached);
    let pending = harness
        .state()
        .pending_track_data_detach
        .as_ref()
        .expect("pending drag");
    let grab_offset = pending.drag.press_origin - pending.docked_rect.min;
    let docked_size = pending.docked_rect.size();
    if released_before_floating_frame {
        harness.drop_at(target);
    } else {
        harness.hover_at(target);
    }
    harness.step();
    assert!(harness.state().pending_track_data_detach.is_none());
    let viewport = harness.ctx.content_rect();
    let maximum = viewport.size() * TRACK_DATA_WINDOW_SIZING.maximum_fraction;
    let expected_size = docked_size.min(maximum);
    let expected_position =
        (target - grab_offset).clamp(viewport.min, viewport.max - expected_size);
    let first = harness
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::new("detached_panel")))
        .expect("first floating rectangle");
    assert!(
        (first.min - expected_position).length() < 2.0,
        "first {first:?}, expected {expected_position:?}"
    );
    assert!(
        (first.size() - expected_size).length() < 2.0,
        "first {first:?}, expected size {expected_size:?}"
    );
    assert!(viewport.expand(1.0).contains_rect(first));
    assert!(harness.query_by_label("Dock").is_some());
    let continued = if released_before_floating_frame {
        assert!(harness.ctx.dragged_id().is_none());
        first
    } else {
        harness.hover_at(target + egui::vec2(-30.0, 20.0));
        harness.step();
        let continued = harness
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new("detached_panel")))
            .expect("continued drag");
        if target == egui::pos2(450.0, 50.0) {
            assert!(
                (continued.min - first.min - egui::vec2(-30.0, 20.0)).length() < 2.0,
                "first {first:?}, continued {continued:?}"
            );
        }
        harness.drop_at(target + egui::vec2(-30.0, 20.0));
        continued
    };
    harness.run_steps(4);
    let settled = harness
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::new("detached_panel")))
        .expect("settled rectangle");
    assert!((settled.min - continued.min).length() < 2.0);
    harness.get_by_label("Dock").click();
    harness.run_steps(4);
    assert!(!harness.state().shared.borrow().tree.detached);
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .tree
            .visible_section_fraction()
            .to_bits(),
        0.42_f32.to_bits()
    );
}

#[test]
fn button_pop_out_centers_cold_geometry_and_restores_user_geometry_after_docking() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.run_steps(4);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .set_visible_section_fraction(0.25);
    harness.get_by_label(ICON_ARROW_SQUARE_OUT).click();
    harness.run_steps(5);
    let id = egui::Id::new("detached_panel");
    let initial = harness
        .ctx
        .memory(|memory| memory.area_rect(id))
        .expect("cold window");
    assert!((initial.center() - harness.ctx.content_rect().center()).length() < 2.0);
    assert!((initial.width() - 350.0).abs() < 2.0);
    let titlebar = initial.min + egui::vec2(80.0, 12.0);
    harness.press_drag_release(titlebar, egui::vec2(100.0, -60.0), 4);
    harness.run_steps(3);
    let moved = harness
        .ctx
        .memory(|memory| memory.area_rect(id))
        .expect("moved window");
    assert!((moved.min - initial.min).length() > 50.0);
    let corner = moved.max - egui::vec2(1.0, 1.0);
    harness.press_drag_release(corner, egui::vec2(70.0, 30.0), 4);
    harness.run_steps(3);
    let resized = harness
        .ctx
        .memory(|memory| memory.area_rect(id))
        .expect("resized window");
    assert!(resized.width() > moved.width() + 30.0);
    harness.get_by_label("Dock").click();
    harness.run_steps(4);
    assert!(!harness.state().shared.borrow().tree.detached);
    harness.get_by_label(ICON_ARROW_SQUARE_OUT).click();
    harness.run_steps(5);
    let reopened = harness
        .ctx
        .memory(|memory| memory.area_rect(id))
        .expect("reopened window");
    assert!((reopened.min - resized.min).length() < 2.0);
    assert!((reopened.size() - resized.size()).length() < 2.0);
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .tree
            .visible_section_fraction()
            .to_bits(),
        0.25_f32.to_bits()
    );
}
