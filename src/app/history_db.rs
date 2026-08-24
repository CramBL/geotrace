//! Background worker for all history-database reads and edits.
//!
//! Every history operation - listing, loading a recording, hiding tracks,
//! deleting recordings, prune previews, auto-prune - runs on a dedicated thread
//! that owns the [`Recordings`]. The UI thread sends [`Request`]s and drains
//! [`Response`]s once per frame (see [`HistoryWorker::poll`]), so a slow disk
//! or a large recording never stalls a render. Inserts still happen on the load
//! threads, which open the database by path. The global database lock keeps the
//! two paths safe.
//!
//! A request that writes runs under a [`PendingWrites`] guard, so the process
//! waits for it on the way out. A request the registry turns away is answered
//! with [`Response::WriteRefused`], naming why.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use egui::Context;
use gt_log_view::LogAttachmentRef;
use gt_pending_writes::{PendingWriteGuard, PendingWrites, WriteKind, WriteRefusal};
use gt_store::{
    AttachedLog, DatabaseRef, DbError, HistoryDatabase, LogAttachmentError, LogAttachmentId,
    LogAttachments as _, LogContentHash, LogToAttach, PruneMode, RecordingEntry, Recordings,
    StoredLogFilter, StoredRecording, TrackRange,
};
use gt_track_builder::SegmentationConfig;
use gt_ui_types::LoadedLogId;

use crate::app::auto_prune::{self, AutoPruneOutcome};
use crate::app::background_thread;
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
    /// Store a log with a recording, log bytes and all.
    AttachLog {
        db_ref: DatabaseRef,
        log: LoadedLogId,
        name: String,
        text: Arc<str>,
        filters: Vec<StoredLogFilter>,
    },
    /// Read back every log attached to a recording that just opened.
    LoadAttachedLogs(DatabaseRef),
    /// Rewrite one attachment's stored filter stack.
    SetAttachedLogFilters {
        attachment: LogAttachmentRef,
        filters: Vec<StoredLogFilter>,
    },
    /// Remove one attachment: its attribute, and the log stored with it.
    DetachLog {
        attachment: LogAttachmentRef,
        log: LoadedLogId,
        name: String,
    },
    /// Whether a recording already holds this exact log.
    FindDuplicateAttachment {
        db_ref: DatabaseRef,
        log: LoadedLogId,
        text: Arc<str>,
    },
}

impl Request {
    /// The label the write registry lists this request under, or [`None`] for
    /// a request that only reads.
    fn database_write_label(&self) -> Option<&'static str> {
        match self {
            Self::List
            | Self::Open(_)
            | Self::PrunePreview(_)
            | Self::LoadSnapRuns(_)
            | Self::LoadAttachedLogs(_)
            | Self::FindDuplicateAttachment { .. } => None,
            Self::SetTracksHidden { hidden: true, .. } => {
                Some("Hiding tracks in recording history")
            }
            Self::SetTracksHidden { hidden: false, .. } => {
                Some("Showing tracks in recording history")
            }
            Self::DeleteTracks { .. } => Some("Deleting tracks from recording history"),
            Self::DeleteHiddenTracks => Some("Deleting hidden tracks from recording history"),
            Self::DeleteRecordings { .. } => Some("Deleting recordings from recording history"),
            Self::RenameIdentity { .. } => Some("Renaming an identity in recording history"),
            Self::AutoPrune { .. } => Some("Auto-pruning recording history"),
            Self::StoreSnapRuns { .. } => Some("Storing snap runs in recording history"),
            Self::AttachLog { .. } => Some("Storing a log with a recording"),
            Self::SetAttachedLogFilters { .. } => Some("Storing an attached log's filters"),
            Self::DetachLog { .. } => Some("Removing an attached log from a recording"),
        }
    }
}

/// One of a recording's attachments, as the worker read it back. `name` comes
/// from the attribute, so a log that could not be read is still nameable.
pub struct RestoredLogAttachment {
    pub id: LogAttachmentId,
    pub name: String,
    pub log: Result<AttachedLog, LogAttachmentError>,
}

