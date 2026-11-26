use egui_kittest::Harness;
use gt_loaded_files::FileHistory;
use gt_pending_writes::WriteKind;
use gt_session_bridge::{
    ErrorCode, GetRecording, ListRecordings, QueryJobState, RecordingReference, Reply, Request,
    SetTrackVisibility, TrackReference, TrackVisibilityAction, TrackVisibilityResult,
    VisibilityTrackRefs,
};
use gt_side_panel::{CheckState, NodeKey, TreeState};
use gt_store::{
    HistoryAccess, HistoryDatabase as _, ReadOnlyHistoryDatabase as _, RecordingUiState, Store,
    UiStateVersionReporter,
};
use gt_test_utils::HarnessInteraction as _;
use gt_types::{DataCategory, FileIdx, TrackIdx, TrackRef};
use rstest::rstest;
use uuid::Uuid;

use crate::app::test_util::{harness, recordings, session};
use crate::app::{App, ui_tests};

#[test]
fn solo_notifies_once_when_only_a_recording_checkbox_changes() {
    let mut harness = ui_tests::app_with_a_recording();
    let empty_file = FileIdx::new(1);
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.loaded_files.push(
            gt_test_utils::loaded_file_with_tracks(Vec::new()),
            FileHistory::None,
        );
        shared.sync_tree_from_loaded_files();
    }
    let track = session::first_track_reference(&mut harness);
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .tree
            .file_node(empty_file)
            .unwrap()
            .check,
        CheckState::On
    );
    let notices = harness.state().toasts.len();
    for _ in 0..2 {
        assert_eq!(
            session::execute(
                &mut harness,
                self::visibility_request(TrackVisibilityAction::Solo, vec![track])
            ),
            Reply::TrackVisibility(TrackVisibilityResult {
                action: TrackVisibilityAction::Solo,
                targeted_tracks: 1,
                changed_tracks: 0
            }),
        );
        assert_eq!(
            harness
                .state()
                .shared
                .borrow()
                .tree
                .file_node(empty_file)
                .unwrap()
                .check,
            CheckState::Off
        );
        assert_eq!(harness.state().toasts.len(), notices + 1);
    }
}

#[test]
fn track_visibility_updates_inspection_and_query_scope_across_recordings() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    let tracks = self::track_references(&mut harness);
    let [first, second, third] = tracks.as_slice() else {
        panic!("expected three loaded tracks");
    };
    let (first, second, third) = (*first, *second, *third);
    let selected = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    {
        let tree = &mut harness.state_mut().shared.borrow_mut().tree;
        tree.apply_click(NodeKey::Track(selected), false, false);
        tree.set_category_visible(selected, DataCategory::Track, false);
    }

    for (action, tracks, expected) in [
        (
            TrackVisibilityAction::Hide,
            vec![first, third, first],
            [2, 2],
        ),
        (TrackVisibilityAction::Show, vec![first], [1, 1]),
        (TrackVisibilityAction::Solo, vec![first, third], [2, 2]),
        (TrackVisibilityAction::Solo, vec![third, first], [2, 0]),
    ] {
        let [targeted_tracks, changed_tracks] = expected;
        let notices = harness.state().toasts.len();
        assert_eq!(
            session::execute(&mut harness, self::visibility_request(action, tracks)),
            Reply::TrackVisibility(TrackVisibilityResult {
                action,
                targeted_tracks,
                changed_tracks
            }),
        );
        assert_eq!(
            harness.state().toasts.len(),
            notices + usize::from(changed_tracks > 0)
        );
    }
    for (track, enabled) in [(first, true), (second, false), (third, true)] {
        let state = session::track_details(&mut harness, track)
            .state
            .tree
            .unwrap();
        assert_eq!(state.enabled, enabled);
        assert_eq!(state.selected, track == first);
    }
    {
        let shared = harness.state().shared.borrow();
        assert!(!shared.tree.track_node(selected).unwrap().track_visible);
        assert!(!shared.zoom_to_visible_request);
    }

    let job = session::start_query(&mut harness, "points | table velocity");
    let QueryJobState::Completed { queries } = session::wait_for_query_to_stop(&mut harness, job)
    else {
        panic!("expected completed point query");
    };
    assert_eq!(queries[0].tracks_with_matches, 2);
}

