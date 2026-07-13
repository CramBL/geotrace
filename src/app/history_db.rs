//! Background worker for all history-database reads and edits.
//!
//! Every history operation - listing, loading a recording, hiding tracks,
//! deleting recordings, prune previews, auto-prune - runs on a dedicated thread
//! that owns the [`Database`]. The UI thread sends [`Request`]s and drains
//! [`Response`]s once per frame (see [`HistoryWorker::poll`]), so a slow disk
//! or a large recording never stalls a render. Inserts still happen on the load
//! threads, which open the database by path. The global database lock keeps the
//! two paths safe.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use egui::Context;
use gt_history::{
    Database, DatabaseRef, DbError, HistoryDatabase, PruneMode, RecordingEntry, StoredRecording,
    TrackRange,
};
use gt_track_builder::SegmentationConfig;

use crate::app::auto_prune::{self, AutoPruneOutcome};
use crate::app::loader::stored_segmentation_from_config;

/// Why recordings are being deleted - selects the completion toast.
#[derive(Clone, Copy)]
pub enum DeleteReason {
    /// A recording deleted from the History list.
    Manual,
    /// The manual prune dialog.
    Prune,
    /// Auto-prune (after the user confirmed, or when confirmation is off).
    AutoPrune,
}

/// A completed mutation, carried back so the UI can show the right toast.
pub enum DbOp {
    TracksHidden {
        count: usize,
    },
    TracksDeleted {
        count: usize,
    },
    RecordingsDeleted {
        count: usize,
        reason: DeleteReason,
    },
    /// An identity was renamed. Carries both names so loaded recordings filed
    /// under `old` can be re-pointed to `new`.
    IdentityRenamed {
        old: String,
        new: String,
    },
}

/// A unit of work for the worker thread.
enum Request {
    List,
    Open(DatabaseRef),
    SetTracksHidden {
        db_ref: DatabaseRef,
        indices: Vec<usize>,
        hidden: bool,
    },
    /// Permanently remove specific tracks' points from one recording (re-encode).
    DeleteTracks {
        db_ref: DatabaseRef,
        indices: Vec<usize>,
    },
    /// Permanently remove every hidden track across all recordings (re-encode).
    DeleteHiddenTracks,
    DeleteRecordings {
        refs: Vec<DatabaseRef>,
        reason: DeleteReason,
    },
    RenameIdentity {
        old: String,
        new: String,
    },
    PrunePreview(PruneMode),
    AutoPrune {
        max_bytes: u64,
        confirm: bool,
    },
    /// Store a recording's serialized snap runs (opaque to the database).
    StoreSnapRuns {
        db_ref: DatabaseRef,
        blob: Vec<u8>,
    },
    /// Fetch a recording's stored snap runs, if any.
    LoadSnapRuns(DatabaseRef),
}

/// A result delivered back to the UI thread, drained via [`HistoryWorker::poll`].
pub enum Response {
    Listed(Result<Vec<RecordingEntry>, DbError>),
    Opened {
        db_ref: DatabaseRef,
        result: Result<StoredRecording, DbError>,
    },
    Mutated {
        op: DbOp,
        result: Result<(), DbError>,
    },
    PrunePreview(Result<Vec<DatabaseRef>, DbError>),
    AutoPruned(Result<AutoPruneOutcome, DbError>),
    /// Outcome of a snap-run store. Failures cost only the cache entry
    /// (the session stores keep working), so the app logs rather than
    /// toasts.
    SnapRunsStored(Result<(), DbError>),
    /// A recording's stored snap runs, `None` when it carries none.
    SnapRunsLoaded {
        db_ref: DatabaseRef,
        blob: Result<Option<Vec<u8>>, DbError>,
    },
}

/// Owns the history-database worker thread and the channels to talk to it.
pub struct HistoryWorker {
    req_tx: Option<Sender<Request>>,
    resp_rx: Receiver<Response>,
    handle: Option<JoinHandle<()>>,
    path: Option<PathBuf>,
}

