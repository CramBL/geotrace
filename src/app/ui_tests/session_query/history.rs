use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE as ICON_CLOCK_COUNTER_CLOCKWISE;
use geotrace_sdk::{Angle, NavFile, NavFileBuilder, NavFix, NavFixTime};
use gt_loaded_files::FileHistory;
use gt_pending_writes::WriteAccess;
use gt_session_bridge::{
    ApplyProcessing, ErrorCode, GetHistoryOpenStatus, HistoryDates, HistoryOpenId,
    HistoryOpenState, HistoryOpenStatus, HistoryRecordings, ListHistoryRecordings, ListRecordings,
    OpenHistoryRecording, QueryCounts, QueryJobState, RecordingReference, Reply, Request,
};
use gt_side_panel::{CheckState, NodeKey};
use gt_store::{
    DatabaseRef, DbError, HistoryAccess, HistoryDatabase as _, LogAttachments as _, LogToAttach,
    ReadOnlyHistoryDatabase as _, RecordingUiState, Recordings, RecordingsHandle, Store,
    StoredLogSettings, StoredSegmentation, StoredYearResolution, TrackRange, TrackState,
    UiStateVersionReporter,
};
use gt_test_utils::{By, HarnessInteraction as _, recording_fixtures};
use gt_track_builder::SegmentationConfig;
use gt_types::{FileIdx, TrackIdx, TrackRef};
use hdf5_pure::{AttrValue, File as Hdf5File};
use rstest::rstest;

use crate::app::archive_recovery::UnavailableArchives;
use crate::app::association_batches::RecordingOperationOrigin;
use crate::app::auto_prune::AutoPruneOutcome;
use crate::app::history_db::{
    DbOp, DeleteReason, DeleteShelvedTracksScope, HistoryWorker, Response,
};
use crate::app::history_list::RecordingListing;
use crate::app::recording_from_disk::LOAD_FROM_DISK_LABEL;
use crate::app::storage::OpenStorage;
use crate::app::test_util::harness::{self, TestDroppedFile};
use crate::app::test_util::recordings;
use crate::app::test_util::session;
use crate::app::{App, ui_tests};

const UNFILTERED: ListHistoryRecordings = ListHistoryRecordings {
    cursor: None,
    dates: None,
    limit: None,
};

#[test]
fn a_history_open_reports_a_worker_admission_failure_in_its_reply() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    {
        let mut db = store.open_recordings().unwrap();
        recordings::insert_recording_as_one_whole_file_track(
            &mut db,
            "recording",
            &recordings::sample_bytes(),
        );
    }
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    let recording = page.recordings.first().unwrap().id;
    harness.state_mut().history.disconnect_requests_for_test();

    let status = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording { recording }),
    ));
    assert_eq!(
        status.state,
        HistoryOpenState::Failed {
            code: ErrorCode::HistoryUnavailable,
        }
    );
    assert_eq!(
        session::execute(
            &mut harness,
            Request::GetHistoryOpenStatus(GetHistoryOpenStatus {
                operation: status.operation,
            }),
        ),
        Reply::HistoryOpen { operation: status },
    );
    assert!(harness.state().shared.borrow().loaded_files.is_empty());
}

#[test]
fn a_history_open_reports_the_loaded_recording() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let mut db = store.open_recordings().unwrap();
    for (identity, start) in [("first", 1_000_000), ("other", 2_000_000)] {
        recordings::insert_recording_as_one_whole_file_track(
            &mut db,
            identity,
            &recordings::bytes_starting_at(start, 2),
        );
    }
    drop(db);
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    let request = Request::OpenHistoryRecording(OpenHistoryRecording {
        recording: page.recordings[0].id,
    });
    let notice_count = harness.state().toasts.len();
    let accepted = {
        let _access = HistoryAccess::lock();
        let accepted = self::open_status(&session::execute(&mut harness, request.clone()));
        assert_eq!(accepted.state, HistoryOpenState::Reading);
        assert_eq!(
            self::open_status(&session::execute(&mut harness, request.clone())),
            accepted
        );
        assert_eq!(
            session::execute(
                &mut harness,
                Request::OpenHistoryRecording(OpenHistoryRecording {
                    recording: page.recordings[1].id,
                })
            ),
            Reply::from(ErrorCode::Busy)
        );
        assert_eq!(harness.state().toasts.len(), notice_count);
        accepted
    };
    let HistoryOpenState::Ready {
        recording,
        reused: false,
    } = self::after_loading(&mut harness, accepted.operation)
    else {
        panic!("expected a loaded recording");
    };
    let Reply::Recordings { page } = session::execute(
        &mut harness,
        Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }),
    ) else {
        panic!("expected loaded recordings");
    };
    assert_eq!(page.recordings.len(), 1);
    assert_eq!(page.recordings[0].id, recording);
    harness.run_steps(2);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP opened a History recording"]
    );
    let repeated = self::open_status(&session::execute(&mut harness, request));
    assert_eq!(
        self::after_loading(&mut harness, repeated.operation),
        HistoryOpenState::Ready {
            recording,
            reused: true
        }
    );
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
    assert_eq!(harness.state().toasts.len(), notice_count + 1);
}

