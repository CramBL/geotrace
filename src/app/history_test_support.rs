//! Fixtures for the tests that drive a [`HistoryWorker`] over a real database.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::DateTime;
use gt_pending_writes::PendingWrites;
use gt_store::{
    HistoryDatabase as _, RecordingEntry, Recordings, RecordingsHandle, StoredFixPlacementRule,
    StoredSegmentation, StoredTrackSplitRule, TrackRange, TrackState,
};
use gt_test_utils::SyntheticGtdSpec;

use crate::app::history_db::{HistoryWorker, Response};

/// The nav points [`sample_bytes`] holds.
pub const SAMPLE_POINT_COUNT: u64 = 20;

/// `point_count` nav points one second apart from `start_secs`.
pub fn bytes_starting_at(start_secs: i64, point_count: usize) -> Vec<u8> {
    gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
        start: DateTime::from_timestamp(start_secs, 0).expect("valid timestamp"),
        point_count,
        step_secs: 1,
        start_lat_deg: 51.5,
        start_lon_deg: -0.1,
        lat_step_deg: 0.0002,
        lon_step_deg: -0.00015,
        heading_deg: 270.0,
        speed_kmh: 22.0,
        eph_m: 2.4,
        sats_seen: 10,
        sats_in_fix: 7,
    })
}

/// Twenty nav points one second apart from 2025-05-23 12:53:20 UTC.
pub fn sample_bytes() -> Vec<u8> {
    bytes_starting_at(1_748_000_000, SAMPLE_POINT_COUNT as usize)
}

pub fn segmentation() -> StoredSegmentation {
    StoredSegmentation {
        track_split_gap_us: 300_000_000,
        track_split_rule: StoredTrackSplitRule::StepInEitherDirection,
        fix_placement_rule: StoredFixPlacementRule::MissingHeadingAndNothingInFix,
        detect_clock_discontinuities: true,
        clock_discontinuity_sigmas: 5.0,
    }
}

pub fn store_recording(path: &Path, bytes: &[u8], tracks: &[TrackRange]) {
    let mut db = Recordings::open_or_create(path).expect("open");
    let meta = gt_store::extract_meta(bytes).expect("meta");
    db.insert("dev", &meta, tracks, segmentation(), bytes)
        .expect("insert");
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

pub fn worker_on(path: &Path) -> HistoryWorker {
    let db = Recordings::open_or_create(path).expect("reopen");
    HistoryWorker::spawn(
        RecordingsHandle::Owner(db),
        egui::Context::default(),
        PendingWrites::default(),
    )
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

/// The recordings the worker's database holds, as the History window lists
/// them.
pub fn listed_recordings(worker: &HistoryWorker) -> Vec<RecordingEntry> {
    worker.list();
    let Response::Listed(Ok(entries)) = next_response(worker) else {
        panic!("expected a Listed response");
    };
    entries
}

/// The one recording the worker's database holds, as the History window lists
/// it.
pub fn only_recording(worker: &HistoryWorker) -> RecordingEntry {
    let mut entries = listed_recordings(worker);
    assert_eq!(entries.len(), 1, "expected exactly one recording");
    entries.remove(0)
}
