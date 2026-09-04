//! Background worker for all history-database reads and edits.
//!
//! Every history operation - listing, loading a recording, shelving tracks,
//! deleting recordings, prune previews, auto-prune - runs on a dedicated thread
//! that owns the [`RecordingsHandle`]. The UI thread sends [`Request`]s and drains
//! [`Response`]s once per frame (see [`HistoryWorker::poll`]), so a slow disk
//! or a large recording never stalls a render. Inserts still happen on the load
//! threads, which open the database by path. The global database lock keeps the
//! two paths safe.
//!
//! A [`WriteRequest`] runs on [`RecordingsHandle::writer`] and under a
//! [`PendingWrites`] guard, so the process waits for it on the way out. A
//! read-only session has no writer, and a rejected write comes back as
//! [`Response::WriteRejected`] holding the [`WriteRejection`].

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use egui::Context;
use gt_log_view::LogAttachmentRef;
use gt_pending_writes::{PendingWriteGuard, PendingWrites, WriteKind, WriteRejection};
use gt_store::{
    AttachedLog, DatabaseRef, DbError, HistoryDatabase, LogAttachmentEntry, LogAttachmentError,
    LogAttachmentId, LogAttachments as _, LogContentHash, LogToAttach, PruneMode,
    ReadOnlyHistoryDatabase, ReadOnlyLogAttachments as _, ReadOnlyRecordings, RecordingEntry,
    Recordings, RecordingsHandle, StoredLogFilter, StoredRecording, TrackRange, TrackState,
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
    TracksShelved {
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
    Read(ReadRequest),
    Write(WriteRequest),
}

/// A request served from the read methods, which [`RecordingsHandle::read`]
/// gives for either variant.
enum ReadRequest {
    List,
    Open(DatabaseRef),
    PrunePreview(PruneMode),
    /// Fetch a recording's stored snap runs, if any.
    LoadSnapRuns(DatabaseRef),
    /// Read back every log attached to a recording that just opened.
    LoadAttachedLogs(DatabaseRef),
    /// Read back the one log `attachment` names, which the log viewer requests
    /// when the user loads it from the list.
    LoadAttachedLog {
        attachment: LogAttachmentRef,
        name: String,
    },
    /// Whether a recording already holds this exact log.
    FindDuplicateAttachment {
        db_ref: DatabaseRef,
        log: LoadedLogId,
        text: Arc<str>,
    },
}

/// A request that changes the database, which only [`RecordingsHandle::writer`]
/// can run.
enum WriteRequest {
    SetTracksShelved {
        db_ref: DatabaseRef,
        rows: Vec<usize>,
        shelved: bool,
    },
    /// Permanently remove the nav points of the tracks in these stored table
    /// rows from one recording (re-encode).
    DeleteTracks {
        db_ref: DatabaseRef,
        rows: Vec<usize>,
    },
    /// Permanently remove every shelved track across all recordings (re-encode).
    DeleteShelvedTracks,
    DeleteRecordings {
        refs: Vec<DatabaseRef>,
        reason: DeleteReason,
    },
    RenameIdentity {
        old: String,
        new: String,
    },
    AutoPrune {
        max_bytes: u64,
        confirm: bool,
    },
    /// Store a recording's serialized snap runs (opaque to the database).
    StoreSnapRuns {
        db_ref: DatabaseRef,
        blob: Vec<u8>,
    },
    /// Store a log with a recording, log bytes and all.
    AttachLog {
        db_ref: DatabaseRef,
        log: LoadedLogId,
        name: String,
        text: Arc<str>,
        filters: Vec<StoredLogFilter>,
    },
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
}

impl WriteRequest {
    /// The label the write registry lists this request under.
    fn database_write_label(&self) -> &'static str {
        match self {
            Self::SetTracksShelved { shelved: true, .. } => "Shelving tracks in recording history",
            Self::SetTracksShelved { shelved: false, .. } => {
                "Unshelving tracks in recording history"
            }
            Self::DeleteTracks { .. } => "Deleting tracks from recording history",
            Self::DeleteShelvedTracks => "Deleting shelved tracks from recording history",
            Self::DeleteRecordings { .. } => "Deleting recordings from recording history",
            Self::RenameIdentity { .. } => "Renaming an identity in recording history",
            Self::AutoPrune { .. } => "Auto-pruning recording history",
            Self::StoreSnapRuns { .. } => "Storing snap runs in recording history",
            Self::AttachLog { .. } => "Storing a log with a recording",
            Self::SetAttachedLogFilters { .. } => "Storing an attached log's filters",
            Self::DetachLog { .. } => "Removing an attached log from a recording",
        }
    }
}

