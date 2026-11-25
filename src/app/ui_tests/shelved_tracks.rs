//! Unshelving the tracks of a recording the view holds.

use std::time::{Duration as StdDuration, Instant};

use chrono::Duration;
use egui_kittest::{Harness, kittest::Queryable as _};
use geotrace_sdk::{Angle, NavFileBuilder, NavFix, NavFixTime};
use gt_loaded_files::{FileHistory, RecordingLoadOrigin};
use gt_log_view::{LoadedLog, RecordingKey};
use gt_side_panel::{CheckState, NodeKey, ShelveConfirmState, TreeState};
use gt_store::{
    DatabaseRef, DbError, HistoryAccess, HistoryDatabase as _, ReadOnlyHistoryDatabase as _,
    RecordingUiState, Store, StoredTrackSnapshot, TrackRange, TrackState, UiStateVersionReporter,
};
use gt_test_utils::HarnessInteraction as _;
use gt_types::{AssociationWindow, FileIdx, LoadedFile, TrackIdx, TrackRef};
use tempfile::TempDir;

use crate::app::history_db::{DbOp, DeleteShelvedTracksScope, Response};
use crate::app::modals::{self, DELETE_PERMANENTLY_BUTTON_LABEL, SHELVE_BUTTON_LABEL};
use crate::app::test_util::harness;
use crate::app::test_util::recordings;
use crate::app::{App, SharedAppState, loader, ui_tests};

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

#[derive(Clone, Copy)]
enum VisibilityEdit {
    ExpiredRead,
    ExpiredReadWithEdit,
    QueuedBehindPurge,
    SavedBeforePurge,
    WhileAwaitingPurgeReply,
}

impl VisibilityEdit {
    fn has_expired_read(self) -> bool {
        matches!(self, Self::ExpiredRead | Self::ExpiredReadWithEdit)
    }
}

fn app_with_two_live_tracks_and_a_shelved_predecessor()
-> (Harness<'static, App>, TempDir, DatabaseRef) {
    const FIXES_PER_TRACK: u64 = 5;
    let mut recorder = NavFileBuilder::new().open();
    let mut rows = Vec::new();
    for (track, hour) in [0, 1, 2].into_iter().enumerate() {
        let start = track as u64 * FIXES_PER_TRACK;
        let end = start + FIXES_PER_TRACK;
        rows.push(TrackRange {
            start,
            end,
            state: if track == 0 {
                TrackState::Shelved
            } else {
                TrackState::Live
            },
        });
        for point in start..end {
            let time =
                ui_tests::base_time() + Duration::hours(hour) + Duration::seconds(point as i64);
            recorder.add_nav_fix(
                NavFix::builder()
                    .time(NavFixTime::Receiver(time))
                    .lat(Angle::degrees(51.5 + point as f64 * 0.0002))
                    .lon(Angle::degrees(-0.1))
                    .heading(Angle::degrees(270.0))
                    .build(),
            );
        }
    }
    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("valid recording")
        .write(&mut bytes)
        .expect("write recording");
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open_in(dir.path());
    let recording = recordings::store_recording(&store.recordings_path(), &bytes, &rows);
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    harness.step();
    harness::send_open_storage_and_step(&mut harness, &databases, &store);
    (harness, dir, recording)
}

fn wait_for_purge_to_invalidate(snapshot: &StoredTrackSnapshot) {
    let deadline = Instant::now() + StdDuration::from_secs(5);
    while snapshot.is_current() {
        assert!(
            Instant::now() < deadline,
            "purge did not invalidate the snapshot"
        );
        std::thread::sleep(StdDuration::from_millis(1));
    }
}