/// A log the worker stored with a recording, and the stack it stored with it.
pub struct StoredLogAttachment {
    pub attachment: LogAttachmentRef,
    pub filters: Vec<StoredLogFilter>,
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
    /// A recording's stored snap runs, `None` when it has no stored runs.
    SnapRunsLoaded {
        db_ref: DatabaseRef,
        blob: Result<Option<Vec<u8>>, DbError>,
    },
    /// Outcome of storing a log with a recording.
    LogAttached {
        log: LoadedLogId,
        name: String,
        result: Result<StoredLogAttachment, LogAttachmentError>,
    },
    /// The logs a recording carries, one entry per attachment it names.
    AttachedLogsLoaded {
        db_ref: DatabaseRef,
        attachments: Result<Vec<RestoredLogAttachment>, DbError>,
    },
    /// Outcome of a filter-stack write. A failure costs only the stored copy:
    /// the loaded log keeps the stack the user is looking at.
    AttachedLogFiltersStored(Result<(), LogAttachmentError>),
    /// Outcome of removing an attachment.
    LogDetached {
        log: LoadedLogId,
        name: String,
        result: Result<(), LogAttachmentError>,
    },
    /// What `recording` already holds the dialog's log as, if anything.
    DuplicateAttachmentFound {
        log: LoadedLogId,
        recording: DatabaseRef,
        existing: Result<Option<String>, DbError>,
    },
    /// The registry turned a write away, and the worker answered without
    /// touching the database.
    WriteRefused {
        label: &'static str,
        refusal: WriteRefusal,
    },
}

/// A second sender on a worker's request channel: while it lives the worker's
/// `recv` cannot fail, so its thread stays on its loop.
#[cfg(test)]
pub struct HeldOpenWorkerThread(Sender<Request>);

#[cfg(test)]
impl HeldOpenWorkerThread {
    /// Lets the worker's thread reach the end of its loop.
    pub fn release(self) {
        drop(self.0);
    }
}