/// One of a recording's attachments, as the worker read it back. The entry
/// comes from the attribute, so a log that could not be read is still listed
/// and named.
pub struct RestoredLogAttachment {
    pub entry: LogAttachmentEntry,
    pub log: Result<AttachedLog, LogAttachmentError>,
}

/// A log the worker stored with a recording, and the attachment it wrote for
/// it.
pub struct StoredLogAttachment {
    pub recording: DatabaseRef,
    pub entry: LogAttachmentEntry,
}

/// The attachment a recording already holds a log as, found by content hash.
pub struct ExistingLogAttachment {
    pub id: LogAttachmentId,
    pub name: String,
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
    /// (the session stores keep working), so the app logs them.
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
    /// The logs a recording carries, one entry per attachment it holds.
    AttachedLogsLoaded {
        db_ref: DatabaseRef,
        attachments: Result<Vec<RestoredLogAttachment>, DbError>,
    },
    /// The one log the viewer requested out of a recording.
    AttachedLogLoaded {
        attachment: LogAttachmentRef,
        name: String,
        log: Result<AttachedLog, LogAttachmentError>,
    },
    /// Outcome of a filter-stack write. A failure costs only the stored copy:
    /// the loaded log keeps the stack the user is looking at.
    AttachedLogFiltersStored(Result<(), LogAttachmentError>),
    /// Outcome of removing an attachment.
    LogDetached {
        attachment: LogAttachmentRef,
        log: LoadedLogId,
        name: String,
        result: Result<(), LogAttachmentError>,
    },
    /// What `recording` already holds the dialog's log as, if anything.
    DuplicateAttachmentFound {
        log: LoadedLogId,
        recording: DatabaseRef,
        existing: Result<Option<ExistingLogAttachment>, DbError>,
    },
    /// The registry rejected the write, and the worker returned without
    /// touching the database.
    WriteRejected {
        label: &'static str,
        rejection: WriteRejection,
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
    pub fn spawn(db: RecordingsHandle, ctx: Context, pending_writes: PendingWrites) -> Self {
        let path = Some(db.read().path().to_owned());
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
        db: RecordingsHandle,
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

    fn send_read(&self, req: ReadRequest) {
        self.send(Request::Read(req));
    }

    fn send_write(&self, req: WriteRequest) {
        self.send(Request::Write(req));
    }

    pub fn list(&self) {
        self.send_read(ReadRequest::List);
    }

    pub fn open(&self, db_ref: DatabaseRef) {
        self.send_read(ReadRequest::Open(db_ref));
    }

    /// Shelve the tracks in the stored table rows `rows` of a recording, or
    /// unshelve them when `shelved` is false.
    pub fn set_tracks_shelved(&self, db_ref: DatabaseRef, rows: Vec<usize>, shelved: bool) {
        self.send_write(WriteRequest::SetTracksShelved {
            db_ref,
            rows,
            shelved,
        });
    }

    /// Permanently delete the tracks in the stored table rows `rows` of a
    /// recording.
    pub fn delete_tracks(&self, db_ref: DatabaseRef, rows: Vec<usize>) {
        self.send_write(WriteRequest::DeleteTracks { db_ref, rows });
    }

    pub fn delete_shelved_tracks(&self) {
        self.send_write(WriteRequest::DeleteShelvedTracks);
    }

    pub fn delete_recordings(&self, refs: Vec<DatabaseRef>, reason: DeleteReason) {
        self.send_write(WriteRequest::DeleteRecordings { refs, reason });
    }

    pub fn rename_identity(&self, old: String, new: String) {
        self.send_write(WriteRequest::RenameIdentity { old, new });
    }

    pub fn prune_preview(&self, mode: PruneMode) {
        self.send_read(ReadRequest::PrunePreview(mode));
    }

    pub fn store_snap_runs(&self, db_ref: DatabaseRef, blob: Vec<u8>) {
        self.send_write(WriteRequest::StoreSnapRuns { db_ref, blob });
    }

    pub fn load_snap_runs(&self, db_ref: DatabaseRef) {
        self.send_read(ReadRequest::LoadSnapRuns(db_ref));
    }

    pub fn attach_log(
        &self,
        db_ref: DatabaseRef,
        log: LoadedLogId,
        name: String,
        text: Arc<str>,
        filters: Vec<StoredLogFilter>,
    ) {
        self.send_write(WriteRequest::AttachLog {
            db_ref,
            log,
            name,
            text,
            filters,
        });
    }

    pub fn load_attached_logs(&self, db_ref: DatabaseRef) {
        self.send_read(ReadRequest::LoadAttachedLogs(db_ref));
    }

    pub fn load_attached_log(&self, attachment: LogAttachmentRef, name: String) {
        self.send_read(ReadRequest::LoadAttachedLog { attachment, name });
    }

    pub fn set_attached_log_filters(
        &self,
        attachment: LogAttachmentRef,
        filters: Vec<StoredLogFilter>,
    ) {
        self.send_write(WriteRequest::SetAttachedLogFilters {
            attachment,
            filters,
        });
    }

    pub fn detach_log(&self, attachment: LogAttachmentRef, log: LoadedLogId, name: String) {
        self.send_write(WriteRequest::DetachLog {
            attachment,
            log,
            name,
        });
    }

    pub fn find_duplicate_attachment(&self, db_ref: DatabaseRef, log: LoadedLogId, text: Arc<str>) {
        self.send_read(ReadRequest::FindDuplicateAttachment { db_ref, log, text });
    }

    pub fn auto_prune(&self, max_bytes: u64, confirm: bool) {
        self.send_write(WriteRequest::AutoPrune { max_bytes, confirm });
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
    mut db: RecordingsHandle,
    req_rx: &Receiver<Request>,
    resp_tx: &Sender<Response>,
    ctx: &Context,
    pending_writes: &PendingWrites,
) {
    while let Ok(req) = req_rx.recv() {
        let resp = match req {
            Request::Read(req) => handle_read_request(db.read(), req),
            Request::Write(req) => run_write_request(&mut db, req, pending_writes),
        };
        // If the UI is gone the send fails, there is nothing left to repaint.
        if resp_tx.send(resp).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

/// Run a write request on the writable database, under a
/// [`PendingWriteGuard`].
///
/// A read-only session has no [`RecordingsHandle::writer`]: its writes come
/// back as [`WriteRejection::ReadOnlySession`], which is what the write
/// registry rejects them with in such a session.
fn run_write_request(
    db: &mut RecordingsHandle,
    req: WriteRequest,
    pending_writes: &PendingWrites,
) -> Response {
    let label = req.database_write_label();
    let Some(db) = db.writer() else {
        return Response::WriteRejected {
            label,
            rejection: WriteRejection::ReadOnlySession,
        };
    };
    match pending_writes.try_begin(label, WriteKind::RecordingDatabase) {
        Ok(_write) => handle_write_request(db, req),
        Err(rejection) => Response::WriteRejected { label, rejection },
    }
}

fn handle_read_request(db: &ReadOnlyRecordings, req: ReadRequest) -> Response {
    match req {
        ReadRequest::List => Response::Listed(db.list_recordings()),
        ReadRequest::Open(db_ref) => {
            let result = db.load(&db_ref);
            Response::Opened { db_ref, result }
        }
        ReadRequest::PrunePreview(mode) => Response::PrunePreview(db.prune_candidates(&mode)),
        ReadRequest::LoadSnapRuns(db_ref) => {
            let blob = db.snap_blob(&db_ref);
            Response::SnapRunsLoaded { db_ref, blob }
        }
        ReadRequest::LoadAttachedLogs(db_ref) => {
            let attachments = db.log_attachments(&db_ref).map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| RestoredLogAttachment {
                        log: db.load_attached_log(&db_ref, entry.id),
                        entry,
                    })
                    .collect()
            });
            Response::AttachedLogsLoaded {
                db_ref,
                attachments,
            }
        }
        ReadRequest::LoadAttachedLog { attachment, name } => {
            let log = db.load_attached_log(&attachment.recording, attachment.id);
            Response::AttachedLogLoaded {
                attachment,
                name,
                log,
            }
        }
        ReadRequest::FindDuplicateAttachment { db_ref, log, text } => {
            let existing = db
                .log_attachment_with_content(&db_ref, LogContentHash::of_log_bytes(text.as_bytes()))
                .map(|entry| {
                    entry.map(|entry| ExistingLogAttachment {
                        id: entry.id,
                        name: entry.attachment.name,
                    })
                });
            Response::DuplicateAttachmentFound {
                log,
                recording: db_ref,
                existing,
            }
        }
    }
}

fn handle_write_request(db: &mut Recordings, req: WriteRequest) -> Response {
    match req {
        WriteRequest::SetTracksShelved {
            db_ref,
            rows,
            shelved,
        } => {
            let count = rows.len();
            let result = db.set_tracks_shelved(&db_ref, &rows, shelved);
            Response::Mutated {
                op: DbOp::TracksShelved { count },
                result,
            }
        }
        WriteRequest::DeleteTracks { db_ref, rows } => {
            let count = rows.len();
            let result = purge_tracks(db, &db_ref, &rows);
            Response::Mutated {
                op: DbOp::TracksDeleted { count },
                result,
            }
        }
        WriteRequest::DeleteShelvedTracks => match purge_all_shelved(db) {
            Ok(count) => Response::Mutated {
                op: DbOp::TracksDeleted { count },
                result: Ok(()),
            },
            Err(e) => Response::Mutated {
                op: DbOp::TracksDeleted { count: 0 },
                result: Err(e),
            },
        },
        WriteRequest::DeleteRecordings { refs, reason } => {
            let count = refs.len();
            let result = db.delete_batch(&refs);
            Response::Mutated {
                op: DbOp::RecordingsDeleted { count, reason },
                result,
            }
        }
        WriteRequest::RenameIdentity { old, new } => {
            let result = db.rename_identity(&old, &new);
            Response::Mutated {
                op: DbOp::IdentityRenamed { old, new },
                result,
            }
        }
        WriteRequest::AutoPrune { max_bytes, confirm } => {
            Response::AutoPruned(auto_prune::run(db, max_bytes, confirm))
        }
        WriteRequest::StoreSnapRuns { db_ref, blob } => {
            Response::SnapRunsStored(db.set_snap_blob(&db_ref, &blob))
        }
        WriteRequest::AttachLog {
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
                        filters,
                    },
                )
                .map(|entry| StoredLogAttachment {
                    recording: db_ref,
                    entry,
                });
            Response::LogAttached { log, name, result }
        }
        WriteRequest::SetAttachedLogFilters {
            attachment,
            filters,
        } => Response::AttachedLogFiltersStored(db.set_attached_log_filters(
            &attachment.recording,
            attachment.id,
            filters,
        )),
        WriteRequest::DetachLog {
            attachment,
            log,
            name,
        } => {
            let result = db.detach_log(&attachment.recording, attachment.id);
            Response::LogDetached {
                attachment,
                log,
                name,
                result,
            }
        }
    }
}