#[rstest]
#[case::cancel(HistoryDecision::Cancel)]
#[case::use_stored(HistoryDecision::UseStored)]
#[case::recalculate(HistoryDecision::Recalculate)]
#[case::recalculate_after_shelving(HistoryDecision::ShelveBeforeRecalculation)]
#[case::worker_replaced(HistoryDecision::ReplaceWorker)]
#[case::recording_replaced(HistoryDecision::ReplaceRecording)]
fn a_history_open_uses_the_current_stored_settings(#[case] decision: HistoryDecision) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let mut db = store.open_recordings().unwrap();
    let db_ref = recordings::insert_recording_as_one_whole_file_track(
        &mut db,
        "recording",
        &recordings::bytes_starting_at(1_000_000, 2),
    );
    let hidden = RecordingUiState::with_hidden_track_numbers([1]);
    let reporter = UiStateVersionReporter::default();
    db.set_recording_ui_state(&db_ref, &hidden, &reporter)
        .unwrap();
    drop(db);
    let mut harness = if matches!(
        decision,
        HistoryDecision::Recalculate | HistoryDecision::ShelveBeforeRecalculation
    ) {
        self::owner_app(&store)
    } else {
        self::read_only_app(&store)
    };
    let page = self::await_page(&mut harness, UNFILTERED);
    let request = Request::OpenHistoryRecording(OpenHistoryRecording {
        recording: page.recordings[0].id,
    });
    let initial = self::open_status(&session::execute(&mut harness, request.clone()));
    assert!(matches!(
        self::after_loading(&mut harness, initial.operation),
        HistoryOpenState::Ready { .. }
    ));
    let old_visibility_read = if matches!(decision, HistoryDecision::Recalculate) {
        let hidden_job = session::start_query(&mut harness, "points");
        assert_eq!(
            session::wait_for_query_to_stop(&mut harness, hidden_job),
            session::completed_single_track_query(0)
        );
        harness
            .state()
            .history
            .load_recording_ui_state(db_ref.clone());
        Some(recordings::next_visibility_response(&mut harness))
    } else {
        None
    };
    let different_split_gap_us = 600_000_000;
    {
        let _access = HistoryAccess::lock();
        let file = Hdf5File::open_rw(store.recordings_path()).unwrap();
        let group = file
            .group(&format!(
                "by_identity/{}/{}",
                gt_store::identity_group_name(&db_ref.identity),
                db_ref.group_name
            ))
            .unwrap();
        group
            .set_attr(
                "seg_track_split_gap_us",
                AttrValue::I64(different_split_gap_us),
            )
            .unwrap();
        file.commit().unwrap();
    }
    let reopened = self::open_status(&session::execute(&mut harness, request));
    assert_eq!(
        self::after_loading(&mut harness, reopened.operation),
        HistoryOpenState::AwaitingTrackSettings
    );
    let button = match decision {
        HistoryDecision::Cancel => "Cancel",
        HistoryDecision::UseStored => "Use stored tracks",
        HistoryDecision::Recalculate | HistoryDecision::ShelveBeforeRecalculation => {
            "Recalculate with current settings"
        }
        HistoryDecision::ReplaceRecording | HistoryDecision::ReplaceWorker => {
            if matches!(decision, HistoryDecision::ReplaceRecording) {
                self::replace_history_payload(&store, &db_ref);
            } else {
                harness
                    .state_mut()
                    .install_history_worker(HistoryWorker::disabled());
            }
            assert_eq!(
                self::after_loading(&mut harness, reopened.operation),
                HistoryOpenState::Expired
            );
            assert!(harness.state().pending_resegment.is_none());
            return;
        }
    };
    harness.run_steps(3);
    harness.get_by_label(crate::app::history_open::TRACK_SETTINGS_DIFFER_TITLE);
    {
        let _access = HistoryAccess::lock();
        harness.get_by_label(button).click();
        harness.run_steps(2);
        if matches!(decision, HistoryDecision::ShelveBeforeRecalculation) {
            let mut db = store.open_recordings().unwrap();
            db.set_tracks_shelved(&db_ref, &[0], true).unwrap();
        }
    }
    let state = self::after_loading(&mut harness, reopened.operation);
    match decision {
        HistoryDecision::Cancel => assert_eq!(state, HistoryOpenState::Cancelled),
        HistoryDecision::ShelveBeforeRecalculation => {
            assert_eq!(state, HistoryOpenState::Expired);
            let stored = store.open_recordings().unwrap().load(&db_ref).unwrap();
            assert_eq!(
                stored.segmentation.unwrap().track_split_gap_us,
                different_split_gap_us
            );
            assert_eq!(stored.track_rows()[0].state, TrackState::Shelved);
        }
        _ => assert!(matches!(
            state,
            HistoryOpenState::Ready { reused: false, .. }
        )),
    }
    if let Some(old_visibility_read) = old_visibility_read {
        let Response::RecordingUiStateLoaded(Ok((old_layout, old_state))) = &old_visibility_read
        else {
            panic!("expected an observed visibility response");
        };
        assert_eq!(old_state, &hidden);
        assert!(!old_layout.is_current());
        harness.state().history.store_recording_ui_state(
            old_layout.clone(),
            old_state.clone(),
            None,
        );
        harness
            .state_mut()
            .handle_history_response(old_visibility_read);
        let job = session::start_query(&mut harness, "points");
        assert_eq!(
            session::wait_for_query_to_stop(&mut harness, job),
            session::completed_single_track_query(1)
        );
        assert_eq!(
            store
                .open_recordings()
                .unwrap()
                .load(&db_ref)
                .unwrap()
                .segmentation,
            Some(recordings::segmentation()),
        );
        {
            let mut shared = harness.state().shared.borrow_mut();
            assert_eq!(shared.loaded_files.len(), 2);
            shared.tree.pending_unload = Some(vec![NodeKey::File(FileIdx::new(1))]);
        }
        harness.run_steps(3);
        assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
        let retained_hidden_job = session::start_query(&mut harness, "points");
        assert_eq!(
            session::wait_for_query_to_stop(&mut harness, retained_hidden_job),
            session::completed_single_track_query(0)
        );
        harness
            .state()
            .history
            .load_recording_ui_state(db_ref.clone());
        let Response::RecordingUiStateLoaded(Ok((_, reopened_visibility))) =
            recordings::next_visibility_response(&mut harness)
        else {
            panic!("expected the visibility saved after unloading");
        };
        assert_eq!(reopened_visibility, RecordingUiState::default());
        let original = harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .view()
            .get(0)
            .unwrap()
            .id();
        let layout = {
            let mut db = store.open_recordings().unwrap();
            db.set_tracks_shelved(&db_ref, &[0], true).unwrap();
            HistoryAccess::lock()
                .read_track_snapshot(&db, &db_ref)
                .unwrap()
        };
        harness
            .state()
            .history
            .set_tracks_shelved(layout.clone(), vec![0], false);
        assert!(harness.step_until(|harness| {
            let shared = harness.state().shared.borrow();
            shared
                .loaded_files
                .view()
                .entry_for_id(original)
                .is_some_and(|entry| {
                    entry
                        .history()
                        .track_snapshot()
                        .is_some_and(|snapshot| snapshot.same_observation(&layout))
                })
        }));
        assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
        let job = session::start_query(&mut harness, "points");
        assert_eq!(
            session::wait_for_query_to_stop(&mut harness, job),
            session::completed_single_track_query(1)
        );
        drop(harness);
        let mut restarted = self::owner_app(&store);
        let page = self::await_page(&mut restarted, UNFILTERED);
        let open = self::open_status(&session::execute(
            &mut restarted,
            Request::OpenHistoryRecording(OpenHistoryRecording {
                recording: page.recordings[0].id,
            }),
        ));
        assert!(matches!(
            self::after_loading(&mut restarted, open.operation),
            HistoryOpenState::Ready { .. }
        ));
        let job = session::start_query(&mut restarted, "points");
        assert_eq!(
            session::wait_for_query_to_stop(&mut restarted, job),
            session::completed_single_track_query(1)
        );
    } else {
        assert_eq!(
            store
                .open_recordings()
                .unwrap()
                .recording_ui_state(&db_ref, &reporter)
                .unwrap(),
            hidden
        );
    }
}

enum HistoryDecision {
    Cancel,
    Recalculate,
    ReplaceRecording,
    ReplaceWorker,
    ShelveBeforeRecalculation,
    UseStored,
}