#[rstest::rstest]
#[case::load_completion_first(false)]
#[case::purge_reply_first(true)]
fn the_tree_uses_cached_visibility_for_a_late_history_load(#[case] purge_reply_first: bool) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let recording =
        recordings::store_recording_with_shelved_middle_track(&store.recordings_path(), None);
    store
        .open_recordings()
        .unwrap()
        .set_recording_ui_state(
            &recording,
            &RecordingUiState::with_hidden_track_numbers([3]),
            &UiStateVersionReporter::default(),
        )
        .unwrap();
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    harness.step();
    harness::send_open_storage_and_step(&mut harness, &databases, &store);
    harness
        .state_mut()
        .processing_config
        .generated_markers
        .detect_clock_discontinuities = false;
    let original_config = harness.state().processing_config;
    let wait_for_load = |app: &mut App| {
        let deadline = Instant::now() + StdDuration::from_secs(5);
        loop {
            for response in app.history.poll() {
                app.handle_history_response(response);
            }
            if let Some(completed) = app.loader.drain().pop() {
                break completed;
            }
            assert!(Instant::now() < deadline, "the History load did not finish");
            std::thread::sleep(StdDuration::from_millis(1));
        }
    };
    harness.state().history.open(recording.clone());
    let completed = wait_for_load(harness.state_mut());
    let loader::LoadCompletion::Recording {
        result: Ok(result), ..
    } = &completed.completion
    else {
        panic!("expected a recording load");
    };
    let row_fixes = |file: &LoadedFile| {
        file.tracks
            .iter()
            .map(|track| {
                let fixes: Vec<_> = track
                    .points
                    .iter()
                    .map(|fix| (fix.tpv.time(), fix.tpv.lat(), fix.tpv.lon()))
                    .collect();
                (track.metadata.index, fixes)
            })
            .collect::<Vec<_>>()
    };
    let original_rows = row_fixes(&result.file);
    assert_eq!(
        original_rows
            .iter()
            .map(|(row, _)| *row)
            .collect::<Vec<_>>(),
        [1, 3],
    );
    let surviving_fixes: Vec<_> = result
        .file
        .tracks
        .iter()
        .flat_map(|track| track.points.iter().cloned())
        .collect();
    assert_eq!(
        gt_track_builder::segment_tracks(&surviving_fixes, &original_config.track_layout).len(),
        1,
    );
    let predecessor = result.history.track_snapshot().unwrap();
    assert!(predecessor.is_current_with_membership());
    harness
        .state()
        .history
        .delete_shelved_tracks(DeleteShelvedTracksScope::OneRecording(recording));
    self::wait_for_purge_to_invalidate(predecessor);
    if purge_reply_first {
        let purged = recordings::next_response(&harness.state().history);
        assert!(matches!(
            purged,
            Response::Mutated {
                op: DbOp::TracksDeleted { count: 1, .. },
                result: Ok(()),
            }
        ));
        harness.state_mut().handle_history_response(purged);
    }
    let app = harness.state_mut();
    app.processing_config.track_layout.track_split_gap = Duration::days(1);
    app.processing_config
        .generated_markers
        .detect_clock_discontinuities = true;
    app.handle_completed_load(completed, 0.0);
    assert!(app.shared.borrow().loaded_files.is_empty());

    let reopened = wait_for_load(app);
    let settings = reopened.stored_history.as_ref().unwrap().settings;
    assert_eq!(settings.config, original_config);
    assert_eq!(settings.mode, loader::GtdLoadMode::Regular);
    assert!(settings.applied_current_marker_settings);
    app.handle_completed_load(reopened, 0.0);

    let shared = harness.state().shared.borrow();
    assert_eq!(shared.loaded_files.len(), 1);
    let entry = shared.loaded_files.view().get(0).unwrap();
    assert_eq!(row_fixes(entry.file()), original_rows);
    let snapshot = entry.history().track_snapshot().unwrap();
    assert!(snapshot.is_current_with_membership());
    assert_eq!(
        snapshot
            .rows()
            .iter()
            .map(|row| (row.start..row.end, row.state))
            .collect::<Vec<_>>(),
        [
            (0..5, TrackState::Live),
            (5..5, TrackState::Deleted),
            (5..10, TrackState::Live),
        ],
    );
    let hidden = TrackRef::new(FileIdx::new(0), TrackIdx::new(1));
    assert_eq!(
        shared.tree.track_node(hidden).unwrap().check,
        CheckState::Off
    );
}

