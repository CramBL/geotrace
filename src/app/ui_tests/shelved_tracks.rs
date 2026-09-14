//! Unshelving the tracks of a recording the view holds.

use egui_kittest::Harness;
use gt_store::{TrackRange, TrackState};
use gt_test_utils::HarnessInteraction as _;

use crate::app::{App, loader, ui_tests};

/// The stored track table of [`ui_tests::two_track_gtd_bytes`] with its second
/// track shelved.
fn one_live_and_one_shelved_track() -> [TrackRange; 2] {
    let [live, mut shelved] = ui_tests::two_live_track_ranges();
    shelved.state = TrackState::Shelved;
    [live, shelved]
}

/// The tracks the view holds for the one recording it has loaded.
fn tracks_of_the_loaded_recording(harness: &Harness<'_, App>) -> Option<usize> {
    let state = harness.state();
    let shared = state.shared.borrow();
    shared
        .loaded_files
        .files()
        .first()
        .map(|file| file.tracks.len())
}

/// A session with the history database holding one recording of a live and a
/// shelved track, and the reference it is stored under.
fn app_with_a_recording_holding_a_shelved_track<'a>()
-> (Harness<'a, App>, tempfile::TempDir, gt_store::DatabaseRef) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let db_ref = ui_tests::insert_recording_into(
        &store,
        &ui_tests::two_track_gtd_bytes(),
        &one_live_and_one_shelved_track(),
        loader::stored_segmentation_from_config(&gt_track_builder::SegmentationConfig::default()),
    );
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.step();
    ui_tests::land_the_databases(&mut harness, &databases, &store);
    (harness, dir, db_ref)
}

#[test]
fn unshelving_a_track_of_a_loaded_recording_puts_it_in_the_view() {
    let (mut harness, _dir, db_ref) = app_with_a_recording_holding_a_shelved_track();
    harness.state().history.open(db_ref.clone());
    assert!(
        harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(1)),
        "the recording did not open with its live track"
    );

    harness
        .state()
        .history
        .set_tracks_shelved(db_ref, vec![1], false);

    assert!(
        harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(2)),
        "the unshelved track did not reach the loaded recording"
    );
    assert_eq!(
        harness.state().shared.borrow().loaded_files.len(),
        1,
        "the recording was loaded a second time beside the entry it had"
    );
}

#[test]
fn unshelving_a_track_of_a_recording_that_is_not_loaded_loads_nothing() {
    let (mut harness, _dir, db_ref) = app_with_a_recording_holding_a_shelved_track();

    harness
        .state()
        .history
        .set_tracks_shelved(db_ref, vec![1], false);

    assert!(
        harness.step_until(|harness| harness.state().toasts.len() == 1),
        "the unshelve reported nothing"
    );
    assert!(
        harness.state().shared.borrow().loaded_files.is_empty(),
        "the unshelve loaded the recording"
    );
    assert!(
        harness.state().loader.loading_jobs.is_empty(),
        "the unshelve started a load"
    );
}