#[test]
fn a_disk_copy_with_different_track_ranges_keeps_its_visibility_local() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let bytes = recording_fixtures::two_track_gtd_bytes();
    let one_track = [TrackRange {
        start: 0,
        end: 20,
        state: TrackState::Live,
    }];
    let longer_gap = TimeDelta::hours(2);
    let stored = StoredSegmentation {
        track_split_gap_us: longer_gap.num_microseconds().unwrap(),
        ..recordings::segmentation()
    };
    let recording = recordings::store_recording_with_segmentation(
        &store.recordings_path(),
        &bytes,
        &one_track,
        stored,
    );
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    let opening = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: page.recordings[0].id,
        }),
    ));
    assert_eq!(
        self::after_loading(&mut harness, opening.operation),
        HistoryOpenState::AwaitingTrackSettings
    );
    harness.run_steps(3);
    harness
        .get_by_label("Recalculate with current settings")
        .click();
    harness.run_steps(2);
    assert!(matches!(
        self::after_loading(&mut harness, opening.operation),
        HistoryOpenState::Ready { .. }
    ));
    harness
        .state_mut()
        .processing_config
        .track_layout
        .track_split_gap = longer_gap;
    let path = dir.path().join("disk-copy.gtd");
    std::fs::write(&path, bytes).unwrap();
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(TestDroppedFile::path(path)));
    harness.step();
    crate::app::test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(
        &mut harness,
    );
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 2));
    {
        let mut shared = harness.state().shared.borrow_mut();
        let files = shared.loaded_files.view();
        let disk = files.get(1).unwrap();
        assert_eq!(disk.history().db_ref(), Some(&recording));
        assert_eq!(disk.file().tracks.len(), 1);
        assert!(disk.history().track_snapshot().is_none());
        shared
            .tree
            .hide_track(TrackRef::new(FileIdx::new(1), TrackIdx::new(0)));
    }
    let job = session::start_query(&mut harness, "points");
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, job),
        QueryJobState::Completed {
            queries: vec![QueryCounts {
                matches: 2,
                tracks_with_matches: 2,
                skipped: 0
            }],
        },
    );
    harness.run_steps(3);
    harness.state().history.load_recording_ui_state(recording);
    let Response::RecordingUiStateLoaded(Ok((_, state))) =
        recordings::next_visibility_response(&mut harness)
    else {
        panic!("expected the stored layout's visibility");
    };
    assert_eq!(state, RecordingUiState::default());
}

#[test]
fn desktop_history_recalculation_clears_visibility_before_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let recording = recordings::store_recording_with_segmentation(
        &store.recordings_path(),
        &recording_fixtures::two_track_gtd_bytes(),
        &ui_tests::two_live_track_ranges(),
        recordings::segmentation(),
    );
    let reporter = UiStateVersionReporter::default();
    store
        .open_recordings()
        .unwrap()
        .set_recording_ui_state(
            &recording,
            &RecordingUiState::with_hidden_track_numbers([1]),
            &reporter,
        )
        .unwrap();

    let mut harness = self::owner_app(&store);
    self::await_page(&mut harness, UNFILTERED);
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1));
    let original = session::start_query(&mut harness, "points");
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, original),
        session::completed_single_track_query(1)
    );

    let merge_gap = TimeDelta::hours(2);
    harness
        .state_mut()
        .processing_config
        .track_layout
        .track_split_gap = merge_gap;
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| harness.state().pending_resegment.is_some()));
    harness.run_steps(3);
    harness
        .get_by_label("Recalculate with current settings")
        .click();
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 2));

    let db = store.open_recordings().unwrap();
    let rows = db.stored_track_table(&recording).unwrap();
    assert_eq!(
        rows,
        [TrackRange {
            start: 0,
            end: 20,
            state: TrackState::Live
        }]
    );
    assert_eq!(
        db.recording_ui_state(&recording, &reporter).unwrap(),
        RecordingUiState::default()
    );
    drop(db);
    drop(harness);

    let mut reopened = self::owner_app(&store);
    reopened
        .state_mut()
        .processing_config
        .track_layout
        .track_split_gap = merge_gap;
    self::await_page(&mut reopened, UNFILTERED);
    reopened.state().history.open(recording);
    assert!(reopened.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1));
    let job = session::start_query(&mut reopened, "points");
    assert_eq!(
        session::wait_for_query_to_stop(&mut reopened, job),
        session::completed_single_track_query(1)
    );
}

