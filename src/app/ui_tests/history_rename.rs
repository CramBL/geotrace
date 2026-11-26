use std::time::{Duration, Instant};

use egui_kittest::Harness;
use gt_loaded_files::{FileHistory, RecordingLoadOrigin};
use gt_side_panel::{CheckState, TreeState};
use gt_store::{
    DatabaseRef, HistoryAccess, HistoryDatabase as _, HistoryRenameOutcome,
    ReadOnlyHistoryDatabase as _, RecordingUiState, Store, StoredTrackSnapshot, TrackRange,
    TrackState, UiStateVersionReporter,
};
use gt_test_utils::{HarnessInteraction as _, recording_fixtures};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use rstest::rstest;

use crate::app::association_batches::RecordingOperationOrigin;
use crate::app::history_db::{
    DbOp, HistoryWorker, LoadedTrackRemoval, RecordingTrackRemoval, Response, StoredTrackAction,
};
use crate::app::test_util::{harness, recordings};
use crate::app::{App, loader, ui_tests};

fn replies_through_listing(worker: &HistoryWorker) -> Vec<Response> {
    let barrier = worker.list().unwrap();
    let mut replies = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        replies.extend(worker.poll());
        if replies
            .iter()
            .any(|reply| matches!(reply, Response::Listed { request, .. } if *request == barrier))
        {
            return replies;
        }
        assert!(
            Instant::now() < deadline,
            "the History worker did not list recordings"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn completed_renames(replies: &[Response]) -> impl Iterator<Item = &HistoryRenameOutcome> {
    replies.iter().filter_map(|reply| match reply {
        Response::Mutated {
            op: DbOp::IdentityRenamed { outcome, .. },
            result,
        } => {
            assert!(result.is_ok(), "History rename failed: {result:?}");
            outcome.as_ref()
        }
        _ => None,
    })
}

fn single_rename_successor(replies: &[Response]) -> &StoredTrackSnapshot {
    let [outcome] = self::completed_renames(replies).collect::<Vec<_>>()[..] else {
        panic!("expected one successful rename");
    };
    let [update] = outcome.snapshot_updates() else {
        panic!("expected one verified layout update");
    };
    update.after()
}

fn apply_single_rename(app: &mut App) -> StoredTrackSnapshot {
    let replies = self::replies_through_listing(&app.history);
    let successor = self::single_rename_successor(&replies).clone();
    for reply in replies {
        app.handle_history_response(reply);
    }
    successor
}

fn saved_visibility(store: &Store, recording: &DatabaseRef) -> RecordingUiState {
    store
        .open_recordings_read_only()
        .unwrap()
        .unwrap()
        .recording_ui_state(recording, &UiStateVersionReporter::default())
        .unwrap()
}

fn open_recording(harness: &mut Harness<'_, App>, recording: &DatabaseRef) {
    let position = harness.state().shared.borrow().loaded_files.len();
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| {
        let shared = harness.state().shared.borrow();
        shared
            .loaded_files
            .view()
            .get(position)
            .and_then(|entry| entry.history().db_ref())
            == Some(recording)
    }));
}

fn reopen_recordings_with_checks(
    store: &Store,
    recordings: &[(DatabaseRef, &[CheckState])],
) -> Harness<'static, App> {
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    harness.step();
    harness::send_open_storage_and_step(&mut harness, &databases, store);
    for (position, (recording, checks)) in recordings.iter().enumerate() {
        self::open_recording(&mut harness, recording);
        let shared = harness.state().shared.borrow();
        let entry = shared.loaded_files.view().get(position).unwrap();
        assert_eq!(entry.file().tracks.len(), checks.len());
        for (index, check) in checks.iter().enumerate() {
            let track = TrackRef::new(entry.file_index(), TrackIdx::new(index));
            assert_eq!(shared.tree.track_node(track).unwrap().check, *check);
        }
    }
    harness
}