/// Permanently remove every shelved track across all recordings, re-encoding
/// each affected recording. Returns the number of tracks removed.
fn purge_all_shelved(db: &mut Recordings) -> Result<usize, DbError> {
    let entries = db.list_recordings()?;
    let mut deleted = 0;
    for entry in entries {
        if entry.shelved_tracks == 0 {
            continue;
        }
        let stored = db.load(&entry.db_ref)?;
        let shelved_rows: Vec<usize> = stored
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.state == TrackState::Shelved)
            .map(|(row, _)| row)
            .collect();
        deleted += shelved_rows.len();
        purge_tracks_with_stored(db, &entry.db_ref, &stored, &shelved_rows)?;
    }
    Ok(deleted)
}

/// Permanently remove the tracks in the stored table rows `drop_rows` from one
/// recording.
fn purge_tracks(
    db: &mut Recordings,
    db_ref: &DatabaseRef,
    drop_rows: &[usize],
) -> Result<(), DbError> {
    let stored = db.load(db_ref)?;
    purge_tracks_with_stored(db, db_ref, &stored, drop_rows)
}

/// Re-encode `db_ref` with the nav points of the tracks in the stored table rows
/// `drop_rows` removed, and store the new bytes under the same reference.
///
/// A row keeps its place for the life of the recording. A dropped row becomes
/// a [`TrackState::Deleted`] tombstone, and the rows after it stay where they
/// are. The surviving rows range-shift onto the compacted nav point sequence,
/// and each tombstone takes the empty range at the offset where its nav points
/// began.
/// When no row survives, the whole recording is deleted instead (an empty
/// re-encode would fail).
///
/// Returns [`DbError::TrackIndexOutOfRange`] or [`DbError::TrackAlreadyDeleted`]
/// and leaves the recording as it is when `drop_rows` holds a row past the end
/// of the table or a row that already holds a tombstone.
///
/// The recording's stored snap runs are dropped by
/// [`HistoryDatabase::replace_recording_in_place`]: the runs name point indices
/// that the re-encode shifts.
fn purge_tracks_with_stored(
    db: &mut Recordings,
    db_ref: &DatabaseRef,
    stored: &StoredRecording,
    drop_rows: &[usize],
) -> Result<(), DbError> {
    // One unusable row rejects the whole request. The caller numbered the
    // tracks against another table, and the rows that do hold a track may hold
    // tracks other than the ones that the user chose.
    for &row in drop_rows {
        match stored.tracks.get(row) {
            None => {
                return Err(DbError::TrackIndexOutOfRange {
                    index: row,
                    stored_track_count: stored.tracks.len(),
                });
            }
            Some(track) if track.state == TrackState::Deleted => {
                return Err(DbError::TrackAlreadyDeleted { index: row });
            }
            Some(_) => {}
        }
    }

    let drop: HashSet<usize> = drop_rows.iter().copied().collect();

    let a_track_survives = stored
        .tracks
        .iter()
        .enumerate()
        .any(|(row, t)| !drop.contains(&row) && t.state != TrackState::Deleted);
    if !a_track_survives {
        return db.delete_batch(std::slice::from_ref(db_ref));
    }

    let drop_ranges: Vec<Range<usize>> = stored
        .tracks
        .iter()
        .enumerate()
        .filter(|(row, _)| drop.contains(row))
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

    // Range-shift the table onto the compacted nav points. A surviving row
    // keeps its length and starts where the previous row ended, and a tombstone
    // takes the empty range there.
    let mut cursor = 0;
    let mut new_tracks = Vec::with_capacity(stored.tracks.len());
    for (row, t) in stored.tracks.iter().enumerate() {
        let deleted = drop.contains(&row) || t.state == TrackState::Deleted;
        let len = if deleted {
            0
        } else {
            t.end.saturating_sub(t.start)
        };
        new_tracks.push(TrackRange {
            start: cursor,
            end: cursor + len,
            state: if deleted {
                TrackState::Deleted
            } else {
                t.state
            },
        });
        cursor += len;
    }

    let settings = stored
        .segmentation
        .unwrap_or_else(|| stored_segmentation_from_config(&SegmentationConfig::default()));
    let meta = gt_store::extract_meta(&new_bytes)?;

    db.replace_recording_in_place(db_ref, &meta, &new_tracks, settings, &new_bytes)
}

