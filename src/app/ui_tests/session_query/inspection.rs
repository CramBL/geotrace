use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use chrono::TimeDelta;
use egui_kittest::Harness;
use geotrace_sdk::{Angle, NavFileBuilder, NavFix, NavFixTime};
use gt_session_bridge::test_util::TestEndpoint;
use gt_session_bridge::{
    ErrorCode, GetRecording, GetTrack, ListRecordings, MAX_TRACKS_PER_PAGE, RecordingDetails,
    RecordingReference, Reply, Request, StoredTrackRow, StoredTrackState, TrackDetails,
    TrackReference, TrackTreeState,
};
use gt_side_panel::NodeKey;
use gt_store::{Store, TrackRange, TrackState};
use gt_test_utils::HarnessInteraction as _;
use gt_types::{FileIdx, TrackIdx, TrackRef};

use crate::app::test_util::{harness, recordings, session};
use crate::app::{App, ui_tests};

#[test]
fn inspection_reports_actual_checkbox_and_row_selection_for_each_recording() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    let Reply::Recordings { page } = session::execute(
        &mut harness,
        Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }),
    ) else {
        panic!("expected loaded recordings");
    };
    let first = page.recordings[0].id;
    let second = page.recordings[1].id;
    let second_track = TrackRef::new(FileIdx::new(1), TrackIdx::new(0));
    {
        let shared = &mut *harness.state_mut().shared.borrow_mut();
        shared.tree.toggle_track_check(second_track);
        shared
            .tree
            .apply_click(NodeKey::Track(second_track), false, false);
    }

    for (recording, state) in [
        (
            first,
            TrackTreeState {
                enabled: true,
                selected: false,
            },
        ),
        (
            second,
            TrackTreeState {
                enabled: false,
                selected: true,
            },
        ),
    ] {
        let details = self::get_recording(&mut harness, self::first_page(recording));
        let track = details.tracks[0];
        let track_state = details.track_states[0];
        assert_eq!(track_state.index, track.index);
        assert_eq!(track_state.state.tree, Some(state));
        assert_eq!(
            self::get_track(&mut harness, track).state,
            track_state.state
        );
        assert_eq!(track_state.state.stored_state, None);
        assert_eq!(details.stored_tracks, None);
    }
    let mut cross_recording = self::first_page(first);
    cross_recording.after =
        Some(self::get_recording(&mut harness, self::first_page(second)).tracks[0]);
    assert_eq!(
        session::execute(&mut harness, Request::GetRecording(cross_recording)),
        Reply::Error {
            code: ErrorCode::StaleTrack
        },
    );
}

#[test]
fn inspection_pages_shelved_history_rows_and_excludes_expired_membership() {
    let [live, mut shelf] = ui_tests::two_live_track_ranges();
    shelf.state = TrackState::Shelved;
    let tombstone = TrackRange {
        start: live.end,
        end: live.end,
        state: TrackState::Deleted,
    };
    let (mut harness, _dir, db_ref) = recordings::app_with_stored_tracks(&[live, tombstone, shelf]);
    harness.state().history.open(db_ref);
    assert!(harness.step_until(|harness| !harness.state().shared.borrow().loaded_files.is_empty()));
    let track = session::first_track_reference(&mut harness);
    self::assert_stored_track_pages(&mut harness, track);

    self::unshelve_before_adoption(&harness, 2);
    let first = self::first_page(track.recording);
    let expired = self::get_recording(&mut harness, first);
    assert_eq!(expired.tracks, [track]);
    assert_eq!(expired.track_states[0].state.stored_state, None);
    assert_eq!(expired.stored_tracks, None);
    assert_eq!(
        self::get_track(&mut harness, track).state.stored_state,
        None
    );

    assert!(harness.step_until(|harness| {
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .view()
            .get(0)
            .unwrap()
            .file()
            .tracks
            .len()
            == 2
    }));
    assert_eq!(
        session::execute(&mut harness, Request::GetTrack(GetTrack { track })),
        Reply::Error {
            code: ErrorCode::StaleTrack
        }
    );
    let refreshed = self::get_recording(
        &mut harness,
        GetRecording {
            stored_track_start: NonZeroUsize::new(3),
            ..first
        },
    );
    let page = refreshed.stored_tracks.unwrap();
    assert_eq!(
        page.rows,
        [StoredTrackRow {
            track_number: 3,
            state: StoredTrackState::Live
        }]
    );
    let new_track = self::get_recording(
        &mut harness,
        GetRecording {
            after: refreshed.next_after,
            ..first
        },
    );
    assert_eq!(new_track.tracks[0].index, 1);
    assert_eq!(new_track.track_states[0].track_number, 3);
    assert_eq!(
        new_track.track_states[0].state.stored_state,
        Some(StoredTrackState::Live)
    );
}