/// Owns the history-database worker thread and the request and response
/// channels to it.
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
    pub fn spawn(db: Recordings, ctx: Context, pending_writes: PendingWrites) -> Self {
        let path = Some(db.path().to_owned());
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (resp_tx, resp_rx) = mpsc::channel::<Response>();
        let handle = background_thread::spawn_or_panic("history-db", move || {
            worker_loop(db, &req_rx, &resp_tx, &ctx, &pending_writes);
        });
        Self {
            req_tx: Some(req_tx),
            resp_rx,
            handle: Some(handle),
            path,
        }
    }

    /// A worker whose thread stays on its request loop until the returned
    /// [`HeldOpenWorkerThread`] drops, so a test controls when the shutdown
    /// join returns.
    #[cfg(test)]
    pub fn spawn_held_open(
        db: Recordings,
        ctx: Context,
        pending_writes: PendingWrites,
    ) -> (Self, HeldOpenWorkerThread) {
        let worker = Self::spawn(db, ctx, pending_writes);
        let held_open = worker
            .req_tx
            .clone()
            .expect("a spawned worker holds a request sender");
        (worker, HeldOpenWorkerThread(held_open))
    }

    /// Whether a backing database is available (the worker is running).
    pub fn available(&self) -> bool {
        self.req_tx.is_some()
    }

    /// Path of the database file, for display.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Hide the display path - for snapshot tests, whose temporary database
    /// path differs every run and would flake the image.
    #[cfg(test)]
    pub fn hide_path(&mut self) {
        self.path = None;
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

    pub fn attach_log(
        &self,
        db_ref: DatabaseRef,
        log: LoadedLogId,
        name: String,
        text: Arc<str>,
        filters: Vec<StoredLogFilter>,
    ) {
        self.send(Request::AttachLog {
            db_ref,
            log,
            name,
            text,
            filters,
        });
    }

    pub fn load_attached_logs(&self, db_ref: DatabaseRef) {
        self.send(Request::LoadAttachedLogs(db_ref));
    }

    pub fn set_attached_log_filters(
        &self,
        attachment: LogAttachmentRef,
        filters: Vec<StoredLogFilter>,
    ) {
        self.send(Request::SetAttachedLogFilters {
            attachment,
            filters,
        });
    }

    pub fn detach_log(&self, attachment: LogAttachmentRef, log: LoadedLogId, name: String) {
        self.send(Request::DetachLog {
            attachment,
            log,
            name,
        });
    }

    pub fn find_duplicate_attachment(&self, db_ref: DatabaseRef, log: LoadedLogId, text: Arc<str>) {
        self.send(Request::FindDuplicateAttachment { db_ref, log, text });
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

    /// End the worker and wait for the request it is on to finish. A worker
    /// from [`HistoryWorker::disabled`] has no thread and returns at once.
    pub fn shutdown(mut self) {
        self.end_and_join_worker_thread();
    }

    /// Ends the worker on a thread of its own, which holds `write` until the
    /// database is closed. The caller returns as soon as that thread is
    /// spawned. A read-only session has no write to hold: it closes a
    /// database it never wrote to.
    ///
    /// Where the thread cannot be spawned the worker is left detached: its
    /// `history-db` thread ends by itself once the request sender drops, and
    /// `write` is released because nothing waits for it.
    pub fn shutdown_on_a_thread_of_its_own(mut self, write: Option<PendingWriteGuard>) {
        let req_tx = self.req_tx.take();
        let handle = self.handle.take();
        let spawned = std::thread::Builder::new()
            .name("history-db-shutdown".to_owned())
            .spawn(move || {
                drop(req_tx);
                if let Some(handle) = handle {
                    handle.join().ok();
                }
                drop(write);
            });
        if let Err(error) = spawned {
            log::error!("Failed to spawn the history shutdown thread: {error:#}");
        }
    }

    /// Dropping the request sender disconnects the worker's `recv`, ending its
    /// loop. Then join so the thread is gone before we return.
    fn end_and_join_worker_thread(&mut self) {
        self.req_tx = None;
        if let Some(handle) = self.handle.take() {
            handle.join().ok();
        }
    }
}

impl Drop for HistoryWorker {
    fn drop(&mut self) {
        self.end_and_join_worker_thread();
    }
}

fn worker_loop(
    mut db: Recordings,
    req_rx: &Receiver<Request>,
    resp_tx: &Sender<Response>,
    ctx: &Context,
    pending_writes: &PendingWrites,
) {
    while let Ok(req) = req_rx.recv() {
        let resp = match req.database_write_label() {
            None => handle_request(&mut db, req),
            Some(label) => match pending_writes.try_begin(label, WriteKind::RecordingDatabase) {
                Ok(_write) => handle_request(&mut db, req),
                Err(refusal) => Response::WriteRefused { label, refusal },
            },
        };
        // If the UI is gone the send fails, there is nothing left to repaint.
        if resp_tx.send(resp).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn handle_request(db: &mut Recordings, req: Request) -> Response {
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
        Request::AttachLog {
            db_ref,
            log,
            name,
            text,
            filters,
        } => {
            let result = db
                .attach_log(
                    &db_ref,
                    &LogToAttach {
                        name: &name,
                        text: &text,
                        filters: filters.clone(),
                    },
                )
                .map(|id| StoredLogAttachment {
                    attachment: LogAttachmentRef {
                        recording: db_ref,
                        id,
                    },
                    filters,
                });
            Response::LogAttached { log, name, result }
        }
        Request::LoadAttachedLogs(db_ref) => {
            let attachments = db.log_attachments(&db_ref).map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| RestoredLogAttachment {
                        id: entry.id,
                        name: entry.attachment.name,
                        log: db.load_attached_log(&db_ref, entry.id),
                    })
                    .collect()
            });
            Response::AttachedLogsLoaded {
                db_ref,
                attachments,
            }
        }
        Request::SetAttachedLogFilters {
            attachment,
            filters,
        } => Response::AttachedLogFiltersStored(db.set_attached_log_filters(
            &attachment.recording,
            attachment.id,
            filters,
        )),
        Request::DetachLog {
            attachment,
            log,
            name,
        } => {
            let result = db.detach_log(&attachment.recording, attachment.id);
            Response::LogDetached { log, name, result }
        }
        Request::FindDuplicateAttachment { db_ref, log, text } => {
            let existing = db
                .log_attachment_with_content(&db_ref, LogContentHash::of_log_bytes(text.as_bytes()))
                .map(|entry| entry.map(|entry| entry.attachment.name));
            Response::DuplicateAttachmentFound {
                log,
                recording: db_ref,
                existing,
            }
        }
    }
}