#[test]
fn track_visibility_keeps_the_batch_unchanged_if_any_reference_fails() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    let tracks = self::track_references(&mut harness);
    let first = tracks[0];
    let foreign = TrackReference {
        recording: RecordingReference {
            session: Uuid::nil(),
            ..tracks[1].recording
        },
        ..tracks[1]
    };
    let stale = TrackReference {
        revision: tracks[1].revision + 1,
        ..tracks[1]
    };
    let missing_tree_track = tracks[2];
    harness.state_mut().shared.borrow_mut().tree.files[2]
        .tracks
        .clear();
    let notices = harness.state().toasts.len();
    for (reference, code) in [
        (foreign, ErrorCode::StaleSession),
        (stale, ErrorCode::StaleTrack),
        (missing_tree_track, ErrorCode::StaleTrack),
    ] {
        assert_eq!(
            session::execute(
                &mut harness,
                self::visibility_request(TrackVisibilityAction::Hide, vec![first, reference])
            ),
            Reply::Error { code },
        );
        assert!(
            session::track_details(&mut harness, first)
                .state
                .tree
                .unwrap()
                .enabled
        );
        assert_eq!(harness.state().toasts.len(), notices);
    }
}

#[rstest]
#[case::observed(false)]
#[case::pending(true)]
fn semantic_show_and_hide_preserve_saved_checkbox_edits_through_shutdown(
    #[case] baseline_pending: bool,
) {
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
    harness.state().history.open(recording.clone());
    let first_node = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    assert!(harness.step_until(|harness| {
        harness
            .state()
            .shared
            .borrow()
            .tree
            .track_node(first_node)
            .is_some_and(|node| node.check == CheckState::Off)
    }));
    let tracks = self::track_references(&mut harness);

    {
        let _access = HistoryAccess::lock();
        if baseline_pending {
            let app = harness.state();
            let mut shared = app.shared.borrow_mut();
            shared.tree = TreeState::new();
            shared.sync_tree_from_loaded_files();
            app.history.load_recording_ui_state(recording.clone());
        }
        assert_eq!(
            session::execute(
                &mut harness,
                self::visibility_request(TrackVisibilityAction::Show, vec![tracks[0]])
            ),
            Reply::TrackVisibility(TrackVisibilityResult {
                action: TrackVisibilityAction::Show,
                targeted_tracks: 1,
                changed_tracks: usize::from(!baseline_pending)
            }),
        );
        assert_eq!(
            harness
                .state()
                .pending_writes
                .snapshot()
                .running
                .iter()
                .filter(|write| write.kind == WriteKind::RecordingDatabase)
                .count(),
            usize::from(!baseline_pending)
        );
        assert_eq!(
            session::execute(
                &mut harness,
                self::visibility_request(TrackVisibilityAction::Hide, vec![tracks[1]])
            ),
            Reply::TrackVisibility(TrackVisibilityResult {
                action: TrackVisibilityAction::Hide,
                targeted_tracks: 1,
                changed_tracks: 1
            }),
        );
        harness.state_mut().begin_shutdown();
        assert_eq!(
            session::execute(
                &mut harness,
                self::visibility_request(TrackVisibilityAction::Hide, vec![tracks[0]])
            ),
            Reply::from(ErrorCode::ShuttingDown),
        );
    }
    harness::finish_shutdown_without_painting(&mut harness);
    drop(harness);

    {
        let db = store.open_recordings_read_only().unwrap().unwrap();
        assert_eq!(
            db.recording_ui_state(&recording, &UiStateVersionReporter::default())
                .unwrap(),
            RecordingUiState::with_hidden_track_numbers([2, 7])
        );
        assert_eq!(
            db.load(&recording).unwrap().track_rows(),
            ui_tests::two_live_track_ranges()
        );
    }
    let (mut reopened, databases) = harness::app_with_the_databases_still_opening(&[]);
    reopened.step();
    harness::send_open_storage_and_step(&mut reopened, &databases, &store);
    reopened.state().history.open(recording);
    assert!(
        reopened.step_until(|harness| !harness.state().shared.borrow().loaded_files.is_empty())
    );
    let tracks = self::track_references(&mut reopened);
    assert!(
        session::track_details(&mut reopened, tracks[0])
            .state
            .tree
            .unwrap()
            .enabled
    );
    assert!(
        !session::track_details(&mut reopened, tracks[1])
            .state
            .tree
            .unwrap()
            .enabled
    );
}

fn visibility_request(action: TrackVisibilityAction, tracks: Vec<TrackReference>) -> Request {
    Request::SetTrackVisibility(SetTrackVisibility {
        action,
        tracks: VisibilityTrackRefs::try_from(tracks).unwrap(),
    })
}

fn track_references(harness: &mut Harness<'_, App>) -> Vec<TrackReference> {
    let Reply::Recordings { page } = session::execute(
        harness,
        Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }),
    ) else {
        panic!("expected loaded recordings");
    };
    page.recordings
        .into_iter()
        .flat_map(|recording| {
            session::recording_details(
                harness,
                GetRecording {
                    recording: recording.id,
                    after: None,
                    stored_track_start: None,
                    limit: None,
                },
            )
            .tracks
        })
        .collect()
}
