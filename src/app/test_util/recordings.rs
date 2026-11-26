//! Recordings written into a real history database, and a `HistoryWorker`
//! spawned on that database.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta};
use egui_kittest::Harness;
use geotrace_sdk::{Angle, NavFileBuilder, NavFix, NavFixTime};
use gt_loaded_files::FileHistory;
use gt_pending_writes::PendingWrites;
use gt_plot::AnalysisConfig;
use gt_store::{
    DatabaseRef, HistoryAccess, HistoryDatabase as _, ReadOnlyRecordings, RecordingDebugTag,
    RecordingEntry, RecordingUiState, Recordings, RecordingsHandle, Store, StoredFixPlacementRule,
    StoredRecording, StoredSegmentation, StoredTrackSnapshot, StoredTrackSplitRule, TrackRange,
    TrackState,
};
use gt_test_utils::history_fixtures::StoredTrackTableFixture;
use gt_test_utils::{SyntheticGtdSpec, recording_fixtures};
use gt_track_builder::SegmentationConfig;
use tempfile::TempDir;

use crate::app::App;
use crate::app::association_batches::AssociationSubmission;
use crate::app::history_db::{HistoryWorker, OpenedRecording, Response};
use crate::app::loader::{self, LoadedRecordingPlacement, LoadedRecordingResult};
use crate::app::test_util::harness;

pub fn recording_outcome(name: &str) -> LoadedRecordingResult {
    let file = gt_loader::load_bytes(&recording_fixtures::minimal_gtd_bytes(), name.to_owned())
        .expect("parse recording");
    let series = gt_plot::prepare_file_series(&file, AnalysisConfig::default());
    LoadedRecordingResult {
        file,
        series,
        history: FileHistory::None,
        history_write_attempted: false,
        applied_current_marker_settings: false,
        placement: LoadedRecordingPlacement::AddAnEntry,
    }
}

pub fn app_with_stored_tracks<'a>(
    tracks: &[TrackRange],
) -> (Harness<'a, App>, TempDir, DatabaseRef) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open_in(dir.path());
    let db_ref = store_recording_with_segmentation(
        &store.recordings_path(),
        &recording_fixtures::two_track_gtd_bytes(),
        tracks,
        loader::stored_segmentation_from_config(&SegmentationConfig::default()),
    );
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    harness.step();
    harness::send_open_storage_and_step(&mut harness, &databases, &store);
    (harness, dir, db_ref)
}

pub fn track_snapshot(path: &Path, recording: &DatabaseRef) -> StoredTrackSnapshot {
    let access = HistoryAccess::lock();
    let db = ReadOnlyRecordings::open_existing_read_only(path).expect("open history");
    access
        .read_track_snapshot(&db, recording)
        .expect("read stored track table")
}

pub fn worker_track_snapshot(
    worker: &HistoryWorker,
    recording: &DatabaseRef,
) -> StoredTrackSnapshot {
    track_snapshot(worker.path().expect("history path"), recording)
}

pub fn synthetic_track_snapshot(
    recording: &DatabaseRef,
    rows: &[TrackRange],
) -> StoredTrackSnapshot {
    let fixture = StoredTrackTableFixture::new(recording, rows);
    track_snapshot(&fixture.path(), recording)
}

pub fn opened_recording(recording: &DatabaseRef, stored: StoredRecording) -> OpenedRecording {
    OpenedRecording {
        track_snapshot: synthetic_track_snapshot(recording, stored.track_rows()),
        stored,
        ui_state: Some(Ok(RecordingUiState::default())),
        matches_reuse_payload: false,
    }
}

impl App {
    pub(in crate::app) fn recording_submission_for_test(&mut self) -> AssociationSubmission {
        self.association_batches.begin_submission(
            self.shared
                .borrow()
                .loaded_files
                .view()
                .entries()
                .map(|entry| entry.id()),
        )
    }
}

/// `point_count` nav points one second apart from `start_secs`.
pub fn bytes_starting_at(start_secs: i64, point_count: usize) -> Vec<u8> {
    gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
        start: DateTime::from_timestamp(start_secs, 0).expect("valid timestamp"),
        point_count,
        ..recording_fixtures::minimal_gtd_spec()
    })
}

/// Twenty nav points one second apart from 2025-05-23 12:53:20 UTC.
pub fn sample_bytes() -> Vec<u8> {
    bytes_starting_at(1_748_000_000, SAMPLE_POINT_COUNT as usize)
}

/// The settings a recording is stored under, matching
/// `SegmentationConfig::default`.
pub fn segmentation() -> StoredSegmentation {
    StoredSegmentation {
        track_split_gap_us: 300_000_000,
        track_split_rule: StoredTrackSplitRule::StepInEitherDirection,
        fix_placement_rule: StoredFixPlacementRule::SynthesizedOrHeadinglessNoFix,
        detect_clock_discontinuities: true,
        clock_discontinuity_sigmas: 5.0,
    }
}

pub fn store_recording(path: &Path, bytes: &[u8], tracks: &[TrackRange]) -> DatabaseRef {
    store_recording_with_segmentation(path, bytes, tracks, segmentation())
}

pub fn store_recording_with_segmentation(
    path: &Path,
    bytes: &[u8],
    tracks: &[TrackRange],
    segmentation: StoredSegmentation,
) -> DatabaseRef {
    let mut db = Recordings::open_or_create(path).expect("open the history database");
    let meta = gt_store::extract_meta(bytes).expect("the recording has metadata");
    db.insert("dev", &meta, tracks, segmentation, bytes)
        .expect("store the recording")
}