#[rstest]
#[case::use_stored_layout(false)]
#[case::recalculate(true)]
fn reopening_after_a_middle_track_purge_uses_the_selected_layout(#[case] recalculate: bool) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let path = store.recordings_path();
    let recording = recordings::store_recording_with_shelved_middle_track(&path, None);
    let mut db = store.open_recordings().unwrap();
    let (original, _) = HistoryAccess::lock()
        .load_recording_with_snapshot(&db, &recording)
        .unwrap();
    let file = NavFile::read(original.bytes.as_slice()).unwrap();
    let fix_times = |range: Range<usize>| {
        file.nav_points()[range]
            .iter()
            .map(|fix| fix.fix.time.effective())
            .collect::<Vec<_>>()
    };
    let first = fix_times(0..5);
    let last = fix_times(10..15);
    let hidden = RecordingUiState::with_hidden_track_numbers([3]);
    let reporter = UiStateVersionReporter::default();
    db.set_recording_ui_state(&recording, &hidden, &reporter)
        .unwrap();
    drop(db);

    let worker = recordings::worker_on(&path);
    worker.delete_shelved_tracks(DeleteShelvedTracksScope::OneRecording(recording.clone()));
    assert!(matches!(
        recordings::next_response(&worker),
        Response::Mutated {
            op: DbOp::TracksDeleted { count: 1, .. },
            result: Ok(()),
        }
    ));
    worker.shutdown();

    let mut harness = self::owner_app(&store);
    if recalculate {
        harness
            .state_mut()
            .processing_config
            .track_layout
            .track_split_gap = TimeDelta::seconds(30);
    }
    self::await_page(&mut harness, UNFILTERED);
    harness.state().history.open(recording);
    if recalculate {
        let label = "Recalculate with current settings";
        assert!(harness.step_until(|harness| harness.query_by_label(label).is_some()));
        harness.get_by_label(label).click();
    }
    assert!(harness.step_until(|harness| !harness.state().shared.borrow().loaded_files.is_empty()));

    let shared = harness.state().shared.borrow();
    let entry = shared.loaded_files.view().get(0).unwrap();
    let tracks: Vec<_> = entry
        .file()
        .tracks
        .iter()
        .map(|track| {
            (
                track.metadata.index,
                track
                    .points
                    .iter()
                    .map(|fix| fix.tpv.time().utc())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let snapshot = entry.history().track_snapshot().unwrap();
    assert!(snapshot.is_current());
    if recalculate {
        assert_eq!(tracks, [(1, [first, last].concat())]);
        assert_eq!(
            snapshot.rows(),
            [TrackRange {
                start: 0,
                end: 10,
                state: TrackState::Live,
            }]
        );
        return;
    }
    assert_eq!(tracks, [(1, first), (3, last)]);
    assert_eq!(
        snapshot.rows(),
        [
            TrackRange {
                start: 0,
                end: 5,
                state: TrackState::Live,
            },
            TrackRange {
                start: 5,
                end: 5,
                state: TrackState::Deleted,
            },
            TrackRange {
                start: 5,
                end: 10,
                state: TrackState::Live,
            },
        ]
    );
    assert_eq!(
        shared
            .tree
            .track_node(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)))
            .unwrap()
            .check,
        CheckState::Off,
    );
}

#[rstest]
#[case::no_stored_rows(CheckedHistoryLayout::Empty)]
#[case::deleted_middle_track(CheckedHistoryLayout::Tombstone)]
#[case::inconsistent_track_columns(CheckedHistoryLayout::Damaged)]
#[case::out_of_bounds_range(CheckedHistoryLayout::InvalidCoverage)]
#[case::misplaced_tombstone(CheckedHistoryLayout::InvalidTombstone)]
fn a_checked_history_open_requires_the_stored_layout_when_present(
    #[case] layout: CheckedHistoryLayout,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let start = ui_tests::base_time();
    let mut nav = NavFileBuilder::new().open();
    for minute in [0, 1, 10] {
        nav.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(start + TimeDelta::minutes(minute)))
                .lat(Angle::degrees(51.5))
                .lon(Angle::degrees(-0.1))
                .build(),
        );
    }
    let mut nav = nav.finish().unwrap();
    nav.nav_points_mut().swap(1, 2);
    let mut bytes = Vec::new();
    nav.write(&mut bytes).unwrap();
    let tracks = match layout {
        CheckedHistoryLayout::Empty => Vec::new(),
        CheckedHistoryLayout::InvalidCoverage => vec![TrackRange {
            start: 0,
            end: 4,
            state: TrackState::Live,
        }],
        CheckedHistoryLayout::InvalidTombstone => vec![
            TrackRange {
                start: 0,
                end: 1,
                state: TrackState::Live,
            },
            TrackRange {
                start: 0,
                end: 0,
                state: TrackState::Deleted,
            },
            TrackRange {
                start: 1,
                end: 2,
                state: TrackState::Live,
            },
            TrackRange {
                start: 2,
                end: 3,
                state: TrackState::Shelved,
            },
        ],
        CheckedHistoryLayout::Tombstone | CheckedHistoryLayout::Damaged => vec![
            TrackRange {
                start: 0,
                end: 1,
                state: TrackState::Live,
            },
            TrackRange {
                start: 1,
                end: 2,
                state: TrackState::Live,
            },
            TrackRange {
                start: 2,
                end: 3,
                state: TrackState::Shelved,
            },
        ],
    };
    let recording = recordings::store_recording_with_segmentation(
        &store.recordings_path(),
        &bytes,
        &tracks,
        recordings::segmentation(),
    );
    if matches!(layout, CheckedHistoryLayout::Damaged) {
        let file = Hdf5File::open_rw(store.recordings_path()).unwrap();
        let group = file
            .group(&format!(
                "by_identity/{}/{}/__geotrace_tracks__",
                gt_store::identity_group_name(&recording.identity),
                recording.group_name,
            ))
            .unwrap();
        group.delete("end").unwrap();
        group
            .create_dataset("end", |dataset| {
                dataset.with_shape(&[1]).with_u64_data(&[1]);
            })
            .unwrap();
        file.commit().unwrap();
    }
    let mut harness = self::owner_app(&store);
    self::await_page(&mut harness, UNFILTERED);
    if matches!(layout, CheckedHistoryLayout::Tombstone) {
        let db = store.open_recordings().unwrap();
        let original = HistoryAccess::lock()
            .read_track_snapshot(&db, &recording)
            .unwrap();
        harness.state().history.delete_tracks(original, vec![1]);
        let deleted = recordings::next_response(&harness.state().history);
        assert!(matches!(
            deleted,
            Response::Mutated {
                op: DbOp::TracksDeleted { .. },
                result: Ok(())
            }
        ));
        harness.state_mut().handle_history_response(deleted);
        let stored = db.stored_track_table(&recording).unwrap();
        assert_eq!(
            stored.iter().map(|row| row.state).collect::<Vec<_>>(),
            [TrackState::Live, TrackState::Deleted, TrackState::Shelved]
        );
    }
    let page = self::await_page(&mut harness, UNFILTERED);
    let opening = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: page.recordings[0].id,
        }),
    ));
    let outcome = self::after_loading(&mut harness, opening.operation);
    match layout {
        CheckedHistoryLayout::Empty => {
            assert!(matches!(outcome, HistoryOpenState::Ready { .. }));
            assert_eq!(harness.state().shared.borrow().loaded_files.len(), 1);
        }
        CheckedHistoryLayout::Tombstone => {
            assert!(matches!(outcome, HistoryOpenState::Ready { .. }));
            let shared = harness.state().shared.borrow();
            let entry = shared.loaded_files.view().get(0).unwrap();
            assert_eq!(entry.file().tracks.len(), 1);
            assert_eq!(entry.file().tracks[0].metadata.index, 1);
            assert_eq!(entry.file().tracks[0].points.len(), 1);
            assert_eq!(entry.file().tracks[0].points[0].tpv.time().utc(), start);
            let snapshot = entry.history().track_snapshot().unwrap();
            assert!(snapshot.is_current_with_membership());
            assert_eq!(
                snapshot
                    .rows()
                    .iter()
                    .map(|row| row.state)
                    .collect::<Vec<_>>(),
                [TrackState::Live, TrackState::Deleted, TrackState::Shelved]
            );
        }
        CheckedHistoryLayout::Damaged
        | CheckedHistoryLayout::InvalidCoverage
        | CheckedHistoryLayout::InvalidTombstone => {
            assert_eq!(
                outcome,
                HistoryOpenState::Failed {
                    code: ErrorCode::HistoryTrackLayoutUnavailable,
                },
            );
            assert!(harness.state().shared.borrow().loaded_files.is_empty());

            harness.state().history.open(recording);
            assert!(
                harness
                    .step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1)
            );
            let shared = harness.state().shared.borrow();
            let entry = shared.loaded_files.view().get(0).unwrap();
            assert!(entry.history().track_snapshot().is_none());
            assert_eq!(entry.file().tracks.len(), 3);
        }
    }
}

enum CheckedHistoryLayout {
    Damaged,
    Empty,
    InvalidCoverage,
    InvalidTombstone,
    Tombstone,
}