#[rstest::rstest]
#[case::saved(VisibilityEdit::SavedBeforePurge)]
#[case::queued(VisibilityEdit::QueuedBehindPurge)]
#[case::awaiting_reply(VisibilityEdit::WhileAwaitingPurgeReply)]
#[case::expired_read(VisibilityEdit::ExpiredRead)]
#[case::expired_read_with_edit(VisibilityEdit::ExpiredReadWithEdit)]
fn a_hidden_surviving_track_keeps_its_visibility_after_purge_and_reopen(
    #[case] edit: VisibilityEdit,
) {
    let (mut harness, dir, recording) = self::app_with_two_live_tracks_and_a_shelved_predecessor();
    let store = Store::open_in(dir.path());
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let sibling = TrackRef::new(track.fi, TrackIdx::new(1));
    let hidden_on_disk = RecordingUiState::with_hidden_track_numbers([2]);
    let (hidden, sibling_check) = if matches!(edit, VisibilityEdit::ExpiredReadWithEdit) {
        (
            RecordingUiState::with_hidden_track_numbers([2, 3]),
            CheckState::Off,
        )
    } else {
        (hidden_on_disk.clone(), CheckState::On)
    };
    let reporter = UiStateVersionReporter::default();
    let saved = || {
        store
            .open_recordings_read_only()
            .unwrap()
            .unwrap()
            .recording_ui_state(&recording, &reporter)
            .unwrap()
    };
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(2)));
    let (initial, unverified_id) = {
        let mut shared = harness.state().shared.borrow_mut();
        if edit.has_expired_read() {
            shared.tree = TreeState::default();
        }
        let entry = shared.loaded_files.view().get(0).unwrap();
        let initial = entry.history().track_snapshot().unwrap().clone();
        let (copy, history) = (
            entry.file().clone(),
            entry
                .history()
                .clone()
                .with_load_origin(RecordingLoadOrigin::Import)
                .with_track_snapshot(None),
        );
        shared.loaded_files.push(copy, history);
        shared.sync_tree_from_loaded_files();
        (initial, shared.loaded_files.view().get(1).unwrap().id())
    };
    match edit {
        VisibilityEdit::SavedBeforePurge => {
            harness.state().shared.borrow_mut().tree.hide_track(track);
            assert!(harness.step_until(|_| saved() == hidden));
        }
        VisibilityEdit::ExpiredRead | VisibilityEdit::ExpiredReadWithEdit => {
            store
                .open_recordings()
                .unwrap()
                .set_recording_ui_state(&recording, &hidden_on_disk, &reporter)
                .unwrap();
            harness
                .state()
                .read_recording_ui_state_if_unobserved(Some(initial.layout()));
        }
        VisibilityEdit::QueuedBehindPurge | VisibilityEdit::WhileAwaitingPurgeReply => {}
    }
    {
        let _access = HistoryAccess::lock();
        harness
            .state()
            .history
            .delete_shelved_tracks(DeleteShelvedTracksScope::OneRecording(recording.clone()));
        if matches!(edit, VisibilityEdit::QueuedBehindPurge) {
            harness.state().shared.borrow_mut().tree.hide_track(track);
            harness.state().store_the_hidden_tracks_the_user_changed();
        }
    }
    if matches!(edit, VisibilityEdit::WhileAwaitingPurgeReply) || edit.has_expired_read() {
        self::wait_for_purge_to_invalidate(&initial);
        let mut shared = harness.state().shared.borrow_mut();
        match edit {
            VisibilityEdit::WhileAwaitingPurgeReply => shared.tree.hide_track(track),
            VisibilityEdit::ExpiredReadWithEdit => {
                let SharedAppState {
                    loaded_files, tree, ..
                } = &mut *shared;
                tree.hide_track(sibling);
                modals::remove_items_from_view(&[NodeKey::Track(sibling)], loaded_files, tree);
            }
            _ => {}
        }
    }
    assert!(harness.step_until(|harness| {
        let shared = harness.state().shared.borrow();
        let entry = shared.loaded_files.view().get(0).unwrap();
        shared.tree.track_node(track).unwrap().check == CheckState::Off
            && entry.history().track_snapshot().is_some_and(|snapshot| {
                snapshot.is_current()
                    && snapshot
                        .rows()
                        .first()
                        .is_some_and(|row| row.state == TrackState::Deleted)
            })
    }));
    {
        let mut shared = harness.state().shared.borrow_mut();
        shared.sync_tree_from_loaded_files();
        assert_eq!(
            shared.tree.track_node(track).unwrap().check,
            CheckState::Off
        );
        assert_eq!(
            shared.tree.track_node(sibling).map(|node| node.check),
            (!matches!(edit, VisibilityEdit::ExpiredReadWithEdit)).then_some(sibling_check),
        );
        assert_eq!(shared.loaded_files.files()[0].tracks[0].metadata.index, 2);
        let unverified = shared
            .loaded_files
            .view()
            .entry_for_id(unverified_id)
            .unwrap();
        assert_eq!(unverified.history().db_ref(), Some(&recording));
        assert!(unverified.history().track_snapshot().is_none());
        let unverified_track = TrackRef::new(FileIdx::new(1), track.index);
        assert_eq!(
            shared.tree.track_node(unverified_track).unwrap().check,
            CheckState::On
        );
    }
    assert!(harness.step_until(|_| saved() == hidden));
    harness
        .state()
        .shared
        .borrow_mut()
        .tree
        .toggle_track_check(track);
    let first_shown = RecordingUiState::with_hidden_track_numbers(
        hidden
            .hidden_track_numbers()
            .iter()
            .copied()
            .filter(|&number| number != 2),
    );
    assert!(harness.step_until(|_| saved() == first_shown));
    harness.state().shared.borrow_mut().tree.hide_track(track);
    assert!(harness.step_until(|_| saved() == hidden));

    drop(harness);
    let (mut reopened, databases) = harness::app_with_the_databases_still_opening(&[]);
    reopened.step();
    harness::send_open_storage_and_step(&mut reopened, &databases, &store);
    reopened.state().history.open(recording);
    assert!(reopened.step_until(|harness| tracks_of_the_loaded_recording(harness) == Some(2)));
    let shared = reopened.state().shared.borrow();
    assert_eq!(
        shared.tree.track_node(sibling).unwrap().check,
        sibling_check
    );
    assert_eq!(
        shared.tree.track_node(track).unwrap().check,
        CheckState::Off
    );
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