/// Store `bytes` under `identity` with a track table of one live track
/// spanning every nav point of the recording.
pub fn insert_recording_as_one_whole_file_track(
    db: &mut Recordings,
    identity: &str,
    bytes: &[u8],
) -> DatabaseRef {
    let meta = gt_store::extract_meta(bytes).expect("parse meta");
    let tracks = [TrackRange {
        start: 0,
        end: meta.nav_point_count,
        state: TrackState::Live,
    }];
    db.insert(identity, &meta, &tracks, segmentation(), bytes)
        .expect("insert")
}

/// Store one recording whose twenty nav points are cut at `bounds` into live
/// tracks.
pub fn seed_recording_cut_at(path: &Path, bounds: &[u64]) {
    let mut tracks = Vec::new();
    let mut start = 0;
    for end in bounds
        .iter()
        .copied()
        .chain(std::iter::once(SAMPLE_POINT_COUNT))
    {
        tracks.push(TrackRange {
            start,
            end,
            state: TrackState::Live,
        });
        start = end;
    }
    store_recording(path, &sample_bytes(), &tracks);
}

/// Store one recording of two ten-point tracks in a database at `path`.
pub fn seed_two_track_recording(path: &Path) {
    seed_recording_cut_at(path, &[10]);
}

pub fn store_recording_with_shelved_middle_track(
    path: &Path,
    debug_time_repair_threshold: Option<TimeDelta>,
) -> DatabaseRef {
    let start = DateTime::from_timestamp(1_748_000_000, 0).unwrap();
    let mut recorder = NavFileBuilder::new().open();
    for offset in [0, 10, 3600] {
        for second in offset..offset + 5 {
            recorder.add_nav_fix(
                NavFix::builder()
                    .time(NavFixTime::Receiver(start + TimeDelta::seconds(second)))
                    .lat(Angle::degrees(51.5))
                    .lon(Angle::degrees(-0.1))
                    .build(),
            );
        }
    }
    let mut recording = recorder.finish().unwrap();
    recording.nav_points_mut().rotate_left(5);
    let mut bytes = Vec::new();
    recording.write(&mut bytes).unwrap();
    let states = [TrackState::Live, TrackState::Shelved, TrackState::Live];
    let rows: [_; 3] = std::array::from_fn(|index| TrackRange {
        start: index as u64 * 5,
        end: (index as u64 + 1) * 5,
        state: states[index],
    });
    let mut meta = gt_store::extract_meta(&bytes).unwrap();
    let mut segmentation = self::segmentation();
    if let Some(threshold) = debug_time_repair_threshold {
        meta.debug_tag = Some(RecordingDebugTag::TimeRepair);
        segmentation.track_split_gap_us = threshold.num_microseconds().unwrap();
    }
    Recordings::open_or_create(path)
        .unwrap()
        .insert("dev", &meta, &rows, segmentation, &bytes)
        .unwrap()
}

pub fn worker_on(path: &Path) -> HistoryWorker {
    let db = Recordings::open_or_create(path).expect("reopen");
    HistoryWorker::spawn(
        RecordingsHandle::Owner(db),
        egui::Context::default(),
        PendingWrites::default(),
    )
}

/// A second sender on a worker's request channel: while it lives the worker's
/// `recv` cannot fail, so its thread stays on its loop.
pub struct HeldOpenWorkerThread(Box<dyn Send>);

impl HeldOpenWorkerThread {
    /// Lets the worker's thread reach the end of its loop.
    pub fn release(self) {
        drop(self.0);
    }
}

/// A worker whose thread stays on its request loop until the returned
/// [`HeldOpenWorkerThread`] drops, so a test controls when the shutdown join
/// returns.
pub fn spawn_worker_held_open(
    db: RecordingsHandle,
    ctx: egui::Context,
    pending_writes: PendingWrites,
) -> (HistoryWorker, HeldOpenWorkerThread) {
    let worker = HistoryWorker::spawn(db, ctx, pending_writes);
    let held_open = HeldOpenWorkerThread(Box::new(worker.second_request_sender()));
    (worker, held_open)
}

/// Block until the worker delivers exactly one response, or time out.
pub fn next_response(worker: &HistoryWorker) -> Response {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut batch = worker.poll();
        if !batch.is_empty() {
            return batch.remove(0);
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a history worker response"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn next_visibility_response(harness: &mut Harness<'_, App>) -> Response {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut visibility = None;
        for response in harness.state().history.poll() {
            if matches!(response, Response::RecordingUiStateLoaded(_)) {
                visibility = Some(response);
            } else {
                harness.state_mut().handle_history_response(response);
            }
        }
        if let Some(response) = visibility {
            return response;
        }
        assert!(
            Instant::now() < deadline,
            "timed out reading track visibility"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The recordings the worker's database holds, as the History window lists
/// them.
pub fn listed_recordings(worker: &HistoryWorker) -> Vec<RecordingEntry> {
    let _ = worker.list();
    let Response::Listed {
        entries: Ok(entries),
        ..
    } = next_response(worker)
    else {
        panic!("expected a Listed response");
    };
    entries.into_entries()
}

/// The one recording the worker's database holds, as the History window lists
/// it.
pub fn only_recording(worker: &HistoryWorker) -> RecordingEntry {
    let mut entries = listed_recordings(worker);
    assert_eq!(entries.len(), 1, "expected exactly one recording");
    entries.remove(0)
}

/// The nav points [`sample_bytes`] holds.
pub const SAMPLE_POINT_COUNT: u64 = 20;