/// Permanently remove every hidden track across all recordings, re-encoding each
/// affected recording. Returns the number of tracks removed.
fn purge_all_hidden(db: &mut Recordings) -> Result<usize, DbError> {
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
    db: &mut Recordings,
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
    db: &mut Recordings,
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
    let meta = gt_store::extract_meta(&new_bytes)?;

    db.delete_batch(std::slice::from_ref(db_ref))?;
    db.insert(&db_ref.identity, &meta, &new_tracks, settings, &new_bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use chrono::DateTime;
    use gt_pending_writes::WriteAccess;
    use gt_store::{StoredSegmentation, TrackRange};
    use gt_test_utils::{SyntheticGtdSpec, pending_writes, synthetic_gtd_bytes};
    use rstest::rstest;

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

    /// Store one recording of two ten-point tracks in a database at `path`.
    fn seed_two_track_recording(path: &Path) {
        let bytes = sample_bytes();
        let mut db = Recordings::open_or_create(path).expect("open");
        let meta = gt_store::extract_meta(&bytes).expect("meta");
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

    /// The reference to the one recording the worker's database holds.
    fn only_recording_ref(worker: &HistoryWorker) -> DatabaseRef {
        worker.list();
        let Response::Listed(Ok(entries)) = next_response(worker) else {
            panic!("expected a Listed response");
        };
        let [entry] = entries.as_slice() else {
            panic!("expected exactly one recording, got {}", entries.len());
        };
        entry.db_ref.clone()
    }

    #[test]
    fn worker_round_trips_every_operation() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");

        seed_two_track_recording(&path);

        let db = Recordings::open_or_create(&path).expect("reopen");
        let worker = HistoryWorker::spawn(db, Context::default(), PendingWrites::default());
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

        seed_two_track_recording(&path);

        let db = Recordings::open_or_create(&path).expect("reopen");
        let worker = HistoryWorker::spawn(db, Context::default(), PendingWrites::default());

        // Hide the first track, then permanently delete all hidden tracks.
        let db_ref = only_recording_ref(&worker);
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
        worker.shutdown();
    }

    #[test]
    fn a_mutation_registers_and_releases_its_write_guard() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let pending_writes = PendingWrites::default();
        let db = Recordings::open_or_create(&path).expect("reopen");
        let worker = HistoryWorker::spawn(db, Context::default(), pending_writes.clone());
        let db_ref = only_recording_ref(&worker);

        worker.set_tracks_hidden(db_ref, vec![0], true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };

        result.expect("hide ok");
        assert_eq!(
            pending_writes.snapshot().recently_finished,
            vec!["Hiding tracks in recording history"],
            "the listing before it registered nothing, the hide registered while it ran"
        );
        assert!(pending_writes.is_idle());
        worker.shutdown();
    }

    #[rstest]
    #[case::shutting_down(pending_writes::shutting_down_registry(), WriteRefusal::ShuttingDown)]
    #[case::read_only_session(
        PendingWrites::new(WriteAccess::ReadOnly),
        WriteRefusal::ReadOnlySession
    )]
    fn a_refused_mutation_is_answered_with_its_reason_and_leaves_the_database_alone(
        #[case] pending_writes: PendingWrites,
        #[case] expected: WriteRefusal,
    ) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let db = Recordings::open_or_create(&path).expect("reopen");
        let worker = HistoryWorker::spawn(db, Context::default(), pending_writes.clone());
        let db_ref = only_recording_ref(&worker);

        worker.set_tracks_hidden(db_ref, vec![0], true);

        let Response::WriteRefused { label, refusal } = next_response(&worker) else {
            panic!("expected the write to be refused");
        };
        assert_eq!(refusal, expected);
        assert_eq!(label, "Hiding tracks in recording history");
        assert!(pending_writes.is_idle());

        // Reads still answer, and report a database the refused write left alone.
        worker.list();
        let Response::Listed(Ok(entries)) = next_response(&worker) else {
            panic!("expected a Listed response");
        };
        assert_eq!(entries.first().map(|entry| entry.hidden_tracks), Some(0));
        worker.shutdown();
    }

    fn db_ref() -> DatabaseRef {
        DatabaseRef {
            identity: "dev".to_owned(),
            group_name: "2025-05-23T12-53-20".to_owned(),
        }
    }

    fn attachment_ref() -> LogAttachmentRef {
        LogAttachmentRef {
            recording: db_ref(),
            id: LogAttachmentId::new_random(),
        }
    }

    #[rstest]
    #[case(Request::List, None)]
    #[case(Request::Open(db_ref()), None)]
    #[case(Request::PrunePreview(PruneMode::ByCount { keep: 0 }), None)]
    #[case(Request::LoadSnapRuns(db_ref()), None)]
    #[case(Request::LoadAttachedLogs(db_ref()), None)]
    #[case(
        Request::FindDuplicateAttachment {
            db_ref: db_ref(),
            log: LoadedLogId::new(1),
            text: "boot".into(),
        },
        None
    )]
    #[case(
        Request::SetTracksHidden {
            db_ref: db_ref(),
            indices: vec![0],
            hidden: true,
        },
        Some("Hiding tracks in recording history")
    )]
    #[case(
        Request::SetTracksHidden {
            db_ref: db_ref(),
            indices: vec![0],
            hidden: false,
        },
        Some("Showing tracks in recording history")
    )]
    #[case(
        Request::DeleteTracks {
            db_ref: db_ref(),
            indices: vec![0],
        },
        Some("Deleting tracks from recording history")
    )]
    #[case(
        Request::DeleteHiddenTracks,
        Some("Deleting hidden tracks from recording history")
    )]
    #[case(
        Request::DeleteRecordings {
            refs: vec![db_ref()],
            reason: DeleteReason::Manual,
        },
        Some("Deleting recordings from recording history")
    )]
    #[case(
        Request::RenameIdentity {
            old: "dev".to_owned(),
            new: "rover".to_owned(),
        },
        Some("Renaming an identity in recording history")
    )]
    #[case(
        Request::AutoPrune {
            max_bytes: 0,
            confirm: false,
        },
        Some("Auto-pruning recording history")
    )]
    #[case(
        Request::StoreSnapRuns {
            db_ref: db_ref(),
            blob: Vec::new(),
        },
        Some("Storing snap runs in recording history")
    )]
    #[case(
        Request::AttachLog {
            db_ref: db_ref(),
            log: LoadedLogId::new(1),
            name: "navsyncd.log".to_owned(),
            text: "boot".into(),
            filters: Vec::new(),
        },
        Some("Storing a log with a recording")
    )]
    #[case(
        Request::SetAttachedLogFilters {
            attachment: attachment_ref(),
            filters: Vec::new(),
        },
        Some("Storing an attached log's filters")
    )]
    #[case(
        Request::DetachLog {
            attachment: attachment_ref(),
            log: LoadedLogId::new(1),
            name: "navsyncd.log".to_owned(),
        },
        Some("Removing an attached log from a recording")
    )]
    fn only_a_request_that_writes_carries_a_write_label(
        #[case] request: Request,
        #[case] label: Option<&str>,
    ) {
        assert_eq!(request.database_write_label(), label);
    }
}