#[cfg(test)]
mod tests {
    use gt_pending_writes::WriteAccess;
    use gt_store::TrackRange;
    use gt_test_utils::pending_writes;
    use rstest::rstest;

    use crate::app::history_test_support::{
        bytes_starting_at, listed_recordings, next_response, only_recording, sample_bytes,
        seed_two_track_recording, store_recording, worker_on,
    };

    use super::*;

    #[test]
    fn worker_round_trips_every_operation() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");

        seed_two_track_recording(&path);

        let worker = worker_on(&path);
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

        // Shelve one track.
        worker.set_tracks_shelved(db_ref.clone(), vec![0], true);
        let Response::Mutated {
            op: DbOp::TracksShelved { count },
            result,
        } = next_response(&worker)
        else {
            panic!("expected a TracksShelved mutation");
        };
        assert_eq!(count, 1);
        result.expect("the shelve runs");

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
    fn worker_permanently_deletes_shelved_tracks() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");

        seed_two_track_recording(&path);

        let worker = worker_on(&path);

        // Shelve the first track, then permanently delete every shelved track.
        let db_ref = only_recording(&worker).db_ref;
        worker.set_tracks_shelved(db_ref, vec![0], true);
        next_response(&worker);

        worker.delete_shelved_tracks();
        let Response::Mutated {
            op: DbOp::TracksDeleted { count },
            result,
        } = next_response(&worker)
        else {
            panic!("expected TracksDeleted");
        };
        assert_eq!(count, 1, "one shelved track removed");
        result.expect("delete ok");

