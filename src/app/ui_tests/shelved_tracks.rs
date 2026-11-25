//! Unshelving the tracks of a recording the view holds.

use std::time::{Duration as StdDuration, Instant};

use chrono::Duration;
use egui_kittest::{Harness, kittest::Queryable as _};
use gt_loaded_files::FileHistory;
use gt_log_view::{LoadedLog, RecordingKey};
use gt_side_panel::{NodeKey, ShelveConfirmState};
use gt_store::{DbError, HistoryAccess, TrackRange, TrackState};
use gt_test_utils::HarnessInteraction as _;
use gt_types::{AssociationWindow, FileIdx};

use crate::app::history_db::{DeleteShelvedTracksScope, Response};
use crate::app::modals::{DELETE_PERMANENTLY_BUTTON_LABEL, SHELVE_BUTTON_LABEL};
use crate::app::test_util::recordings;
use crate::app::{App, ui_tests};

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

#[test]
fn unshelving_a_track_of_a_loaded_recording_puts_it_in_the_view() {
    let (mut harness, _dir, db_ref) =
        recordings::app_with_stored_tracks(&one_live_and_one_shelved_track());
    harness.state().history.open(db_ref.clone());
    assert!(
        harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(1)),
        "the recording did not open with its live track"
    );

    harness.state().history.set_tracks_shelved(
        recordings::worker_track_snapshot(&harness.state().history, &db_ref),
        vec![1],
        false,
    );

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
    let (mut harness, _dir, db_ref) =
        recordings::app_with_stored_tracks(&one_live_and_one_shelved_track());

    harness.state().history.set_tracks_shelved(
        recordings::worker_track_snapshot(&harness.state().history, &db_ref),
        vec![1],
        false,
    );

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

#[test]
fn merging_loaded_tracks_cancels_a_pending_shelve_and_clears_its_stored_row_mapping() {
    let stored_rows = ui_tests::two_live_track_ranges();
    let (mut harness, _dir, recording) = recordings::app_with_stored_tracks(&stored_rows);
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(2)));
    {
        let mut shared = harness.state().shared.borrow_mut();
        assert!(
            shared
                .loaded_files
                .view()
                .get(0)
                .unwrap()
                .history()
                .track_snapshot()
                .is_some()
        );
        shared.tree.shelve_confirm = Some(ShelveConfirmState {
            items: vec![NodeKey::File(FileIdx::new(0))],
            delete_permanently: true,
        });
    }
    let app = harness.state_mut();
    app.processing_config.track_layout.track_split_gap = Duration::hours(2);
    app.apply_resegmentation();
    assert_eq!(tracks_of_the_loaded_recording(&harness), Some(1));
    let shared = harness.state().shared.borrow();
    assert!(shared.tree.shelve_confirm.is_none());
    assert!(
        shared
            .loaded_files
            .view()
            .get(0)
            .unwrap()
            .history()
            .track_snapshot()
            .is_none()
    );
    let stored = recordings::worker_track_snapshot(&harness.state().history, &recording);
    assert_eq!(stored.rows(), &stored_rows);
}

#[test]
fn removing_shelved_rows_keeps_the_live_track_addressable_without_reopening() {
    let (mut harness, _dir, recording) =
        recordings::app_with_stored_tracks(&one_live_and_one_shelved_track());
    harness.state().history.open(recording);
    assert!(harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(1)));

    harness
        .state()
        .history
        .delete_shelved_tracks(DeleteShelvedTracksScope::EveryRecording);

    assert!(harness.step_until(|harness| {
        let shared = harness.state().shared.borrow();
        let Some(entry) = shared.loaded_files.view().get(0) else {
            return false;
        };
        entry.history().track_snapshot().is_some_and(|snapshot| {
            snapshot.is_current()
                && snapshot
                    .rows()
                    .get(1)
                    .is_some_and(|row| row.state == TrackState::Deleted)
        })
    }));
    assert_eq!(tracks_of_the_loaded_recording(&harness), Some(1));
}

