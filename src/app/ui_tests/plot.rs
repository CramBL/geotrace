use egui_kittest::{Harness, kittest::Queryable as _};
use egui_phosphor::regular::ARROW_LINE_UP_LEFT as ICON_ARROW_LINE_UP_LEFT;
use egui_phosphor::regular::DOTS_SIX as ICON_DOTS_SIX;
use gt_test_utils::HarnessInteraction as _;
use rstest::rstest;

use crate::app::App;
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

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
        TestDroppedFile::bytes(ui_tests::minimal_gtd_bytes(), "test.gtd"),
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
    let mut harness = ui_tests::harness_with_three_files_loaded();
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
    let mut harness = ui_tests::harness_with_three_files_loaded();

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
    let mut harness = ui_tests::harness_with_three_files_loaded();
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