#[rstest]
#[case::damaged(ZeroFixTrackTable::Damaged)]
#[case::missing(ZeroFixTrackTable::Missing)]
#[case::present(ZeroFixTrackTable::Present)]
fn a_zero_fix_recording_uses_snapshots_for_empty_or_missing_tables(
    #[case] table: ZeroFixTrackTable,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let file = NavFileBuilder::new()
        .with_identity("dev")
        .unwrap()
        .open()
        .finish()
        .unwrap();
    let mut bytes = Vec::new();
    file.write(&mut bytes).unwrap();
    let path = store.recordings_path();
    let recording = recordings::store_recording(&path, &bytes, &[]);
    {
        let file = Hdf5File::open_rw(path).unwrap();
        let group = file
            .group(&format!(
                "by_identity/{}/{}",
                gt_store::identity_group_name(&recording.identity),
                recording.group_name,
            ))
            .unwrap();
        match table {
            ZeroFixTrackTable::Damaged => group
                .group("__geotrace_tracks__")
                .unwrap()
                .delete("end")
                .unwrap(),
            ZeroFixTrackTable::Missing => group.delete("__geotrace_tracks__").unwrap(),
            ZeroFixTrackTable::Present => {}
        }
        file.commit().unwrap();
    }
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    let damaged = matches!(table, ZeroFixTrackTable::Damaged);
    if damaged {
        let opening = self::open_status(&session::execute(
            &mut harness,
            Request::OpenHistoryRecording(OpenHistoryRecording {
                recording: page.recordings[0].id,
            }),
        ));
        assert_eq!(
            self::after_loading(&mut harness, opening.operation),
            HistoryOpenState::Failed {
                code: ErrorCode::HistoryTrackLayoutUnavailable,
            }
        );
    }
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1));
    let history_verified = {
        let shared = harness.state().shared.borrow();
        let entry = shared.loaded_files.view().get(0).unwrap();
        assert_eq!(entry.history().db_ref(), Some(&recording));
        assert!(entry.file().tracks.is_empty());
        entry.history().track_snapshot().is_some()
    };

    let disk = dir.path().join("empty.gtd");
    std::fs::write(&disk, bytes).unwrap();
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(TestDroppedFile::path(disk)));
    harness.step();
    harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 2));
    let shared = harness.state().shared.borrow();
    let entry = shared.loaded_files.view().get(1).unwrap();
    assert_eq!(entry.history().db_ref(), Some(&recording));
    assert!(entry.file().tracks.is_empty());
    let disk_verified = entry.history().track_snapshot().is_some();
    assert_eq!((history_verified, disk_verified), (!damaged, !damaged));
}

enum ZeroFixTrackTable {
    Damaged,
    Missing,
    Present,
}

#[rstest]
#[case::unchanged_layout(TimeDelta::minutes(5), 2)]
#[case::merged_layout(TimeDelta::hours(2), 1)]
fn history_open_status_checks_the_layout_after_apply_processing(
    #[case] split_gap: TimeDelta,
    #[case] tracks_after_processing: usize,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    recordings::store_recording_with_segmentation(
        &store.recordings_path(),
        &recording_fixtures::two_track_gtd_bytes(),
        &ui_tests::two_live_track_ranges(),
        recordings::segmentation(),
    );
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    let open = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: page.recordings[0].id,
        }),
    ));
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1));
    harness
        .state_mut()
        .processing_config
        .track_layout
        .track_split_gap = split_gap;
    assert_eq!(
        session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
        Reply::ProcessingApplied,
    );
    harness.run_steps(3);
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .get(0)
            .unwrap()
            .tracks
            .len(),
        tracks_after_processing,
    );
    let status = self::after_loading(&mut harness, open.operation);
    if tracks_after_processing == 2 {
        assert!(matches!(status, HistoryOpenState::Ready { .. }));
    } else {
        assert_eq!(status, HistoryOpenState::Expired);
    }
}

#[rstest]
#[case::selected_track(HistoryViewChange::UnloadSelectedTrack)]
#[case::unrelated_recording(HistoryViewChange::UnloadUnrelatedRecording)]
fn history_open_status_checks_the_selected_recording_after_unload(
    #[case] change: HistoryViewChange,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let selected = recordings::store_recording_with_segmentation(
        &store.recordings_path(),
        &recording_fixtures::two_track_gtd_bytes(),
        &ui_tests::two_live_track_ranges(),
        recordings::segmentation(),
    );
    let unrelated = recordings::insert_recording_as_one_whole_file_track(
        &mut store.open_recordings().unwrap(),
        "unrelated",
        &recordings::bytes_starting_at(1_000_000, 2),
    );
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    harness.state().history.open(unrelated);
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 1));
    let selected_id = page
        .recordings
        .iter()
        .find(|entry| entry.identity == selected.identity)
        .unwrap()
        .id;
    let open = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: selected_id,
        }),
    ));
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 2));
    let adopted_file = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .view()
        .get(1)
        .unwrap()
        .id();
    match change {
        HistoryViewChange::UnloadSelectedTrack => {
            harness
                .state()
                .shared
                .borrow_mut()
                .tree
                .expand_file(FileIdx::new(1));
            harness.run_steps(2);
            harness
                .bottommost_matching(By::new().label_contains("#2 "))
                .click_secondary();
            harness.run_steps(2);
            harness.get_by_label("Unload").click();
        }
        HistoryViewChange::UnloadUnrelatedRecording => {
            harness.state().shared.borrow_mut().tree.pending_unload =
                Some(vec![NodeKey::File(FileIdx::new(0))]);
        }
    }
    harness.run_steps(3);
    let state = self::after_loading(&mut harness, open.operation);
    let shared = harness.state().shared.borrow();
    let file = shared
        .loaded_files
        .view()
        .entry_for_id(adopted_file)
        .unwrap();
    let snapshot = file.history().track_snapshot().unwrap();
    assert!(snapshot.is_current_with_membership());
    assert_eq!(snapshot.rows(), ui_tests::two_live_track_ranges());
    match change {
        HistoryViewChange::UnloadSelectedTrack => {
            assert_eq!(file.file().tracks.len(), 1);
            assert_eq!(state, HistoryOpenState::Expired);
        }
        HistoryViewChange::UnloadUnrelatedRecording => {
            assert_eq!(shared.loaded_files.len(), 1);
            assert_eq!(file.file().tracks.len(), 2);
            assert_eq!(
                state,
                HistoryOpenState::Ready {
                    recording: RecordingReference {
                        session: open.operation.session,
                        number: adopted_file.into(),
                    },
                    reused: false,
                },
            );
        }
    }
}

enum HistoryViewChange {
    UnloadSelectedTrack,
    UnloadUnrelatedRecording,
}

#[rstest]
#[case::removed(HistoryChange::Removed, HistoryOpenState::Expired)]
#[case::replaced(HistoryChange::Replaced, HistoryOpenState::Expired)]
#[case::unreadable(HistoryChange::Unreadable, HistoryOpenState::Failed {code: ErrorCode::HistoryOpenFailed})]
fn an_open_reports_errors_reading_the_selected_history_recording(
    #[case] change: HistoryChange,
    #[case] expected: HistoryOpenState,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let removed = recordings::insert_recording_as_one_whole_file_track(
        &mut store.open_recordings().unwrap(),
        "removed",
        &recordings::bytes_starting_at(1_000_000, 2),
    );
    let mut harness = self::read_only_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    match change {
        HistoryChange::Removed => store
            .open_recordings()
            .unwrap()
            .delete_batch(&[removed])
            .unwrap(),
        HistoryChange::Replaced => self::replace_history_payload(&store, &removed),
        HistoryChange::Unreadable => std::fs::write(store.recordings_path(), []).unwrap(),
    }
    let opening = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: page.recordings[0].id,
        }),
    ));
    assert_eq!(
        self::after_loading(&mut harness, opening.operation),
        expected
    );
    assert!(harness.state().shared.borrow().loaded_files.is_empty());
    if matches!(change, HistoryChange::Removed) {
        assert!(
            self::await_page(&mut harness, UNFILTERED)
                .recordings
                .is_empty()
        );
    }
}