#[test]
fn history_rename_rebinds_only_the_loaded_recording_with_the_observed_predecessor() {
    let (mut harness, _dir, recording) =
        recordings::app_with_stored_tracks(&ui_tests::two_live_track_ranges());
    self::open_recording(&mut harness, &recording);
    let checked_id = {
        let mut shared = harness.state().shared.borrow_mut();
        let entry = shared.loaded_files.view().get(0).unwrap();
        assert_ne!(entry.identity(), Some(recording.identity.as_str()));
        let id = entry.id();
        let mut unverified = entry
            .history()
            .clone()
            .with_load_origin(RecordingLoadOrigin::Import)
            .with_track_snapshot(None);
        let FileHistory::Recording { identity, .. } = &mut unverified else {
            panic!("expected recording history");
        };
        identity.clone_from(&recording.identity);
        let file = entry.file().clone();
        shared.loaded_files.push(file, unverified);
        shared.sync_tree_from_loaded_files();
        id
    };
    let app = harness.state_mut();
    app.history
        .rename_identity(recording.identity.clone(), "renamed".to_owned());
    let successor = self::apply_single_rename(app);

    let shared = app.shared.borrow();
    let view = shared.loaded_files.view();
    let checked = view.entry_for_id(checked_id).unwrap();
    assert_eq!(checked.identity(), Some("renamed"));
    assert_eq!(checked.history().db_ref(), Some(successor.recording()));
    let snapshot = checked.history().track_snapshot().unwrap();
    assert!(snapshot.same_layout_and_payload(&successor));
    assert!(snapshot.is_current_with_membership());
    assert_eq!(snapshot.rows(), &ui_tests::two_live_track_ranges());
    let disk_copy = view.get(1).unwrap().history();
    assert_eq!(disk_copy.db_ref(), Some(&recording));
    assert!(disk_copy.track_snapshot().is_none());
}

#[test]
fn shelving_the_destination_before_delivery_preserves_the_rename() {
    let (mut harness, _dir, recording) =
        recordings::app_with_stored_tracks(&ui_tests::two_live_track_ranges());
    self::open_recording(&mut harness, &recording);
    let app = harness.state_mut();
    let file = app.shared.borrow().loaded_files.view().get(0).unwrap().id();
    app.history
        .rename_identity(recording.identity, "renamed".to_owned());
    let rename_replies = self::replies_through_listing(&app.history);
    let successor = self::single_rename_successor(&rename_replies);
    app.history.remove_loaded_tracks(
        StoredTrackAction::Shelve,
        RecordingTrackRemoval::from_loaded(
            successor.clone(),
            vec![LoadedTrackRemoval::new(file, vec![0])],
        ),
    );
    let shelve_replies = self::replies_through_listing(&app.history);
    assert!(shelve_replies.iter().any(|reply| matches!(
        reply,
        Response::Mutated {
            op: DbOp::TracksShelved { count: 1, .. },
            result: Ok(()),
        }
    )));
    assert!(successor.is_current());
    assert!(!successor.is_current_with_membership());
    let recording = successor.recording().clone();
    for reply in rename_replies.into_iter().chain(shelve_replies) {
        app.handle_history_response(reply);
    }

    let actual = recordings::worker_track_snapshot(&app.history, &recording);
    let shared = app.shared.borrow();
    let entry = shared.loaded_files.view().entry_for_id(file).unwrap();
    assert_eq!(entry.history().db_ref(), Some(&recording));
    assert_eq!(entry.file().tracks.len(), 1);
    assert_eq!(entry.file().tracks[0].metadata.index, 2);
    let snapshot = entry.history().track_snapshot().unwrap();
    assert!(snapshot.same_layout_and_payload(&actual));
    assert!(!snapshot.is_current_with_membership());
    assert!(actual.is_current_with_membership());
    assert_eq!(actual.rows()[0].state, TrackState::Shelved);
}