impl HistoryWorker {
    /// A worker with no backing database (history unavailable, or test builds).
    /// All requests are dropped and [`HistoryWorker::poll`] never yields anything.
    pub fn disabled() -> Self {
        // The sender is dropped immediately, so the receiver is always empty.
        let (_, resp_rx) = mpsc::channel::<Response>();
        Self {
            req_tx: None,
            resp_rx,
            handle: None,
            path: None,
        }
    }

    /// Spawn the worker thread, moving `db` onto it.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    pub fn spawn(db: Database, ctx: Context) -> Self {
        let path = Some(db.path().to_owned());
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (resp_tx, resp_rx) = mpsc::channel::<Response>();
        let handle = std::thread::Builder::new()
            .name("history-db".to_owned())
            .spawn(move || worker_loop(db, &req_rx, &resp_tx, &ctx))
            .expect("failed to spawn history-db worker thread");
        Self {
            req_tx: Some(req_tx),
            resp_rx,
            handle: Some(handle),
            path,
        }
    }

    /// Whether a backing database is available (the worker is running).
    pub fn available(&self) -> bool {
        self.req_tx.is_some()
    }

    /// Path of the database file, for display.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn send(&self, req: Request) {
        if let Some(tx) = &self.req_tx {
            // A send error only happens once the worker has gone during teardown.
            tx.send(req).ok();
        }
    }

    pub fn list(&self) {
        self.send(Request::List);
    }

    pub fn open(&self, db_ref: DatabaseRef) {
        self.send(Request::Open(db_ref));
    }

    pub fn set_tracks_hidden(&self, db_ref: DatabaseRef, indices: Vec<usize>, hidden: bool) {
        self.send(Request::SetTracksHidden {
            db_ref,
            indices,
            hidden,
        });
    }

    pub fn delete_tracks(&self, db_ref: DatabaseRef, indices: Vec<usize>) {
        self.send(Request::DeleteTracks { db_ref, indices });
    }

    pub fn delete_hidden_tracks(&self) {
        self.send(Request::DeleteHiddenTracks);
    }

    pub fn delete_recordings(&self, refs: Vec<DatabaseRef>, reason: DeleteReason) {
        self.send(Request::DeleteRecordings { refs, reason });
    }

    pub fn rename_identity(&self, old: String, new: String) {
        self.send(Request::RenameIdentity { old, new });
    }

    pub fn prune_preview(&self, mode: PruneMode) {
        self.send(Request::PrunePreview(mode));
    }

    pub fn store_snap_runs(&self, db_ref: DatabaseRef, blob: Vec<u8>) {
        self.send(Request::StoreSnapRuns { db_ref, blob });
    }

    pub fn load_snap_runs(&self, db_ref: DatabaseRef) {
        self.send(Request::LoadSnapRuns(db_ref));
    }

    pub fn auto_prune(&self, max_bytes: u64, confirm: bool) {
        self.send(Request::AutoPrune { max_bytes, confirm });
    }

    /// Drain every response that has arrived since the last call.
    pub fn poll(&self) -> Vec<Response> {
        let mut out = Vec::new();
        while let Ok(resp) = self.resp_rx.try_recv() {
            out.push(resp);
        }
        out
    }
}

impl Drop for HistoryWorker {
    fn drop(&mut self) {
        // Dropping the request sender disconnects the worker's `recv`, ending its
        // loop. Then join so the thread is gone before we return.
        self.req_tx = None;
        if let Some(handle) = self.handle.take() {
            handle.join().ok();
        }
    }
}