enum HistoryChange {
    Removed,
    Replaced,
    Unreadable,
}

#[test]
fn a_hidden_session_restores_saved_visibility_before_ready_and_loads_attached_logs() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let mut db = store.open_recordings().unwrap();
    let selected = recordings::insert_recording_as_one_whole_file_track(
        &mut db,
        "recording",
        &recordings::bytes_starting_at(1_000_000, 2),
    );
    db.set_recording_ui_state(
        &selected,
        &RecordingUiState::with_hidden_track_numbers([1]),
        &UiStateVersionReporter::default(),
    )
    .unwrap();
    self::attach_saved_log(&mut db, &selected);
    drop(db);
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    let open = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: page.recordings[0].id,
        }),
    ));
    let response = recordings::next_response(&harness.state().history);
    assert!(matches!(&response, Response::Opened { result: Ok(_), .. }));
    harness.state_mut().handle_history_response(response);

    let deadline = Instant::now() + Duration::from_secs(5);
    {
        let _access = HistoryAccess::lock();
        while harness.state().shared.borrow().loaded_files.is_empty() {
            harness::step_minimized_logic(&mut harness);
            assert!(Instant::now() < deadline, "timed out loading the recording");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(harness.state().history.restoring_session_recordings());
        assert!(harness.state().logs.is_empty());
        assert!(matches!(
            self::open_status(&session::execute(
                &mut harness,
                Request::GetHistoryOpenStatus(GetHistoryOpenStatus {
                    operation: open.operation,
                }),
            ))
            .state,
            HistoryOpenState::Ready { .. }
        ));
        let job = session::start_query(&mut harness, "points");
        assert_eq!(
            session::wait_for_query_to_stop(&mut harness, job),
            session::completed_single_track_query(0)
        );
    }

    while harness.state().logs.is_empty() {
        harness::step_minimized_logic(&mut harness);
        assert!(
            Instant::now() < deadline,
            "timed out loading the attachment"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!harness.state().history.restoring_session_recordings());
    assert!(harness.state().loader.loading_jobs.is_empty());
}

#[test]
fn a_history_open_leaves_saved_logs_unloaded_when_a_disk_copy_shares_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let mut db = store.open_recordings().unwrap();
    let selected = recordings::insert_recording_as_one_whole_file_track(
        &mut db,
        "recording",
        &recordings::bytes_starting_at(1_000_000, 2),
    );
    self::attach_saved_log(&mut db, &selected);
    drop(db);
    let bytes = recordings::bytes_starting_at(2_000_000, 2);
    let path = dir.path().join("earlier-copy.gtd");
    std::fs::write(&path, &bytes).unwrap();
    let file = gt_loader::load_file(&path).unwrap();
    let history = FileHistory::recording(
        selected.identity.clone(),
        gt_store::extract_meta(&bytes).unwrap(),
        Some(selected),
    );
    let mut harness = self::owner_app(&store);
    {
        let mut shared = harness.state().shared.borrow_mut();
        shared.loaded_files.push(file, history);
        shared.sync_tree_from_loaded_files();
        let shared = &mut *shared;
        shared.plot_state.rebuild_all(&shared.loaded_files);
    }
    let page = self::await_page(&mut harness, UNFILTERED);
    let opening = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: page.recordings[0].id,
        }),
    ));
    assert!(matches!(
        self::after_loading(&mut harness, opening.operation),
        HistoryOpenState::Ready { reused: false, .. }
    ));
    assert!(harness.step_until(
        |harness| !harness.state().history.restoring_session_recordings()
            && harness.state().loader.loading_jobs.is_empty()
    ));
    assert_eq!(harness.state().shared.borrow().loaded_files.len(), 2);
    assert!(harness.state().logs.is_empty());
}

fn attach_saved_log(db: &mut Recordings, recording: &DatabaseRef) {
    db.attach_log(
        recording,
        &LogToAttach {
            name: "saved.log",
            text: "1970-01-01T00:00:01Z svc: saved log\n",
            settings: StoredLogSettings {
                association_window: None,
                filters: Default::default(),
                year_reference: None,
                year_resolution: StoredYearResolution::Infer,
            },
        },
    )
    .unwrap();
}

enum PendingHistoryChange {
    ReplacePayload,
    UnshelveTrack,
}

#[rstest]
#[case::replace_payload(PendingHistoryChange::ReplacePayload)]
#[case::unshelve_track(PendingHistoryChange::UnshelveTrack)]
fn a_history_open_expires_when_the_recording_changes_before_adoption(
    #[case] change: PendingHistoryChange,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let selected = recordings::insert_recording_as_one_whole_file_track(
        &mut store.open_recordings().unwrap(),
        "recording",
        &recordings::bytes_starting_at(1_000_000, 2),
    );
    if matches!(change, PendingHistoryChange::UnshelveTrack) {
        store
            .open_recordings()
            .unwrap()
            .set_tracks_shelved(&selected, &[0], true)
            .unwrap();
    }
    let mut harness = self::owner_app(&store);
    let page = self::await_page(&mut harness, UNFILTERED);
    let opening = self::open_status(&session::execute(
        &mut harness,
        Request::OpenHistoryRecording(OpenHistoryRecording {
            recording: page.recordings[0].id,
        }),
    ));
    let response = recordings::next_response(&harness.state().history);
    let Response::Opened {
        result: Ok(opened), ..
    } = &response
    else {
        panic!("expected a recording read");
    };
    let observation = opened.track_snapshot.clone();
    harness.state_mut().handle_history_response(response);
    match change {
        PendingHistoryChange::ReplacePayload => self::replace_history_payload(&store, &selected),
        PendingHistoryChange::UnshelveTrack => {
            harness
                .state()
                .history
                .set_tracks_shelved(observation.clone(), vec![0], false);
            let unshelved = recordings::next_response(&harness.state().history);
            assert!(matches!(
                unshelved,
                Response::Mutated {
                    op: DbOp::TracksUnshelved { .. },
                    result: Ok(())
                }
            ));
            harness.state_mut().handle_history_response(unshelved);
            assert!(observation.is_current());
        }
    }
    assert_eq!(
        self::after_loading(&mut harness, opening.operation),
        HistoryOpenState::Expired
    );
    assert!(harness.state().shared.borrow().loaded_files.is_empty());
}