        // The recording now has a single ten-point track, every one of them live.
        worker.list();
        let Response::Listed(Ok(entries)) = next_response(&worker) else {
            panic!("expected Listed");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].total_tracks, 1);
        assert_eq!(entries[0].shelved_tracks, 0);
        assert_eq!(entries[0].meta.nav_point_count, 10);

        let new_ref = entries[0].db_ref.clone();
        worker.open(new_ref);
        let Response::Opened { result, .. } = next_response(&worker) else {
            panic!("expected Opened");
        };
        let stored = result.expect("open ok");
        assert_eq!(
            stored.tracks,
            vec![
                TrackRange {
                    start: 0,
                    end: 0,
                    state: TrackState::Deleted,
                },
                TrackRange {
                    start: 0,
                    end: 10,
                    state: TrackState::Live,
                }
            ],
            "the deleted track leaves a tombstone, and the track that stays keeps its row"
        );
    }

    #[test]
    fn deleting_a_track_index_the_stored_table_does_not_have_reports_the_tracks_it_removed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let worker = worker_on(&path);

        let recording = only_recording(&worker);
        let tracks_before = recording.total_tracks;
        worker.delete_tracks(recording.db_ref, vec![2]);
        let Response::Mutated { op, result } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        // A failed mutation counts as zero here: only a successful one raises
        // a toast with a count.
        let reported = match (op, result) {
            (DbOp::TracksDeleted { count }, Ok(())) => count,
            _ => 0,
        };

        let tracks_lost = tracks_before - only_recording(&worker).total_tracks;
        assert_eq!(
            reported, tracks_lost,
            "the delete reports the number of tracks it removed"
        );
        worker.shutdown();
    }

    /// One index of the request is in range and one is past the end, which is
    /// what a delete of two tracks looks like once the stored table is shorter
    /// than the session's numbering.
    #[test]
    fn deleting_a_track_index_the_stored_table_does_not_have_removes_no_track_at_all() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let worker = worker_on(&path);

        worker.delete_tracks(only_recording(&worker).db_ref, vec![0, 2]);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        let Err(DbError::TrackIndexOutOfRange {
            index,
            stored_track_count,
        }) = result
        else {
            panic!("expected the delete to name the index the table does not have");
        };
        assert_eq!((index, stored_track_count), (2, 2));

        let recording = only_recording(&worker);
        assert_eq!(recording.total_tracks, 2);
        assert_eq!(recording.meta.nav_point_count, 20);
        worker.shutdown();
    }

    #[test]
    fn shelving_a_track_after_a_delete_re_encoded_the_recording_stores_the_shelve() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let worker = worker_on(&path);

        let session_ref = only_recording(&worker).db_ref;
        worker.delete_tracks(session_ref.clone(), vec![0]);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the delete runs");

        // The track that the session still holds sits in stored row 1, where
        // the delete left it.
        worker.set_tracks_shelved(session_ref, vec![1], true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the shelve runs");

        assert_eq!(
            only_recording(&worker).shelved_tracks,
            1,
            "the track the session shelved is shelved in history"
        );
        worker.shutdown();
    }

    /// The History window's "Delete shelved data" sweeps every recording,
    /// including the one this session has open.
    #[test]
    fn shelving_a_track_after_the_shelved_data_sweep_re_encoded_the_recording_stores_the_shelve() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let worker = worker_on(&path);

        let session_ref = only_recording(&worker).db_ref;
        worker.set_tracks_shelved(session_ref.clone(), vec![0], true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the first shelve runs");

        worker.delete_shelved_tracks();
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the sweep runs");

        // The track that the session still holds sits in stored row 1, where
        // the sweep left it.
        worker.set_tracks_shelved(session_ref, vec![1], true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the second shelve runs");

        assert_eq!(
            only_recording(&worker).shelved_tracks,
            1,
            "the track the session shelved is shelved in history"
        );
        worker.shutdown();
    }

    #[test]
    fn deleting_a_stored_row_that_holds_a_tombstone_removes_no_track_at_all() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let worker = worker_on(&path);

        let db_ref = only_recording(&worker).db_ref;
        worker.delete_tracks(db_ref.clone(), vec![0]);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the first delete runs");

        worker.delete_tracks(db_ref, vec![0]);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        let Err(DbError::TrackAlreadyDeleted { index }) = result else {
            panic!("expected the delete to report the row that holds a tombstone");
        };
        assert_eq!(index, 0);

        let recording = only_recording(&worker);
        assert_eq!(recording.total_tracks, 1);
        assert_eq!(recording.meta.nav_point_count, 10);
        worker.shutdown();
    }

    /// The sweep addresses the stored rows too. A tombstone that an earlier
    /// delete left keeps its row, and the sweep reads the rows around it.
    #[test]
    fn the_shelved_data_sweep_of_a_recording_with_a_tombstone_deletes_the_shelved_track() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        store_recording(
            &path,
            &sample_bytes(),
            &[
                TrackRange {
                    start: 0,
                    end: 7,
                    state: TrackState::Live,
                },
                TrackRange {
                    start: 7,
                    end: 14,
                    state: TrackState::Live,
                },
                TrackRange {
                    start: 14,
                    end: 20,
                    state: TrackState::Live,
                },
            ],
        );
        let worker = worker_on(&path);

        let db_ref = only_recording(&worker).db_ref;
        worker.delete_tracks(db_ref.clone(), vec![0]);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the delete runs");

        worker.set_tracks_shelved(db_ref.clone(), vec![1], true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the shelve runs");

        worker.delete_shelved_tracks();
        let Response::Mutated {
            op: DbOp::TracksDeleted { count },
            result,
        } = next_response(&worker)
        else {
            panic!("expected a TracksDeleted mutation");
        };
        result.expect("the sweep runs");
        assert_eq!(count, 1);

        assert_eq!(
            only_recording(&worker).meta.nav_point_count,
            6,
            "the recording keeps the six points of its last track"
        );
        worker.open(db_ref);
        let Response::Opened { result, .. } = next_response(&worker) else {
            panic!("expected an Opened response");
        };
        assert_eq!(
            result.expect("the recording opens").tracks,
            vec![
                TrackRange {
                    start: 0,
                    end: 0,
                    state: TrackState::Deleted,
                },
                TrackRange {
                    start: 0,
                    end: 0,
                    state: TrackState::Deleted,
                },
                TrackRange {
                    start: 0,
                    end: 6,
                    state: TrackState::Live,
                }
            ]
        );
        worker.shutdown();
    }

    /// The logs the user stored with a recording belong to the recording, not
    /// to one of its tracks.
    #[test]
    fn deleting_one_track_of_a_recording_keeps_the_logs_attached_to_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let worker = worker_on(&path);

        let db_ref = only_recording(&worker).db_ref;
        worker.attach_log(
            db_ref.clone(),
            LoadedLogId::new(1),
            "field-notes.log".to_owned(),
            "one line".into(),
            Vec::new(),
        );
        let Response::LogAttached { result, .. } = next_response(&worker) else {
            panic!("expected a LogAttached response");
        };
        result.expect("the log is attached");

        worker.delete_tracks(db_ref, vec![0]);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the delete runs");

        worker.load_attached_logs(only_recording(&worker).db_ref);
        let Response::AttachedLogsLoaded { attachments, .. } = next_response(&worker) else {
            panic!("expected an AttachedLogsLoaded response");
        };
        assert_eq!(
            attachments.expect("the attachments are read").len(),
            1,
            "the log stored with the recording survives the delete of one of its tracks"
        );
        worker.shutdown();
    }

    /// The duplicate check matches the re-encoded recording against the second
    /// stored one: both hold the same ten points.
    #[test]
    fn deleting_a_track_keeps_the_recording_when_a_stored_one_matches_what_is_left() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        store_recording(
            &path,
            &bytes_starting_at(1_748_000_010, 10),
            &[TrackRange {
                start: 0,
                end: 10,
                state: TrackState::Live,
            }],
        );

        let worker = worker_on(&path);
        let db_ref = listed_recordings(&worker)
            .iter()
            .find(|entry| entry.total_tracks == 2)
            .map(|entry| entry.db_ref.clone())
            .expect("the two-track recording is listed");

        worker.delete_tracks(db_ref, vec![0]);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the delete runs");

        assert_eq!(
            listed_recordings(&worker).len(),
            2,
            "the recording the delete re-encoded is still stored"
        );
        worker.shutdown();
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
        let worker = HistoryWorker::spawn(
            RecordingsHandle::Owner(db),
            Context::default(),
            pending_writes.clone(),
        );
        let db_ref = only_recording(&worker).db_ref;

        worker.set_tracks_shelved(db_ref, vec![0], true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };

        result.expect("the shelve runs");
        assert_eq!(
            pending_writes.snapshot().recently_finished,
            vec!["Shelving tracks in recording history"],
            "the listing before it registered nothing, the shelve registered while it ran"
        );
        assert!(pending_writes.is_idle());
        worker.shutdown();
    }

    #[rstest]
    #[case::shutting_down(pending_writes::shutting_down_registry(), WriteRejection::ShuttingDown)]
    #[case::read_only_session(
        PendingWrites::new(WriteAccess::ReadOnly),
        WriteRejection::ReadOnlySession
    )]
    fn a_rejected_mutation_returns_its_reason_and_leaves_the_database_alone(
        #[case] pending_writes: PendingWrites,
        #[case] expected: WriteRejection,
    ) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let db = Recordings::open_or_create(&path).expect("reopen");
        let worker = HistoryWorker::spawn(
            RecordingsHandle::Owner(db),
            Context::default(),
            pending_writes.clone(),
        );
        let db_ref = only_recording(&worker).db_ref;

        worker.set_tracks_shelved(db_ref, vec![0], true);

        let Response::WriteRejected { label, rejection } = next_response(&worker) else {
            panic!("expected the write to be rejected");
        };
        assert_eq!(rejection, expected);
        assert_eq!(label, "Shelving tracks in recording history");
        assert!(pending_writes.is_idle());

        // Reads still return, and report a database the rejected write left alone.
        worker.list();
        let Response::Listed(Ok(entries)) = next_response(&worker) else {
            panic!("expected a Listed response");
        };
        assert_eq!(entries.first().map(|entry| entry.shelved_tracks), Some(0));
        worker.shutdown();
    }

    /// The write registry allows this session's writes, and the read-only
    /// handle still has no [`RecordingsHandle::writer`] to run one on.
    #[test]
    fn a_write_on_a_read_only_handle_is_rejected_where_the_registry_allows_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_two_track_recording(&path);
        let pending_writes = PendingWrites::default();
        let db = ReadOnlyRecordings::open_existing_read_only(&path).expect("open read-only");
        let worker = HistoryWorker::spawn(
            RecordingsHandle::ReadOnly(db),
            Context::default(),
            pending_writes.clone(),
        );
        let db_ref = only_recording(&worker).db_ref;

        worker.set_tracks_shelved(db_ref, vec![0], true);

        let Response::WriteRejected { label, rejection } = next_response(&worker) else {
            panic!("expected the write to be rejected");
        };
        assert_eq!(rejection, WriteRejection::ReadOnlySession);
        assert_eq!(label, "Shelving tracks in recording history");
        assert_eq!(
            pending_writes.snapshot().recently_finished,
            Vec::<String>::new(),
            "the rejected write registered with the write registry"
        );

        // Reads still return, and report a database the rejected write left alone.
        worker.list();
        let Response::Listed(Ok(entries)) = next_response(&worker) else {
            panic!("expected a Listed response");
        };
        assert_eq!(entries.first().map(|entry| entry.shelved_tracks), Some(0));
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
    #[case(
        WriteRequest::SetTracksShelved {
            db_ref: db_ref(),
            rows: vec![0],
            shelved: true,
        },
        "Shelving tracks in recording history"
    )]
    #[case(
        WriteRequest::SetTracksShelved {
            db_ref: db_ref(),
            rows: vec![0],
            shelved: false,
        },
        "Unshelving tracks in recording history"
    )]
    #[case(
        WriteRequest::DeleteTracks {
            db_ref: db_ref(),
            rows: vec![0],
        },
        "Deleting tracks from recording history"
    )]
    #[case(
        WriteRequest::DeleteShelvedTracks,
        "Deleting shelved tracks from recording history"
    )]
    #[case(
        WriteRequest::DeleteRecordings {
            refs: vec![db_ref()],
            reason: DeleteReason::Manual,
        },
        "Deleting recordings from recording history"
    )]
    #[case(
        WriteRequest::RenameIdentity {
            old: "dev".to_owned(),
            new: "rover".to_owned(),
        },
        "Renaming an identity in recording history"
    )]
    #[case(
        WriteRequest::AutoPrune {
            max_bytes: 0,
            confirm: false,
        },
        "Auto-pruning recording history"
    )]
    #[case(
        WriteRequest::StoreSnapRuns {
            db_ref: db_ref(),
            blob: Vec::new(),
        },
        "Storing snap runs in recording history"
    )]
    #[case(
        WriteRequest::AttachLog {
            db_ref: db_ref(),
            log: LoadedLogId::new(1),
            name: "navsyncd.log".to_owned(),
            text: "boot".into(),
            filters: Vec::new(),
        },
        "Storing a log with a recording"
    )]
    #[case(
        WriteRequest::SetAttachedLogFilters {
            attachment: attachment_ref(),
            filters: Vec::new(),
        },
        "Storing an attached log's filters"
    )]
    #[case(
        WriteRequest::DetachLog {
            attachment: attachment_ref(),
            log: LoadedLogId::new(1),
            name: "navsyncd.log".to_owned(),
        },
        "Removing an attached log from a recording"
    )]
    fn each_write_request_carries_the_label_the_registry_lists_it_under(
        #[case] request: WriteRequest,
        #[case] label: &str,
    ) {
        assert_eq!(request.database_write_label(), label);
    }
}