fn worker_loop(
    mut db: Database,
    req_rx: &Receiver<Request>,
    resp_tx: &Sender<Response>,
    ctx: &Context,
) {
    while let Ok(req) = req_rx.recv() {
        let resp = handle_request(&mut db, req);
        // If the UI is gone the send fails, there is nothing left to repaint.
        if resp_tx.send(resp).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn handle_request(db: &mut Database, req: Request) -> Response {
    match req {
        Request::List => Response::Listed(db.list_recordings()),
        Request::Open(db_ref) => {
            let result = db.load(&db_ref);
            Response::Opened { db_ref, result }
        }
        Request::SetTracksHidden {
            db_ref,
            indices,
            hidden,
        } => {
            let count = indices.len();
            let result = db.set_tracks_hidden(&db_ref, &indices, hidden);
            Response::Mutated {
                op: DbOp::TracksHidden { count },
                result,
            }
        }
        Request::DeleteTracks { db_ref, indices } => {
            let count = indices.len();
            let result = purge_tracks(db, &db_ref, &indices);
            Response::Mutated {
                op: DbOp::TracksDeleted { count },
                result,
            }
        }
        Request::DeleteHiddenTracks => match purge_all_hidden(db) {
            Ok(count) => Response::Mutated {
                op: DbOp::TracksDeleted { count },
                result: Ok(()),
            },
            Err(e) => Response::Mutated {
                op: DbOp::TracksDeleted { count: 0 },
                result: Err(e),
            },
        },
        Request::DeleteRecordings { refs, reason } => {
            let count = refs.len();
            let result = db.delete_batch(&refs);
            Response::Mutated {
                op: DbOp::RecordingsDeleted { count, reason },
                result,
            }
        }
        Request::RenameIdentity { old, new } => {
            let result = db.rename_identity(&old, &new);
            Response::Mutated {
                op: DbOp::IdentityRenamed { old, new },
                result,
            }
        }
        Request::PrunePreview(mode) => Response::PrunePreview(db.prune_candidates(&mode)),
        Request::StoreSnapRuns { db_ref, blob } => {
            Response::SnapRunsStored(db.set_snap_blob(&db_ref, &blob))
        }
        Request::LoadSnapRuns(db_ref) => {
            let blob = db.snap_blob(&db_ref);
            Response::SnapRunsLoaded { db_ref, blob }
        }
        Request::AutoPrune { max_bytes, confirm } => {
            Response::AutoPruned(auto_prune::run(db, max_bytes, confirm))
        }
    }
}

/// Permanently remove every hidden track across all recordings, re-encoding each
/// affected recording. Returns the number of tracks removed.
fn purge_all_hidden(db: &mut Database) -> Result<usize, DbError> {
    let entries = db.list_recordings()?;
    let mut deleted = 0;
    for entry in entries {
        if entry.hidden_tracks == 0 {
            continue;
        }
        let stored = db.load(&entry.db_ref)?;
        let hidden: Vec<usize> = stored
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.hidden)
            .map(|(i, _)| i)
            .collect();
        deleted += hidden.len();
        purge_tracks_with_stored(db, &entry.db_ref, &stored, &hidden)?;
    }
    Ok(deleted)
}

/// Permanently remove the tracks at `drop_indices` from one recording.
fn purge_tracks(
    db: &mut Database,
    db_ref: &DatabaseRef,
    drop_indices: &[usize],
) -> Result<(), DbError> {
    let stored = db.load(db_ref)?;
    purge_tracks_with_stored(db, db_ref, &stored, drop_indices)
}

/// Re-encode `db_ref` with the points of the tracks at `drop_indices` removed,
/// then replace the stored recording. Surviving tracks keep their hidden flag and
/// are range-shifted onto the compacted point sequence. When nothing survives the
/// whole recording is deleted instead (an empty re-encode would fail).
///
/// The delete-and-reinsert intentionally drops any stored snap runs with the
/// old recording group (pinned by gt-history's
/// `snap_blob_is_dropped_with_its_recording`): the purge shifts point
/// indices, so the stored runs could no longer be matched to their points.
fn purge_tracks_with_stored(
    db: &mut Database,
    db_ref: &DatabaseRef,
    stored: &StoredRecording,
    drop_indices: &[usize],
) -> Result<(), DbError> {
    let drop: HashSet<usize> = drop_indices.iter().copied().collect();

    let kept: Vec<TrackRange> = stored
        .tracks
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, t)| *t)
        .collect();
    if kept.is_empty() {
        return db.delete_batch(std::slice::from_ref(db_ref));
    }

    let drop_ranges: Vec<Range<usize>> = stored
        .tracks
        .iter()
        .enumerate()
        .filter(|(i, _)| drop.contains(i))
        .map(|(_, t)| {
            let start = usize::try_from(t.start).unwrap_or(usize::MAX);
            let end = usize::try_from(t.end).unwrap_or(usize::MAX);
            start..end
        })
        .collect();
    if drop_ranges.is_empty() {
        return Ok(());
    }

    let new_bytes = gt_loader::reencode_dropping_ranges(&stored.bytes, &drop_ranges)
        .map_err(|e| DbError::Backend(e.to_string()))?;

    // Range-shift the survivors: each keeps its length, starting where the
    // previous survivor ended.
    let mut cursor = 0;
    let mut new_tracks = Vec::with_capacity(kept.len());
    for t in &kept {
        let len = t.end.saturating_sub(t.start);
        new_tracks.push(TrackRange {
            start: cursor,
            end: cursor + len,
            hidden: t.hidden,
        });
        cursor += len;
    }

    let settings = stored
        .segmentation
        .unwrap_or_else(|| stored_segmentation_from_config(&SegmentationConfig::default()));
    let meta = gt_history::extract_meta(&new_bytes)?;

    db.delete_batch(std::slice::from_ref(db_ref))?;
    db.insert(&db_ref.identity, &meta, &new_tracks, settings, &new_bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use chrono::DateTime;
    use gt_history::{StoredSegmentation, TrackRange};
    use gt_test_utils::{SyntheticGtdSpec, synthetic_gtd_bytes};

    // Brings in `HistoryWorker`, `Request`/`Response`, `Database`, `Context`,
    // `PruneMode`, `AutoPruneOutcome`, etc. without re-importing sibling modules.
    use super::*;

    fn sample_bytes() -> Vec<u8> {
        synthetic_gtd_bytes(SyntheticGtdSpec {
            start: DateTime::from_timestamp(1_748_000_000, 0).expect("valid timestamp"),
            point_count: 20,
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

    /// Block until the worker delivers exactly one response, or time out.
    fn next_response(worker: &HistoryWorker) -> Response {
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

    #[test]
    fn worker_round_trips_every_operation() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");

        // Seed one recording (two tracks) through a plain handle.
        let bytes = sample_bytes();
        {
            let mut db = Database::open_or_create(&path).expect("open");
            let meta = gt_history::extract_meta(&bytes).expect("meta");
            let tracks = [
                TrackRange {
                    start: 0,
                    end: 10,
                    hidden: false,
                },
                TrackRange {
                    start: 10,
                    end: 20,
                    hidden: false,
                },
            ];
            let settings = StoredSegmentation {
                track_split_gap_us: 300_000_000,
                detect_clock_discontinuities: true,
                clock_discontinuity_sigmas: 5.0,
            };
            db.insert("dev", &meta, &tracks, settings, &bytes)
                .expect("insert");
        }

        let db = Database::open_or_create(&path).expect("reopen");
        let worker = HistoryWorker::spawn(db, Context::default());
        assert!(worker.available());
        assert_eq!(worker.path(), Some(path.as_path()));

        // List
        worker.list();
        let Response::Listed(Ok(entries)) = next_response(&worker) else {
            panic!("expected a Listed response");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].total_tracks, 2);
        let db_ref = entries[0].db_ref.clone();

        // Open returns the stored recording (bytes + tracks).
        worker.open(db_ref.clone());
        let Response::Opened { result, .. } = next_response(&worker) else {
            panic!("expected an Opened response");
        };
        let stored = result.expect("open ok");
        assert!(!stored.bytes.is_empty());
        assert_eq!(stored.tracks.len(), 2);

        // Hide one track.
        worker.set_tracks_hidden(db_ref.clone(), vec![0], true);
        let Response::Mutated {
            op: DbOp::TracksHidden { count },
            result,
        } = next_response(&worker)
        else {
            panic!("expected a TracksHidden mutation");
        };
        assert_eq!(count, 1);
        result.expect("hide ok");

        // Prune preview reports candidates without deleting.
        worker.prune_preview(PruneMode::ByCount { keep: 0 });
        let Response::PrunePreview(Ok(candidates)) = next_response(&worker) else {
            panic!("expected a PrunePreview response");
        };
        assert_eq!(candidates.len(), 1);

        // Auto-prune with an enormous budget leaves everything in place.
        worker.auto_prune(u64::MAX, false);
        let Response::AutoPruned(Ok(AutoPruneOutcome::NotNeeded)) = next_response(&worker) else {
            panic!("expected AutoPruned(NotNeeded)");
        };

        // Delete the recording.
        worker.delete_recordings(vec![db_ref], DeleteReason::Manual);
        let Response::Mutated {
            op: DbOp::RecordingsDeleted { count, .. },
            result,
        } = next_response(&worker)
        else {
            panic!("expected a RecordingsDeleted mutation");
        };
        assert_eq!(count, 1);
        result.expect("delete ok");

        worker.list();
        let Response::Listed(Ok(entries)) = next_response(&worker) else {
            panic!("expected a Listed response");
        };
        assert!(entries.is_empty(), "recording should be gone after delete");
    }

    #[test]
    fn worker_permanently_deletes_hidden_tracks() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");

        // 20 points, manually split into two ten-point tracks.
        let bytes = sample_bytes();
        let settings = StoredSegmentation {
            track_split_gap_us: 300_000_000,
            detect_clock_discontinuities: false,
            clock_discontinuity_sigmas: 5.0,
        };
        {
            let mut db = Database::open_or_create(&path).expect("open");
            let meta = gt_history::extract_meta(&bytes).expect("meta");
            let tracks = [
                TrackRange {
                    start: 0,
                    end: 10,
                    hidden: false,
                },
                TrackRange {
                    start: 10,
                    end: 20,
                    hidden: false,
                },
            ];
            db.insert("dev", &meta, &tracks, settings, &bytes)
                .expect("insert");
        }

        let db = Database::open_or_create(&path).expect("reopen");
        let worker = HistoryWorker::spawn(db, Context::default());

        // Hide the first track, then permanently delete all hidden tracks.
        let db_ref = {
            worker.list();
            let Response::Listed(Ok(entries)) = next_response(&worker) else {
                panic!("expected Listed");
            };
            entries[0].db_ref.clone()
        };
        worker.set_tracks_hidden(db_ref, vec![0], true);
        next_response(&worker);

        worker.delete_hidden_tracks();
        let Response::Mutated {
            op: DbOp::TracksDeleted { count },
            result,
        } = next_response(&worker)
        else {
            panic!("expected TracksDeleted");
        };
        assert_eq!(count, 1, "one hidden track removed");
        result.expect("delete ok");

        // The recording now has a single ten-point track and no hidden tracks.
        worker.list();
        let Response::Listed(Ok(entries)) = next_response(&worker) else {
            panic!("expected Listed");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].total_tracks, 1);
        assert_eq!(entries[0].hidden_tracks, 0);
        assert_eq!(entries[0].meta.nav_point_count, 10);

        let new_ref = entries[0].db_ref.clone();
        worker.open(new_ref);
        let Response::Opened { result, .. } = next_response(&worker) else {
            panic!("expected Opened");
        };
        let stored = result.expect("open ok");
        assert_eq!(
            stored.tracks,
            vec![TrackRange {
                start: 0,
                end: 10,
                hidden: false,
            }]
        );
    }

    #[test]
    fn disabled_worker_is_inert() {
        let worker = HistoryWorker::disabled();
        assert!(!worker.available());
        assert!(worker.path().is_none());
        worker.list();
        assert!(worker.poll().is_empty());
    }
}