fn replace_history_payload(store: &Store, recording: &DatabaseRef) {
    let bytes = recordings::bytes_starting_at(2_000_000, 2);
    let meta = gt_store::extract_meta(&bytes).unwrap();
    let rows = [TrackRange {
        start: 0,
        end: 2,
        state: TrackState::Live,
    }];
    store
        .open_recordings()
        .unwrap()
        .replace_recording_in_place(recording, &meta, &rows, recordings::segmentation(), &bytes)
        .unwrap();
}

fn open_status(reply: &Reply) -> HistoryOpenStatus {
    let Reply::HistoryOpen { operation } = reply else {
        panic!("expected History open, got {reply:?}");
    };
    *operation
}

fn after_loading(harness: &mut Harness<'_, App>, operation: HistoryOpenId) -> HistoryOpenState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = self::open_status(&session::execute(
            harness,
            Request::GetHistoryOpenStatus(GetHistoryOpenStatus { operation }),
        ))
        .state;
        if !matches!(state, HistoryOpenState::Reading | HistoryOpenState::Loading) {
            return state;
        }
        assert!(Instant::now() < deadline, "timed out opening History");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn history_discovery_distinguishes_opening_list_failure_and_an_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    assert_eq!(
        session::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
        Reply::from(ErrorCode::HistoryNotReady)
    );
    databases
        .send(harness::storage_opened_in(
            &store,
            &harness.ctx,
            &harness.state().pending_writes,
        ))
        .unwrap();
    assert_eq!(
        session::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
        Reply::from(ErrorCode::HistoryNotReady)
    );

    let app = harness.state_mut();
    let Response::Listed { request, .. } = recordings::next_response(&app.history) else {
        panic!("expected the pending History list");
    };
    app.handle_history_response(Response::Listed {
        request,
        entries: Err(DbError::Busy),
    });
    {
        let _access = HistoryAccess::lock();
        assert_eq!(
            session::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
            Reply::from(ErrorCode::HistoryListingFailed)
        );
        harness.state_mut().history_window.open = true;
        harness.run_steps(3);
        harness.get_by_label_contains("Failed to load history");
        harness.state_mut().history_window.open = false;
    }

    let obsolete = recordings::next_response(&harness.state().history);
    harness.state_mut().invalidate_history_listing();
    {
        let _access = HistoryAccess::lock();
        assert_eq!(
            session::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
            Reply::from(ErrorCode::HistoryNotReady)
        );
        harness.state_mut().handle_history_response(obsolete);
        assert!(matches!(
            harness.state().history_listing.view(),
            RecordingListing::Pending
        ));
        harness.state_mut().history_window.open = true;
        harness.run_steps(3);
        harness.get_by_label_contains("Failed to load history");
        harness.state_mut().history_window.open = false;
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(
        harness.state().history_listing.view(),
        RecordingListing::Ready { .. }
    ) {
        harness::step_minimized_logic(&mut harness);
        assert!(Instant::now() < deadline, "timed out listing History");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        self::await_page(&mut harness, UNFILTERED),
        HistoryRecordings {
            recordings: Vec::new(),
            next_cursor: None
        }
    );

    let _access = HistoryAccess::lock();
    harness.state_mut().history_window.open = true;
    harness.run_steps(3);
    harness.get_by_label("No recordings in history yet");
    assert!(
        harness
            .query_by_label_contains("Failed to load history")
            .is_none()
    );
}

#[test]
fn history_pages_list_latest_starts_first_and_missing_ranges_last() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    {
        let mut db = store.open_recordings().unwrap();
        for (identity, seconds, points) in [
            ("oldest", -10, 24),
            ("untimed", 0, 0),
            ("newest", 10, 2),
            ("epoch", 0, 1),
        ] {
            let bytes = recordings::bytes_starting_at(seconds, points);
            let meta = gt_store::extract_meta(&bytes).unwrap();
            let track = (meta.nav_point_count > 0).then_some(TrackRange {
                start: 0,
                end: meta.nav_point_count,
                state: TrackState::Live,
            });
            db.insert(
                identity,
                &meta,
                track.as_slice(),
                recordings::segmentation(),
                &bytes,
            )
            .unwrap();
        }
    }

    let mut harness = self::owner_app(&store);
    let mut request = ListHistoryRecordings {
        limit: Some(NonZeroUsize::MIN),
        ..UNFILTERED
    };
    for (identity, start_us) in [
        ("newest", Some(10_000_000)),
        ("epoch", Some(0)),
        ("oldest", Some(-10_000_000)),
        ("untimed", None),
    ] {
        let page = self::await_page(&mut harness, request);
        let [entry] = page.recordings.as_slice() else {
            panic!("expected one History recording per page");
        };
        assert_eq!(entry.identity, identity);
        assert_eq!(entry.time_range.map(|range| range.start_us()), start_us);
        request.cursor = page.next_cursor;
    }
    assert_eq!(request.cursor, None);
}

#[test]
fn a_stored_recording_spanning_midnight_is_discoverable_after_reopening_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let day = NaiveDate::from_ymd_opt(2025, 2, 2).unwrap();
    let midnight = day.and_hms_opt(0, 0, 0).unwrap().and_utc();
    self::store_recording_across_midnight(&store, midnight);
    let request = ListHistoryRecordings {
        dates: Some(HistoryDates {
            from: Some(day),
            through: Some(day),
        }),
        ..UNFILTERED
    };

    let mut owner = self::owner_app(&store);
    let first = self::await_page(&mut owner, request);
    assert_eq!(first.recordings.len(), 1);
    let restored = &first.recordings[0];
    assert_eq!(restored.identity, "dev");
    assert_eq!(restored.title.as_deref(), Some("Nattur 🌙"));
    assert_eq!(
        (
            restored.nav_points,
            restored.total_tracks,
            restored.shelved_tracks
        ),
        (2, 1, 1)
    );
    let range = restored.time_range.unwrap();
    assert_eq!(range.start_us(), midnight.timestamp_micros() - 500_001);
    assert_eq!(range.end_us(), midnight.timestamp_micros() + 500_001);
    assert!(!owner.state().history_window.open);
    assert!(owner.state().shared.borrow().loaded_files.is_empty());
    drop(owner);

    let before = std::fs::read(store.recordings_path()).unwrap();
    let mut reader = self::read_only_app(&store);
    assert_eq!(self::await_page(&mut reader, request), first);
    assert!(!reader.state().history_window.open);
    drop(reader);
    assert_eq!(std::fs::read(store.recordings_path()).unwrap(), before);
}

#[test]
fn history_discovery_reports_when_read_only_startup_has_no_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let mut reader = self::read_only_app(&store);
    assert_eq!(
        session::execute(&mut reader, Request::ListHistoryRecordings(UNFILTERED)),
        Reply::from(ErrorCode::HistoryUnavailable)
    );
    assert!(!store.recordings_path().exists());
}