fn unshelve_before_adoption(harness: &Harness<'_, App>, row: usize) {
    let snapshot = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .view()
        .get(0)
        .unwrap()
        .history()
        .track_snapshot()
        .unwrap()
        .clone();
    assert!(snapshot.is_current_with_membership());
    harness
        .state()
        .history
        .set_tracks_shelved(snapshot.clone(), vec![row], false);
    let deadline = Instant::now() + Duration::from_secs(5);
    while snapshot.is_current_with_membership() {
        assert!(
            Instant::now() < deadline,
            "History did not unshelve the track"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(snapshot.is_current());
}

fn assert_stored_track_pages(harness: &mut Harness<'_, App>, track: TrackReference) {
    let mut args = self::first_page(track.recording);
    for (number, state) in [
        (1, StoredTrackState::Live),
        (2, StoredTrackState::Deleted),
        (3, StoredTrackState::Shelved),
    ] {
        let details = self::get_recording(harness, args);
        assert_eq!(
            details.tracks,
            if number == 1 { vec![track] } else { Vec::new() }
        );
        assert_eq!(details.next_after, None);
        let page = details.stored_tracks.unwrap();
        assert_eq!(
            page.rows,
            [StoredTrackRow {
                track_number: number,
                state
            }]
        );
        assert_eq!(
            page.next_start,
            NonZeroUsize::new(number + 1).filter(|_| number < 3)
        );
        args.after = Some(track);
        args.stored_track_start = page.next_start;
    }
    assert_eq!(
        self::get_track(harness, track).state.stored_state,
        Some(StoredTrackState::Live)
    );
}

fn first_page(recording: RecordingReference) -> GetRecording {
    GetRecording {
        recording,
        after: None,
        stored_track_start: None,
        limit: Some(NonZeroUsize::MIN),
    }
}

#[test]
fn full_loaded_and_stored_track_pages_with_large_metadata_fit_the_wire_reply() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open_in(directory.path());
    let title = "Grøn\"\n".repeat(512);
    let mut builder = NavFileBuilder::new().with_title(&title).unwrap().open();
    let mut rows = Vec::new();
    for index in 0..=MAX_TRACKS_PER_PAGE {
        builder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(
                    ui_tests::base_time() + TimeDelta::seconds(index as i64),
                ))
                .lat(Angle::degrees(51.5))
                .lon(Angle::degrees(-0.1))
                .build(),
        );
        rows.push(TrackRange {
            start: index as u64,
            end: index as u64 + 1,
            state: if index == MAX_TRACKS_PER_PAGE {
                TrackState::Shelved
            } else {
                TrackState::Live
            },
        });
    }
    let mut bytes = Vec::new();
    builder.finish().unwrap().write(&mut bytes).unwrap();
    let db_ref = recordings::store_recording(&store.recordings_path(), &bytes, &rows);
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    harness.step();
    harness::send_open_storage_and_step(&mut harness, &databases, &store);
    harness.state().history.open(db_ref);
    assert!(harness.step_until(|harness| !harness.state().shared.borrow().loaded_files.is_empty()));
    let endpoint = session::install_bridge(&mut harness);
    let Reply::Recordings { page } = self::wire_call(
        &mut harness,
        &endpoint,
        &Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }),
    ) else {
        panic!("expected loaded recording");
    };
    let first = GetRecording {
        limit: Some(NonZeroUsize::MAX),
        ..self::first_page(page.recordings[0].id)
    };
    let Reply::Recording { details } =
        self::wire_call(&mut harness, &endpoint, &Request::GetRecording(first))
    else {
        panic!("expected full track pages within the frame limit");
    };
    assert!(details.recording.metadata_truncated);
    assert_eq!(details.tracks.len(), MAX_TRACKS_PER_PAGE);
    assert_eq!(details.track_states.len(), MAX_TRACKS_PER_PAGE);
    let stored = details.stored_tracks.unwrap();
    assert_eq!(stored.rows.len(), MAX_TRACKS_PER_PAGE);
    assert_eq!(
        stored.rows.last().unwrap().track_number,
        MAX_TRACKS_PER_PAGE
    );
    let Reply::Recording { details } = self::wire_call(
        &mut harness,
        &endpoint,
        &Request::GetRecording(GetRecording {
            stored_track_start: stored.next_start,
            after: details.tracks.last().copied(),
            ..first
        }),
    ) else {
        panic!("expected stored continuation");
    };
    assert!(details.tracks.is_empty());
    assert!(details.track_states.is_empty());
    let stored = details.stored_tracks.unwrap();
    assert_eq!(
        stored.rows,
        [StoredTrackRow {
            track_number: MAX_TRACKS_PER_PAGE + 1,
            state: StoredTrackState::Shelved
        }]
    );
    assert_eq!(stored.next_start, None);
}

fn wire_call(harness: &mut Harness<'_, App>, endpoint: &TestEndpoint, request: &Request) -> Reply {
    let bytes = session::exchange_payload(harness, endpoint, &serde_json::to_vec(request).unwrap());
    serde_json::from_slice(&bytes).unwrap()
}

fn get_recording(harness: &mut Harness<'_, App>, args: GetRecording) -> RecordingDetails {
    let Reply::Recording { details } = session::execute(harness, Request::GetRecording(args))
    else {
        panic!("expected recording details");
    };
    details
}

fn get_track(harness: &mut Harness<'_, App>, track: TrackReference) -> TrackDetails {
    let Reply::Track { details } = session::execute(harness, Request::GetTrack(GetTrack { track }))
    else {
        panic!("expected track details");
    };
    details
}
