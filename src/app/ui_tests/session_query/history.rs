use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use egui_kittest::Harness;
use geotrace_sdk::{Angle, NavFileBuilder, NavFix, NavFixTime};
use gt_pending_writes::WriteAccess;
use gt_session_bridge::{
    ErrorCode, HistoryDates, HistoryRecordings, ListHistoryRecordings, Reply, Request,
};
use gt_store::{DbError, HistoryDatabase as _, RecordingsHandle, Store, TrackRange, TrackState};
use gt_track_builder::SegmentationConfig;
use rstest::rstest;

use super::{App, ui_tests};
use crate::app::archive_recovery::UnavailableArchives;
use crate::app::association_batches::RecordingOperationOrigin;
use crate::app::auto_prune::AutoPruneOutcome;
use crate::app::history_db::{DbOp, DeleteReason, HistoryWorker, Response};
use crate::app::storage::OpenStorage;
use crate::app::test_util::recordings;

const UNFILTERED: ListHistoryRecordings = ListHistoryRecordings {
    cursor: None,
    dates: None,
    limit: None,
};

#[test]
fn history_discovery_distinguishes_opening_list_failure_and_an_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    assert_eq!(
        super::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
        Reply::from(ErrorCode::HistoryNotReady)
    );
    databases
        .send(ui_tests::storage_opened_in(
            &store,
            &harness.ctx,
            &harness.state().pending_writes,
        ))
        .unwrap();
    assert_eq!(
        super::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
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
    assert_eq!(
        super::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
        Reply::from(ErrorCode::HistoryListingFailed)
    );

    let app = harness.state_mut();
    let obsolete = recordings::next_response(&app.history);
    app.history_window.invalidate();
    app.handle_history_response(obsolete);
    assert_eq!(
        super::execute(&mut harness, Request::ListHistoryRecordings(UNFILTERED)),
        Reply::from(ErrorCode::HistoryNotReady)
    );
    assert_eq!(
        self::await_page(&mut harness, UNFILTERED),
        HistoryRecordings {
            recordings: Vec::new(),
            next_cursor: None
        }
    );
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
        super::execute(&mut reader, Request::ListHistoryRecordings(UNFILTERED)),
        Reply::from(ErrorCode::HistoryUnavailable)
    );
    assert!(!store.recordings_path().exists());
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
        super::execute(
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
        &ui_tests::minimal_gtd_bytes(),
    );
    let mut loaded = ui_tests::log_load_order::recording_outcome("imported.gtd");
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
    let reference = ui_tests::insert_recording_into(
        store,
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
    let (harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    databases
        .send(ui_tests::storage_opened_in(
            store,
            &harness.ctx,
            &harness.state().pending_writes,
        ))
        .unwrap();
    harness
}

fn read_only_app(store: &Store) -> Harness<'static, App> {
    let (harness, databases) =
        ui_tests::app_with_the_databases_still_opening_for(&[], WriteAccess::ReadOnly);
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
        match super::execute(harness, Request::ListHistoryRecordings(request)) {
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