#[test]
fn the_history_toolbar_preserves_the_cursor_on_close_and_expires_it_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    {
        let mut db = store.open_recordings().unwrap();
        for (identity, start) in [("first", 1_000_000), ("second", 2_000_000)] {
            recordings::insert_recording_as_one_whole_file_track(
                &mut db,
                identity,
                &recordings::bytes_starting_at(start, 2),
            );
        }
    }
    let mut harness = self::owner_app(&store);
    harness.step();
    harness.get_by_label(ICON_CLOCK_COUNTER_CLOCKWISE).click();
    harness.run_steps(2);
    assert!(harness.state().history_window.open);

    let request = ListHistoryRecordings {
        limit: Some(NonZeroUsize::MIN),
        ..UNFILTERED
    };
    let first = self::await_page(&mut harness, request);
    let cursor = first.next_cursor.expect("two stored recordings");
    let continuation = ListHistoryRecordings {
        cursor: Some(cursor),
        ..request
    };
    let second = self::await_page(&mut harness, continuation);

    harness.get_by_label(ICON_CLOCK_COUNTER_CLOCKWISE).click();
    harness.run_steps(2);
    assert!(!harness.state().history_window.open);
    assert_eq!(
        session::execute(&mut harness, Request::ListHistoryRecordings(continuation)),
        Reply::HistoryRecordings { page: second }
    );

    harness.get_by_label(ICON_CLOCK_COUNTER_CLOCKWISE).click();
    harness.run_steps(2);
    assert!(harness.state().history_window.open);
    assert_eq!(
        session::execute(&mut harness, Request::ListHistoryRecordings(continuation)),
        Reply::from(ErrorCode::InvalidHistoryCursor)
    );
    let refreshed = self::await_page(&mut harness, request);
    assert_eq!(refreshed.recordings, first.recordings);
    assert_ne!(
        refreshed.next_cursor.expect("two stored recordings"),
        cursor
    );
}

#[rstest]
#[case::mutation(Response::Mutated {
    op: DbOp::RecordingsDeleted { count: 2, reason: DeleteReason::Manual },
    result: Err(DbError::Backend("batch deletion failed".to_owned())),
})]
#[case::auto_prune(Response::AutoPruned(Ok(AutoPruneOutcome::DeleteAttempt(Err(
    DbError::Backend("batch deletion failed".to_owned()),
)))))]
fn a_partial_history_write_expires_the_list_cursor(#[case] completion: Response) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let deleted = {
        let mut db = store.open_recordings().unwrap();
        recordings::insert_recording_as_one_whole_file_track(
            &mut db,
            "kept",
            &recordings::bytes_starting_at(1_000_000, 2),
        );
        recordings::insert_recording_as_one_whole_file_track(
            &mut db,
            "deleted",
            &recordings::bytes_starting_at(2_000_000, 2),
        )
    };
    let mut harness = self::owner_app(&store);
    let request = ListHistoryRecordings {
        limit: Some(NonZeroUsize::MIN),
        ..UNFILTERED
    };
    let original = self::await_page(&mut harness, request);
    let cursor = original.next_cursor.expect("two stored recordings");

    store
        .open_recordings()
        .unwrap()
        .delete_batch(&[deleted])
        .unwrap();
    harness.state_mut().handle_history_response(completion);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::ListHistoryRecordings(ListHistoryRecordings {
                cursor: Some(cursor),
                ..request
            })
        ),
        Reply::from(ErrorCode::InvalidHistoryCursor)
    );
    let fresh = self::await_page(&mut harness, request);
    assert_eq!(fresh.recordings.len(), 1);
    assert_eq!(fresh.recordings[0].identity, "kept");
    assert_eq!(fresh.next_cursor, None);
}

#[test]
fn an_import_with_a_failed_history_write_refreshes_the_listing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let mut harness = self::owner_app(&store);
    assert!(
        self::await_page(&mut harness, UNFILTERED)
            .recordings
            .is_empty()
    );

    recordings::insert_recording_as_one_whole_file_track(
        &mut store.open_recordings().unwrap(),
        "imported",
        &recording_fixtures::minimal_gtd_bytes(),
    );
    let mut loaded = recordings::recording_outcome("imported.gtd");
    loaded.history_write_attempted = true;
    let complete = harness
        .state_mut()
        .loader
        .controlled_recording_load_for_test("imported.gtd", RecordingOperationOrigin::Independent);
    complete(Ok(loaded));
    let refreshed = self::await_page(&mut harness, UNFILTERED);
    assert_eq!(refreshed.recordings.len(), 1);
    assert_eq!(refreshed.recordings[0].identity, "imported");
}

fn store_recording_across_midnight(store: &Store, midnight: DateTime<Utc>) {
    let mut recorder = NavFileBuilder::new()
        .with_title("Nattur 🌙")
        .unwrap()
        .open();
    for offset_us in [-500_001, 500_001] {
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(
                    midnight + TimeDelta::microseconds(offset_us),
                ))
                .lat(Angle::degrees(55.7))
                .lon(Angle::degrees(12.6))
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();
    let reference = recordings::store_recording_with_segmentation(
        &store.recordings_path(),
        &bytes,
        &[TrackRange {
            start: 0,
            end: 2,
            state: TrackState::Live,
        }],
        crate::app::loader::stored_segmentation_from_config(&SegmentationConfig::default()),
    );
    store
        .open_recordings()
        .unwrap()
        .set_tracks_shelved(&reference, &[0], true)
        .unwrap();
}

fn owner_app(store: &Store) -> Harness<'static, App> {
    let (harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    databases
        .send(harness::storage_opened_in(
            store,
            &harness.ctx,
            &harness.state().pending_writes,
        ))
        .unwrap();
    harness
}

fn read_only_app(store: &Store) -> Harness<'static, App> {
    let (harness, databases) =
        harness::app_with_the_databases_still_opening_for(&[], WriteAccess::ReadOnly);
    let history =
        store
            .open_recordings_read_only()
            .unwrap()
            .map_or_else(HistoryWorker::disabled, |db| {
                HistoryWorker::spawn(
                    RecordingsHandle::ReadOnly(db),
                    harness.ctx.clone(),
                    harness.state().pending_writes.clone(),
                )
            });
    databases
        .send(OpenStorage {
            history,
            history_failure: None,
            archive: None,
            geomagnetic_indices: None,
            tec_maps: None,
            solar_flares: None,
            unavailable_archives: UnavailableArchives::default(),
        })
        .unwrap();
    harness
}

fn await_page(harness: &mut Harness<'_, App>, request: ListHistoryRecordings) -> HistoryRecordings {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match session::execute(harness, Request::ListHistoryRecordings(request)) {
            Reply::HistoryRecordings { page } => return page,
            Reply::Error {
                code: ErrorCode::HistoryNotReady,
            } => {
                assert!(Instant::now() < deadline, "timed out waiting for History");
                std::thread::sleep(Duration::from_millis(5));
            }
            reply => panic!("expected a History page, got {reply:?}"),
        }
    }
}