fn load_disk_copy_before_history_replies(app: &mut App, expected: &StoredTrackSnapshot) {
    let position = app.shared.borrow().loaded_files.len();
    app.loader.spawn_gtd_bytes(
        recording_fixtures::two_track_gtd_bytes().into(),
        "copy.gtd".to_owned(),
        app.processing_config,
        loader::GtdLoadMode::Regular,
        None,
        RecordingOperationOrigin::Independent,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let completed = loop {
        if let Some(completed) = app.loader.drain().pop() {
            break completed;
        }
        assert!(Instant::now() < deadline, "the disk copy did not load");
        std::thread::sleep(Duration::from_millis(5));
    };
    app.handle_completed_load(completed, 0.0);
    let shared = app.shared.borrow();
    let entry = shared.loaded_files.view().get(position).unwrap();
    assert_eq!(
        entry.history().load_origin(),
        Some(RecordingLoadOrigin::Import)
    );
    assert!(
        entry
            .history()
            .track_snapshot()
            .unwrap()
            .same_layout_and_payload(expected)
    );
}

#[derive(Clone, Copy)]
enum CheckboxEdits {
    DistinctTracks,
    PredecessorLast,
    SuccessorLast,
    SavedPredecessorVisibility,
}

impl CheckboxEdits {
    fn apply(self, tree: &mut TreeState) {
        let original_first = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let original_second = TrackRef::new(original_first.fi, TrackIdx::new(1));
        let copy_second = TrackRef::new(FileIdx::new(1), TrackIdx::new(1));
        match self {
            Self::DistinctTracks => {
                tree.hide_track(original_first);
                tree.hide_track(copy_second);
            }
            Self::PredecessorLast => {
                tree.hide_track(copy_second);
                tree.toggle_track_check(copy_second);
                tree.hide_track(original_second);
            }
            Self::SuccessorLast => {
                tree.hide_track(original_second);
                tree.hide_track(copy_second);
                tree.toggle_track_check(copy_second);
            }
            Self::SavedPredecessorVisibility => tree.hide_track(original_second),
        }
    }
}

#[rstest]
#[case::distinct_tracks(CheckboxEdits::DistinctTracks, &[1, 2], &[CheckState::Off, CheckState::Off])]
#[case::predecessor_last(CheckboxEdits::PredecessorLast, &[2], &[CheckState::On, CheckState::Off])]
#[case::successor_last(CheckboxEdits::SuccessorLast, &[], &[CheckState::On, CheckState::On])]
#[case::saved_predecessor_visibility(CheckboxEdits::SavedPredecessorVisibility, &[1, 2], &[CheckState::Off, CheckState::Off])]
fn checkbox_edits_across_a_rename_follow_the_latest_action(
    #[case] edits: CheckboxEdits,
    #[case] hidden: &[u64],
    #[case] checks: &[CheckState],
) {
    let (mut harness, dir, recording) =
        recordings::app_with_stored_tracks(&ui_tests::two_live_track_ranges());
    let store = Store::open_in(dir.path());
    let saved_predecessor = if matches!(edits, CheckboxEdits::SavedPredecessorVisibility) {
        let saved = RecordingUiState::with_hidden_track_numbers([1]);
        store
            .open_recordings()
            .unwrap()
            .set_recording_ui_state(&recording, &saved, &UiStateVersionReporter::default())
            .unwrap();
        saved
    } else {
        RecordingUiState::default()
    };
    self::open_recording(&mut harness, &recording);
    let app = harness.state_mut();
    app.history
        .rename_identity(recording.identity, "renamed".to_owned());
    let rename_replies = self::replies_through_listing(&app.history);
    let successor = self::single_rename_successor(&rename_replies);
    let destination = successor.recording().clone();
    self::load_disk_copy_before_history_replies(app, successor);
    let mut read_replies = self::replies_through_listing(&app.history);
    assert!(read_replies.iter().any(|reply| matches!(reply,
        Response::RecordingUiStateLoaded(Ok((layout, state)))
            if layout.same_observation(successor.layout()) && *state == saved_predecessor
    )));
    if matches!(edits, CheckboxEdits::PredecessorLast) {
        for reply in read_replies.drain(..) {
            app.handle_history_response(reply);
        }
    }
    edits.apply(&mut app.shared.borrow_mut().tree);
    app.store_the_hidden_tracks_the_user_changed();
    for reply in rename_replies.into_iter().chain(read_replies) {
        app.handle_history_response(reply);
    }
    {
        let copy_first = TrackRef::new(FileIdx::new(1), TrackIdx::new(0));
        let shared = app.shared.borrow();
        assert_eq!(shared.tree.track_node(copy_first).unwrap().check, checks[0]);
        assert_eq!(
            shared.tree.visibility().files[1].tracks[0].enabled,
            checks[0] == CheckState::On
        );
    }
    let expected = RecordingUiState::with_hidden_track_numbers(hidden.iter().copied());
    let saved = self::replies_through_listing(&app.history);
    assert!(
        saved
            .iter()
            .any(|reply| matches!(reply, Response::RecordingUiStateStored(Ok(()))))
    );
    assert_eq!(self::saved_visibility(&store, &destination), expected);
    drop(harness);
    self::reopen_recordings_with_checks(&store, &[(destination, checks)]);
}

#[test]
fn processing_after_history_moves_a_recording_clears_its_stored_visibility() {
    let (mut harness, dir, recording) =
        recordings::app_with_stored_tracks(&ui_tests::two_live_track_ranges());
    let store = Store::open_in(dir.path());
    store
        .open_recordings()
        .unwrap()
        .set_recording_ui_state(
            &recording,
            &RecordingUiState::with_hidden_track_numbers([1]),
            &UiStateVersionReporter::default(),
        )
        .unwrap();
    self::open_recording(&mut harness, &recording);
    let app = harness.state_mut();
    let first = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let first_check = |app: &App| app.shared.borrow().tree.track_node(first).unwrap().check;
    let loaded_id = app.shared.borrow().loaded_files.view().get(0).unwrap().id();
    let predecessor = recordings::worker_track_snapshot(&app.history, &recording);
    assert!(predecessor.layout().is_current());
    assert_eq!(first_check(app), CheckState::Off);
    app.history
        .rename_identity(recording.identity, "renamed".to_owned());
    let replies = self::replies_through_listing(&app.history);
    assert!(!predecessor.layout().is_current());
    let successor = self::single_rename_successor(&replies).clone();

    app.apply_resegmentation();
    assert_eq!(first_check(app), CheckState::On);
    for reply in replies {
        app.handle_history_response(reply);
    }
    {
        let shared = app.shared.borrow();
        assert_eq!(shared.loaded_files.len(), 1);
        let entry = shared.loaded_files.view().get(0).unwrap();
        assert_eq!(entry.id(), loaded_id);
        let adopted = entry.history().track_snapshot().unwrap();
        assert!(adopted.is_current());
        assert!(adopted.same_layout_and_payload(&successor));
    }
    let displayed = first_check(app);
    let writes = self::replies_through_listing(&app.history);
    assert_eq!(
        (
            displayed,
            self::saved_visibility(&store, successor.recording())
        ),
        (CheckState::On, RecordingUiState::default())
    );
    assert!(
        writes
            .iter()
            .any(|reply| matches!(reply, Response::RecordingUiStateStored(Ok(()))))
    );
    drop(harness);
    self::reopen_recordings_with_checks(
        &store,
        &[(
            successor.recording().clone(),
            &[CheckState::On, CheckState::On],
        )],
    );
}

#[test]
fn processing_during_a_rename_supersedes_a_pending_visibility_read() {
    let (mut harness, dir, recording) =
        recordings::app_with_stored_tracks(&ui_tests::two_live_track_ranges());
    let store = Store::open_in(dir.path());
    let saved = RecordingUiState::with_hidden_track_numbers([1, 7]);
    store
        .open_recordings()
        .unwrap()
        .set_recording_ui_state(&recording, &saved, &UiStateVersionReporter::default())
        .unwrap();
    let app = harness.state_mut();
    let predecessor = recordings::worker_track_snapshot(&app.history, &recording);
    self::load_disk_copy_before_history_replies(app, &predecessor);
    let old_reads = self::replies_through_listing(&app.history);
    assert!(old_reads.iter().any(|reply| matches!(reply,
        Response::RecordingUiStateLoaded(Ok((layout, state)))
            if layout.same_observation(predecessor.layout()) && *state == saved
    )));
    assert!(
        app.shared
            .borrow()
            .tree
            .hidden_track_numbers_for_layout(predecessor.layout())
            .is_none()
    );
    app.history
        .rename_identity(recording.identity, "renamed".to_owned());
    let replies = self::replies_through_listing(&app.history);
    assert!(!predecessor.layout().is_current());
    let destination = self::single_rename_successor(&replies).recording().clone();

    let first = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let second = TrackRef::new(first.fi, TrackIdx::new(1));
    app.shared.borrow_mut().tree.hide_track(first);
    app.apply_resegmentation();
    app.shared.borrow_mut().tree.hide_track(second);
    assert_eq!(self::saved_visibility(&store, &destination), saved);
    for reply in replies.into_iter().chain(old_reads) {
        app.handle_history_response(reply);
    }

    let checks = {
        let shared = app.shared.borrow();
        [first, second].map(|track| shared.tree.track_node(track).unwrap().check)
    };
    self::replies_through_listing(&app.history);
    assert_eq!(
        (checks, self::saved_visibility(&store, &destination)),
        (
            [CheckState::On, CheckState::Off],
            RecordingUiState::with_hidden_track_numbers([2]),
        )
    );
    drop(harness);
    self::reopen_recordings_with_checks(&store, &[(destination, &checks)]);
}

#[test]
fn applying_processing_during_a_rename_keeps_only_later_visibility_edits() {
    let (mut harness, dir, recording) =
        recordings::app_with_stored_tracks(&ui_tests::two_live_track_ranges());
    let store = Store::open_in(dir.path());
    store
        .open_recordings()
        .unwrap()
        .set_recording_ui_state(
            &recording,
            &RecordingUiState::with_hidden_track_numbers([1, 7]),
            &UiStateVersionReporter::default(),
        )
        .unwrap();
    self::open_recording(&mut harness, &recording);
    let app = harness.state_mut();
    app.history
        .rename_identity(recording.identity, "renamed".to_owned());
    let rename_replies = self::replies_through_listing(&app.history);
    let successor = self::single_rename_successor(&rename_replies);
    let destination = successor.recording().clone();
    self::load_disk_copy_before_history_replies(app, successor);
    let old_reads = self::replies_through_listing(&app.history);

    let original_second = TrackRef::new(FileIdx::new(0), TrackIdx::new(1));
    let copy_first = TrackRef::new(FileIdx::new(1), TrackIdx::new(0));
    app.shared.borrow_mut().tree.hide_track(original_second);
    app.apply_resegmentation();
    app.shared.borrow_mut().tree.hide_track(copy_first);
    for reply in rename_replies.into_iter().chain(old_reads) {
        app.handle_history_response(reply);
    }
    let writes = self::replies_through_listing(&app.history);
    assert!(
        writes
            .iter()
            .any(|reply| matches!(reply, Response::RecordingUiStateStored(Ok(()))))
    );
    assert_eq!(
        self::saved_visibility(&store, &destination),
        RecordingUiState::with_hidden_track_numbers([1])
    );
}

#[derive(Clone, Copy)]
enum RenameDelivery {
    Sequential,
    Batched,
    ExpiredRead,
}

#[rstest]
#[case::sequential(RenameDelivery::Sequential)]
#[case::batched(RenameDelivery::Batched)]
#[case::expired_read(RenameDelivery::ExpiredRead)]
fn chained_renames_preserve_visibility_after_shelving(#[case] delivery: RenameDelivery) {
    let (mut harness, dir, original) =
        recordings::app_with_stored_tracks(&ui_tests::two_live_track_ranges());
    let store = Store::open_in(dir.path());
    let initially_hidden = RecordingUiState::with_hidden_track_numbers([1]);
    store
        .open_recordings()
        .unwrap()
        .set_recording_ui_state(
            &original,
            &initially_hidden,
            &UiStateVersionReporter::default(),
        )
        .unwrap();
    harness.state().history.open(original.clone());
    let first = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let second = TrackRef::new(first.fi, TrackIdx::new(1));
    assert!(harness.step_until(|harness| {
        let shared = harness.state().shared.borrow();
        shared
            .tree
            .track_node(first)
            .is_some_and(|node| node.check == CheckState::Off)
    }));
    let app = harness.state_mut();
    let (file_id, predecessor) = {
        let shared = app.shared.borrow();
        let entry = shared.loaded_files.view().get(0).unwrap();
        (
            entry.id(),
            entry.history().track_snapshot().unwrap().clone(),
        )
    };
    if matches!(delivery, RenameDelivery::ExpiredRead) {
        let mut shared = app.shared.borrow_mut();
        shared.tree = TreeState::default();
        shared.sync_tree_from_loaded_files();
        drop(shared);
        app.read_recording_ui_state_if_unobserved(Some(predecessor.layout()));
    }
    {
        let _access = HistoryAccess::lock();
        app.history.remove_loaded_tracks(
            StoredTrackAction::Shelve,
            RecordingTrackRemoval::from_loaded(
                predecessor.clone(),
                vec![LoadedTrackRemoval::new(file_id, vec![0])],
            ),
        );
        app.history
            .rename_identity(original.identity, "middle".to_owned());
        if !matches!(delivery, RenameDelivery::ExpiredRead) {
            app.shared.borrow_mut().tree.hide_track(second);
            app.store_the_hidden_tracks_the_user_changed();
        }
        if !matches!(delivery, RenameDelivery::Sequential) {
            app.history
                .rename_identity("middle".to_owned(), "final".to_owned());
        }
    }

    let replies = self::replies_through_listing(&app.history);
    let renames: Vec<_> = self::completed_renames(&replies).collect();
    let sequential = matches!(delivery, RenameDelivery::Sequential);
    assert_eq!(renames.len(), if sequential { 1 } else { 2 });
    let [first_move] = renames[0].snapshot_updates() else {
        panic!("expected a verified move");
    };
    assert!(first_move.before().same_layout_and_payload(&predecessor));
    assert_eq!(predecessor.rows()[0].state, TrackState::Live);
    assert_eq!(first_move.before().rows()[0].state, TrackState::Shelved);
    assert!(!predecessor.is_current_with_membership());
    assert_eq!(first_move.after().is_current(), sequential);
    if matches!(delivery, RenameDelivery::ExpiredRead) {
        assert!(replies.iter().any(|reply| {
            matches!(reply, Response::RecordingUiStateLoaded(Ok((layout, state)))
                if layout.same_observation(predecessor.layout()) && !layout.is_current()
                    && *state == initially_hidden)
        }));
        app.shared.borrow_mut().tree.hide_track(second);
    }
    for reply in replies {
        app.handle_history_response(reply);
    }
    if sequential {
        let shared = app.shared.borrow();
        let entry = shared.loaded_files.view().entry_for_id(file_id).unwrap();
        assert_eq!(entry.identity(), Some("middle"));
        assert!(
            entry
                .history()
                .track_snapshot()
                .unwrap()
                .is_current_with_membership()
        );
        drop(shared);
        app.history
            .rename_identity("middle".to_owned(), "final".to_owned());
        let replies = self::replies_through_listing(&app.history);
        assert_eq!(self::completed_renames(&replies).count(), 1);
        for reply in replies {
            app.handle_history_response(reply);
        }
    }

    let final_recording = {
        let mut shared = app.shared.borrow_mut();
        shared.sync_tree_from_loaded_files();
        let entry = shared.loaded_files.view().entry_for_id(file_id).unwrap();
        let snapshot = entry.history().track_snapshot().unwrap();
        assert!(snapshot.is_current_with_membership());
        assert_eq!(entry.identity(), Some("final"));
        assert_eq!(entry.history().db_ref(), Some(snapshot.recording()));
        assert_eq!(entry.file().tracks.len(), 1);
        assert_eq!(entry.file().tracks[0].metadata.index, 2);
        assert_eq!(
            shared.tree.track_node(first).unwrap().check,
            CheckState::Off
        );
        snapshot.recording().clone()
    };
    let both_hidden = RecordingUiState::with_hidden_track_numbers([1, 2]);
    assert!(
        harness.step_until(|_| self::saved_visibility(&store, &final_recording) == both_hidden)
    );
    harness
        .state()
        .shared
        .borrow_mut()
        .tree
        .toggle_track_check(first);
    assert!(
        harness
            .step_until(|_| self::saved_visibility(&store, &final_recording) == initially_hidden)
    );
    harness.state().shared.borrow_mut().tree.hide_track(first);
    assert!(
        harness.step_until(|_| self::saved_visibility(&store, &final_recording) == both_hidden)
    );

    drop(harness);
    let reopened =
        self::reopen_recordings_with_checks(&store, &[(final_recording, &[CheckState::Off])]);
    let shared = reopened.state().shared.borrow();
    assert_eq!(shared.loaded_files.files()[0].tracks[0].metadata.index, 2);
}

fn replace_source_and_insert_incumbent(
    store: &Store,
    source: &DatabaseRef,
    rows: &[TrackRange],
) -> DatabaseRef {
    let reporter = UiStateVersionReporter::default();
    let source_hidden = RecordingUiState::with_hidden_track_numbers([2]);
    let destination_hidden = RecordingUiState::with_hidden_track_numbers([1]);
    let replacement = recordings::bytes_starting_at(1_750_000_000, 20);
    let replacement_meta = gt_store::extract_meta(&replacement).unwrap();
    let incumbent = recordings::bytes_starting_at(1_751_000_000, 20);
    let incumbent_meta = gt_store::extract_meta(&incumbent).unwrap();
    let mut db = store.open_recordings().unwrap();
    db.replace_recording_in_place(
        source,
        &replacement_meta,
        rows,
        recordings::segmentation(),
        &replacement,
    )
    .unwrap();
    let destination = db
        .insert(
            "destination",
            &incumbent_meta,
            rows,
            recordings::segmentation(),
            &incumbent,
        )
        .unwrap();
    db.set_recording_ui_state(source, &source_hidden, &reporter)
        .unwrap();
    db.set_recording_ui_state(&destination, &destination_hidden, &reporter)
        .unwrap();
    destination
}

#[test]
fn history_merge_preserves_incumbent_visibility_and_excludes_an_expired_payload() {
    let rows = ui_tests::two_live_track_ranges();
    let (mut harness, dir, source) = recordings::app_with_stored_tracks(&rows);
    let store = Store::open_in(dir.path());
    self::open_recording(&mut harness, &source);
    let (old_id, old_snapshot) = {
        let shared = harness.state().shared.borrow();
        let entry = shared.loaded_files.view().get(0).unwrap();
        (
            entry.id(),
            entry.history().track_snapshot().unwrap().clone(),
        )
    };
    let destination = self::replace_source_and_insert_incumbent(&store, &source, &rows);
    assert!(!old_snapshot.is_current());
    self::open_recording(&mut harness, &source);
    self::open_recording(&mut harness, &destination);
    let moved_track = TrackRef::new(FileIdx::new(1), TrackIdx::new(1));
    let incumbent_track = TrackRef::new(FileIdx::new(2), TrackIdx::new(0));
    let (moved_id, incumbent_id, incumbent_snapshot) = {
        let shared = harness.state().shared.borrow();
        let view = shared.loaded_files.view();
        let moved = view.get(1).unwrap();
        let current = moved.history().track_snapshot().unwrap();
        assert_eq!(current.rows(), old_snapshot.rows());
        assert!(!current.same_payload_revision(&old_snapshot));
        let incumbent = view.get(2).unwrap();
        (
            moved.id(),
            incumbent.id(),
            incumbent.history().track_snapshot().unwrap().clone(),
        )
    };

    let app = harness.state_mut();
    app.history
        .rename_identity(source.identity.clone(), destination.identity.clone());
    let successor = self::apply_single_rename(app);
    let moved_recording = successor.recording().clone();
    {
        let mut shared = app.shared.borrow_mut();
        shared.sync_tree_from_loaded_files();
        let view = shared.loaded_files.view();
        let stale = view.entry_for_id(old_id).unwrap().history();
        assert_eq!(stale.db_ref(), Some(&source));
        assert!(
            stale
                .track_snapshot()
                .unwrap()
                .same_layout_and_payload(&old_snapshot)
        );
        let moved = view.entry_for_id(moved_id).unwrap().history();
        assert_eq!(moved.db_ref(), Some(&moved_recording));
        let moved_snapshot = moved.track_snapshot().unwrap();
        assert!(moved_snapshot.is_current_with_membership());
        assert!(moved_snapshot.same_layout_and_payload(&successor));
        let incumbent = view.entry_for_id(incumbent_id).unwrap().history();
        assert_eq!(incumbent.db_ref(), Some(&destination));
        assert!(incumbent_snapshot.is_current_with_membership());
        assert!(
            incumbent
                .track_snapshot()
                .unwrap()
                .same_layout_and_payload(&incumbent_snapshot)
        );
        assert_eq!(
            shared.tree.track_node(moved_track).unwrap().check,
            CheckState::Off
        );
        assert_eq!(
            shared.tree.track_node(incumbent_track).unwrap().check,
            CheckState::Off
        );
        shared.tree.toggle_track_check(moved_track);
        shared
            .tree
            .hide_track(TrackRef::new(incumbent_track.fi, TrackIdx::new(1)));
    }
    let destination_hidden = RecordingUiState::with_hidden_track_numbers([1, 2]);
    assert!(harness.step_until(|_| {
        self::saved_visibility(&store, &moved_recording) == RecordingUiState::default()
            && self::saved_visibility(&store, &destination) == destination_hidden
    }));

    drop(harness);
    self::reopen_recordings_with_checks(
        &store,
        &[
            (moved_recording, &[CheckState::On, CheckState::On]),
            (destination, &[CheckState::Off, CheckState::Off]),
        ],
    );
}
