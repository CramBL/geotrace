use egui_kittest::kittest::Queryable as _;
use gt_types::{FileIdx, TrackIdx, TrackRef};

use crate::app::ui_tests;

/// The plot's file legend names recordings through the user's template, the
/// same source the side panel rows read - never the raw filename.
#[test]
fn plot_legend_follows_the_recording_name_template() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
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
        ui_tests::node_outside_the_side_panel(&harness, name);
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
    let mut harness = ui_tests::harness_with_three_files_loaded();
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