#[test]
fn a_rejected_delete_keeps_the_recording_and_its_log_loaded() {
    let (mut harness, _dir, recording) =
        recordings::app_with_stored_tracks(&one_live_and_one_shelved_track());
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(1)));

    let parsed = gt_logfile::parse_log(
        gt_test_utils::UNRESOLVED_YEARS_LOG.into(),
        gt_test_utils::synthetic_log_start(),
    )
    .expect("parse log");
    let mut log = LoadedLog::new(
        Some("receiver.log".to_owned()),
        parsed,
        AssociationWindow::try_from_seconds(60).expect("valid association window"),
    );
    let app = harness.state_mut();
    log.anchor_to(
        RecordingKey::Stored(recording.clone()),
        &app.shared.borrow().loaded_files.view(),
    );
    app.logs.push(log);

    let access = HistoryAccess::lock();
    app.history
        .delete_shelved_tracks(DeleteShelvedTracksScope::EveryRecording);
    app.shared.borrow_mut().tree.shelve_confirm = Some(ShelveConfirmState {
        items: vec![NodeKey::File(FileIdx::new(0))],
        delete_permanently: true,
    });
    harness.run_steps(3);
    harness
        .get_by_label(DELETE_PERMANENTLY_BUTTON_LABEL)
        .click();
    harness.run_steps(2);

    assert!(
        harness
            .state()
            .shared
            .borrow()
            .tree
            .shelve_confirm
            .is_none()
    );
    assert_eq!(tracks_of_the_loaded_recording(&harness), Some(1));
    assert_eq!(harness.state().logs.len(), 1);
    drop(access);

    let deadline = Instant::now() + StdDuration::from_secs(5);
    let mut rejected = false;
    while !rejected {
        for response in harness.state().history.poll() {
            rejected |= matches!(
                &response,
                Response::Mutated {
                    result: Err(DbError::StoredTrackLayoutChanged),
                    ..
                }
            );
            harness.state_mut().handle_history_response(response);
        }
        assert!(
            Instant::now() < deadline,
            "delete did not report the stale layout"
        );
        std::thread::sleep(StdDuration::from_millis(5));
    }
    assert_eq!(tracks_of_the_loaded_recording(&harness), Some(1));
    assert_eq!(harness.state().logs.len(), 1);
    let stored = recordings::worker_track_snapshot(&harness.state().history, &recording);
    let states: Vec<_> = stored.rows().iter().map(|row| row.state).collect();
    assert_eq!(states, [TrackState::Live, TrackState::Deleted]);
}

#[test]
fn a_mixed_shelve_selection_lists_and_removes_only_the_stored_recording() {
    let (mut harness, _dir, recording) =
        recordings::app_with_stored_tracks(&one_live_and_one_shelved_track());
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(1)));
    let filename = "session-only.gtd";
    {
        let mut shared = harness.state().shared.borrow_mut();
        let mut file = gt_test_utils::segmented_recording(1);
        file.metadata.filename = filename.to_owned();
        shared.loaded_files.push(file, FileHistory::None);
        shared.sync_tree_from_loaded_files();
        shared.tree.shelve_confirm = Some(ShelveConfirmState {
            items: vec![
                NodeKey::File(FileIdx::new(0)),
                NodeKey::File(FileIdx::new(1)),
            ],
            delete_permanently: false,
        });
    }

    harness.run_steps(3);
    let dialog = harness.get_by_label("Shelve 1 item?");
    assert!(dialog.query_by_label(&recording.identity).is_some());
    assert!(dialog.query_by_label(filename).is_none());
    harness.get_by_label(SHELVE_BUTTON_LABEL).click();
    assert!(harness.step_until(|harness| {
        let shared = harness.state().shared.borrow();
        shared.loaded_files.len() == 1
            && shared
                .loaded_files
                .view()
                .get(0)
                .is_some_and(|entry| entry.file().metadata.filename == filename)
    }));

    let stored = recordings::worker_track_snapshot(&harness.state().history, &recording);
    assert!(
        stored
            .rows()
            .iter()
            .all(|row| row.state == TrackState::Shelved)
    );
}
