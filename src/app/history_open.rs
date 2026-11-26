use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use egui::{Button, Grid, Label, RichText};
use gt_loaded_files::LoadedFileId;
use gt_pending_writes::{PendingWriteGuard, WriteKind};
use gt_session_bridge::ErrorCode;
use gt_store::{
    DatabaseRef, DbError, HistoryEntries, RecordingMeta, StoredFixPlacementRule,
    StoredTrackSplitRule, TrackState,
};
use gt_ui_components::HeldBodyLines;

use super::anchored_dialog::AnchoredDialogKind;
use super::association_batches::RecordingOperationOrigin;
use super::auto_prune::AutoPruneOutcome;
use super::history::{self, HistoryWindowFrame};
use super::history_db::{ListRequestId, OpenedRecording, SessionRestoration};
use super::load_failures::{LoadFailure, LoadFailureCause, LoadReadRequest, LoadSource};
use super::loader::{
    AttachedLogRequester, CompletedLoad, LoadCompletion, LoadedRecordingPlacement,
    RecalculationAuthority, StoredHistoryLoadSettings,
};
use super::session_history::OpenStop;
use super::{App, ResegmentPrompt, SharedAppState, history_db, loader, modals, storage};

/// What the user chose in the prompt for a recordings database that would not
/// open. Each failure offers one of the three remedies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryFailureChoice {
    ClearTheWriteLock,
    /// Go on with the database left as it is: recordings load but are not
    /// stored.
    Dismiss,
    Recreate,
    Reopen,
}

/// What the user chose in the "Track settings differ" prompt.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ResegmentChoice {
    Cancel,
    RecalculateWithCurrentSettings,
    UseStoredTracks,
}

/// The marks the user set by hand on the recording the "Track settings differ"
/// prompt is about: the shelved tracks of its stored track table, and the
/// hidden tracks of its stored UI state.
///
/// A recalculation writes a track table with every track live and drops the
/// recording's hidden track numbers, which addressed the previous track
/// boundaries.
#[derive(Clone, Copy, PartialEq, Eq)]
struct MarksDroppedByRecalculating {
    shelved_tracks: usize,
    hidden_tracks: usize,
}

impl MarksDroppedByRecalculating {
    fn of_the_prompted_recording(
        prompt: &ResegmentPrompt,
        tree: &gt_side_panel::TreeState,
        saved_hidden_track_count: Option<usize>,
    ) -> Self {
        Self {
            shelved_tracks: prompt
                .track_snapshot
                .rows()
                .iter()
                .filter(|track| track.state == TrackState::Shelved)
                .count(),
            hidden_tracks: tree
                .hidden_track_numbers_for_layout(prompt.track_snapshot.layout())
                .map_or(saved_hidden_track_count.unwrap_or_default(), BTreeSet::len),
        }
    }

    /// The line the prompt shows with the recalculate choice, and `None` for a
    /// recording with neither kind of mark.
    fn warning_line(self) -> Option<String> {
        let Self {
            shelved_tracks,
            hidden_tracks,
        } = self;
        let shelved_label = gt_fmt::pluralize(shelved_tracks, "track", "tracks");
        let hidden_label = gt_fmt::pluralize(hidden_tracks, "track", "tracks");
        match (shelved_tracks, hidden_tracks) {
            (0, 0) => None,
            (0, _) => Some(format!(
                "Recalculating shows the {hidden_tracks} hidden {hidden_label} of this recording."
            )),
            (_, 0) => Some(format!(
                "Recalculating puts the {shelved_tracks} shelved {shelved_label} of this \
                 recording back in the working set."
            )),
            _ => Some(format!(
                "Recalculating puts the {shelved_tracks} shelved {shelved_label} of this \
                 recording back in the working set and shows its {hidden_tracks} hidden \
                 {hidden_label}."
            )),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AutoPruneChoice {
    Cancel,
    Delete,
}

impl App {
    pub(super) fn sync_db_path(&mut self) {
        self.loader.db_path = if self.storage_settings.enabled {
            self.history.path().map(std::path::Path::to_owned)
        } else {
            None
        };
    }

    /// Follow an auto-store edit made by either place the storage controls
    /// render, with `storage_before_edit` read before they rendered.
    pub(super) fn sync_db_path_if_auto_store_changed(
        &mut self,
        storage_before_edit: crate::settings::StorageSettings,
    ) {
        if self.storage_settings.enabled != storage_before_edit.enabled {
            self.sync_db_path();
        }
    }

    /// Put `worker` behind the app's history, ending the worker it replaces.
    ///
    /// Ending the previous worker joins its thread, blocking until the request
    /// it is on finishes.
    pub(super) fn install_history_worker(&mut self, worker: history_db::HistoryWorker) {
        self.session_history.expire();
        self.pending_resegment
            .take_if(|prompt| prompt.origin.session_history().is_some());
        self.queued_resegments
            .retain(|prompt| prompt.origin.session_history().is_none());
        let previous = std::mem::replace(&mut self.history, worker);
        previous.shutdown();
        self.sync_db_path();
        self.invalidate_history_listing();
        self.store_the_hidden_tracks_the_settings_file_lists();
    }

    pub(super) fn expire_stale_history_settings(&mut self) {
        if let Some(operation) = self.session_history.expire_stale_settings_decision() {
            self.invalidate_history_listing();
            let is_expired =
                |prompt: &ResegmentPrompt| prompt.origin.session_history() == Some(operation);
            self.pending_resegment.take_if(|prompt| is_expired(prompt));
            self.queued_resegments.retain(|prompt| !is_expired(prompt));
        }
    }

    /// Put a freshly opened database behind the history worker.
    fn adopt_history_database(
        &mut self,
        db: gt_store::Recordings,
        ctx: &egui::Context,
        toast: &str,
    ) {
        self.install_history_worker(history_db::HistoryWorker::spawn(
            gt_store::RecordingsHandle::Owner(db),
            ctx.clone(),
            self.pending_writes.clone(),
        ));
        self.toasts.info(toast);
    }

    /// Registers a write to the recordings database, logging a rejection at
    /// debug level.
    fn try_begin_recording_history_write(&self, label: &'static str) -> Option<PendingWriteGuard> {
        match self
            .pending_writes
            .try_begin(label, WriteKind::RecordingDatabase)
        {
            Ok(write) => Some(write),
            Err(rejection) => {
                log::debug!("Did not run {label:?}: {rejection}");
                None
            }
        }
    }

    /// Retry opening after a transient failure, e.g. another process released
    /// the file.
    pub(super) fn reopen_history_database(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        let Some(_write) = self.try_begin_recording_history_write(OPENING_THE_DATABASE) else {
            return;
        };
        match storage::reopen_recordings(path) {
            Ok(db) => self.adopt_history_database(db, ctx, "Opened the history database"),
            Err(failure) => {
                log::warn!(
                    "History database at {} still unavailable",
                    failure.path().display()
                );
                self.history_failure = Some(failure);
            }
        }
    }

    /// Clear a stale write lock and bring the history database online, after the
    /// user confirmed no other process is using it.
    pub(super) fn recover_history_database(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        use gt_store::HistoryDatabase;
        let Some(_write) = self.try_begin_recording_history_write(CLEARING_THE_WRITE_LOCK) else {
            return;
        };
        let result = gt_store::Recordings::clear_write_lock(path)
            .and_then(|()| gt_store::Recordings::open_or_create(path));
        match result {
            Ok(db) => {
                self.adopt_history_database(db, ctx, "Recovered the history database");
            }
            Err(e) => {
                log::error!("Failed to recover history database: {e}");
                self.toasts
                    .error(format!("Could not recover history database: {e}"));
            }
        }
    }

    /// Recreate a corrupted history database from scratch, optionally renaming the
    /// unreadable original to `<name>.corrupt.bak` first.
    pub(super) fn recreate_history_database(
        &mut self,
        path: &std::path::Path,
        keep_backup: bool,
        ctx: &egui::Context,
    ) {
        use gt_store::HistoryDatabase;
        let Some(_write) = self.try_begin_recording_history_write(RECREATING_THE_DATABASE) else {
            return;
        };
        if keep_backup {
            let backup = corrupt_backup_path(path);
            if let Err(e) = std::fs::rename(path, &backup) {
                log::error!("Failed to back up corrupted database: {e}");
                self.toasts
                    .error(format!("Could not back up the database: {e}"));
                return;
            }
            log::info!("Backed up corrupted database to {}", backup.display());
        } else if let Err(e) = std::fs::remove_file(path) {
            log::error!("Failed to remove corrupted database: {e}");
            self.toasts
                .error(format!("Could not remove the database: {e}"));
            return;
        }

        match gt_store::Recordings::open_or_create(path) {
            Ok(db) => {
                self.adopt_history_database(db, ctx, "Created a fresh history database");
            }
            Err(e) => {
                log::error!("Failed to recreate history database: {e}");
                self.toasts
                    .error(format!("Could not recreate the database: {e}"));
            }
        }
    }

    /// Query the history worker for whether auto-pruning is needed. The result
    /// comes back as a [`history_db::Response::AutoPruned`]. Called after each
    /// successful GTD insert.
    pub(super) fn check_auto_prune(&self) {
        if !self.storage_settings.auto_prune_enabled {
            return;
        }
        self.history.auto_prune(
            self.storage_settings.auto_prune_max_bytes,
            self.storage_settings.auto_prune_confirm,
        );
    }

    /// Begin opening a recording from history. Reproduces the stored tracks,
    /// leaves the shelved ones out, and regenerates markers with current
    /// settings.
    /// When a stored track setting differs from the current one it raises a
    /// prompt instead (recalculate vs. use the stored tracks).
    fn begin_history_open(
        &mut self,
        placement: loader::LoadedRecordingPlacement,
        mut opened: OpenedRecording,
        origin: RecordingOperationOrigin,
        previous_settings: Option<StoredHistoryLoadSettings>,
    ) {
        // Reuse the original filename: the identity is the filename (with an
        // "auto:" prefix for auto-derived ones).
        let db_ref = opened.track_snapshot.recording();
        let filename = db_ref
            .identity
            .strip_prefix("auto:")
            .unwrap_or(&db_ref.identity)
            .to_owned();

        let mode = loader::GtdLoadMode::for_stored_recording(&opened.stored);
        let previous_settings = previous_settings.filter(|settings| {
            opened
                .stored
                .segmentation
                .is_none_or(|stored| loader::stored_tracks_match_config(&stored, &settings.config))
        });
        let settings_differ = previous_settings.is_none()
            && !opened.track_snapshot.rows().is_empty()
            && opened.stored.segmentation.is_some_and(|settings| {
                !loader::stored_tracks_match_config(&settings, &self.processing_config)
            });
        if let Some(operation) = origin.session_history() {
            if settings_differ {
                self.session_history.await_settings(operation, &mut opened);
            } else if !self.session_history.begin_load(
                operation,
                self.shared.borrow().loaded_files.view(),
                &mut opened,
                self.processing_config,
            ) {
                return;
            }
        }
        let OpenedRecording {
            stored,
            track_snapshot,
            ui_state,
            ..
        } = opened;
        if let Some(ui_state) = ui_state {
            self.apply_the_ui_state_stored_with_a_recording(
                track_snapshot.layout().clone(),
                ui_state,
            );
        }
        match (previous_settings, stored.segmentation) {
            // A stored track setting differs from the current one: let the user
            // choose before changing track ranges that the shelved state may
            // refer to, or the positions the fixes are drawn at.
            (None, Some(stored_settings)) if settings_differ => {
                let marker_settings_changed = !loader::marker_settings_match_config(
                    &stored_settings,
                    &self.processing_config,
                );
                if let StoredTrackSplitRule::Unrecognized(rule) = stored_settings.track_split_rule {
                    log::warn!(
                        "'{filename}' has tracks split by rule {rule}, which this version does not implement. They can only be recalculated."
                    );
                }
                if let StoredFixPlacementRule::Unrecognized(rule) =
                    stored_settings.fix_placement_rule
                {
                    log::warn!(
                        "'{filename}' has fixes placed by rule {rule}, which this version does not implement. They can only be recalculated."
                    );
                }
                let prompt = ResegmentPrompt {
                    origin,
                    filename,
                    bytes: stored.bytes.into(),
                    stored: stored_settings,
                    track_snapshot,
                    marker_settings_changed,
                    mode,
                    placement,
                };
                if self.pending_resegment.is_some() || !self.queued_resegments.is_empty() {
                    self.queued_resegments.push_back(prompt);
                } else {
                    self.pending_resegment = Some(prompt);
                }
            }
            (previous, stored_settings) => {
                let settings = previous.unwrap_or_else(|| {
                    let config = stored_settings.map_or(self.processing_config, |stored| {
                        loader::config_from_stored_segmentation(&stored, self.processing_config)
                    });
                    let applied_current_marker_settings = stored_settings.is_some_and(|stored| {
                        !loader::marker_settings_match_config(&stored, &self.processing_config)
                    });
                    StoredHistoryLoadSettings {
                        config,
                        mode,
                        applied_current_marker_settings,
                    }
                });
                let track_table = loader::OpenTrackTable::from_stored_recording(
                    &stored.track_table,
                    track_snapshot,
                );
                self.loader.spawn_gtd_bytes(
                    stored.bytes.into(),
                    filename,
                    settings.config,
                    settings.mode,
                    Some(loader::HistoryOpen::ApplyShelved {
                        track_table,
                        applied_current_marker_settings: settings.applied_current_marker_settings,
                        placement,
                    }),
                    origin,
                );
            }
        }
    }

    pub(super) fn refresh_expired_history_load(
        &self,
        completed: CompletedLoad,
    ) -> Option<CompletedLoad> {
        match completed {
            CompletedLoad {
                stored_history: Some(previous),
                completion:
                    LoadCompletion::Recording {
                        origin,
                        result: Ok(result),
                    },
                ..
            } if !previous.layout.is_current() => {
                self.history.refresh_stored_open(
                    previous.layout.recording().clone(),
                    origin,
                    result.placement,
                    previous.settings,
                );
                None
            }
            completed => Some(completed),
        }
    }

    fn remove_confirmed_tracks_from_view(&mut self, removal: &history_db::RecordingTrackRemoval) {
        let removed_recordings = {
            let mut shared = self.shared.borrow_mut();
            let state = &mut *shared;
            let keys = removal.loaded_keys(state.loaded_files.view());
            if keys.is_empty() {
                return;
            }
            let removed = modals::removed_recording_keys(&keys, state.loaded_files.view());
            modals::remove_items_from_view(&keys, &mut state.loaded_files, &mut state.tree);
            state.plot_state.rebuild_all(&state.loaded_files);
            removed
        };
        self.unload_logs_and_forget_attachments_of(&removed_recordings);
        self.on_track_indices_changed();
    }

    fn apply_mutation_outcome(&mut self, op: &history_db::DbOp, result: &Result<(), DbError>) {
        if let history_db::DbOp::TracksDeleted {
            view_removal: Some(removal),
            ..
        }
        | history_db::DbOp::TracksShelved {
            view_removal: Some(removal),
            ..
        } = op
        {
            self.remove_confirmed_tracks_from_view(removal);
        }
        let snapshot_updates = match op {
            history_db::DbOp::TracksDeleted {
                snapshot_updates, ..
            } => snapshot_updates.as_slice(),
            history_db::DbOp::IdentityRenamed {
                outcome: Some(outcome),
                ..
            } => outcome.snapshot_updates(),
            _ => &[],
        };
        if !snapshot_updates.is_empty() {
            let layouts_to_read = {
                let SharedAppState {
                    loaded_files, tree, ..
                } = &mut *self.shared.borrow_mut();
                let adopted = loaded_files.apply_track_snapshot_updates(snapshot_updates);
                tree.sync_after_track_snapshot_updates(
                    loaded_files.view(),
                    snapshot_updates,
                    &adopted,
                )
            };
            for layout in &layouts_to_read {
                self.read_recording_ui_state_if_unobserved(Some(layout));
            }
            self.store_the_hidden_tracks_the_user_changed();
        }
        match result {
            Ok(()) => {
                let mut loading_the_recording_again = false;
                match op {
                    history_db::DbOp::TracksUnshelved { db_ref, .. } => {
                        loading_the_recording_again =
                            self.reload_the_recording_if_it_is_loaded(db_ref);
                    }
                    history_db::DbOp::TracksShelved { .. }
                    | history_db::DbOp::TracksDeleted { .. }
                    | history_db::DbOp::RecordingsDeleted { .. }
                    | history_db::DbOp::IdentityRenamed { .. } => {}
                }
                self.toasts
                    .info(mutation_toast(op, loading_the_recording_again));
            }
            Err(e) => {
                log::error!("History update failed: {e}");
                self.toasts.error(format!("History update failed: {e}"));
            }
        }
    }

    /// Request the recording `db_ref` from the history worker again, where the
    /// view already holds that recording. The app then puts the loaded
    /// recording over that recording's entry in the view, with the unshelved
    /// tracks in it.
    ///
    /// Returns whether the request went out.
    fn reload_the_recording_if_it_is_loaded(&self, db_ref: &gt_store::DatabaseRef) -> bool {
        let shared = self.shared.borrow();
        let loaded = shared
            .loaded_files
            .view()
            .stored_recording_index(db_ref)
            .is_some();
        drop(shared);
        if !loaded {
            return false;
        }
        self.history.open_over_the_loaded_entry(db_ref.clone());
        true
    }

    fn open_the_recording_read_from_history(
        &mut self,
        origin: RecordingOperationOrigin,
        db_ref: DatabaseRef,
        placement: LoadedRecordingPlacement,
        settings: Option<Box<StoredHistoryLoadSettings>>,
        result: Result<OpenedRecording, DbError>,
    ) {
        let retry = match &origin {
            RecordingOperationOrigin::Retry(attempt) => Some(*attempt),
            _ => None,
        };
        if retry.is_some_and(|attempt| !self.load_failures.retry_is_pending(attempt)) {
            return;
        }
        let operation = origin.session_history();
        if operation.is_some_and(|id| !self.session_history.is_pending(id)) {
            return;
        }
        match result {
            Ok(opened) => {
                if let Some(id) = operation
                    && !opened.track_snapshot.is_current_with_membership()
                {
                    self.invalidate_history_listing();
                    self.session_history.finish(id, OpenStop::Expired);
                    return;
                }
                self.begin_history_open(
                    placement,
                    opened,
                    origin,
                    settings.map(|settings| *settings),
                );
            }
            Err(e) => {
                log::error!("Failed to load recording from history: {e}");
                if let Some(id) = operation {
                    let state = match e {
                        DbError::StoredTrackLayoutChanged => {
                            self.invalidate_history_listing();
                            OpenStop::Expired
                        }
                        DbError::StoredTrackTableDamaged => {
                            OpenStop::Failed(ErrorCode::HistoryTrackLayoutUnavailable)
                        }
                        _ => OpenStop::Failed(ErrorCode::HistoryOpenFailed),
                    };
                    self.session_history.finish(id, state);
                    return;
                }
                let request = LoadReadRequest::OpenStoredRecording {
                    recording: db_ref.clone(),
                    placement,
                };
                let failure = LoadFailure::new(
                    LoadSource::StoredRecording(db_ref),
                    LoadFailureCause::OpenStoredRecording(Box::new(e)),
                )
                .with_original_read(Some(request));
                if let Some(attempt) = retry {
                    self.load_failures.finish_retry(attempt, Err(failure));
                } else {
                    self.load_failures.push(failure);
                }
            }
        }
    }

    /// Apply a result delivered by the history worker thread.
    pub(super) fn handle_history_response(&mut self, resp: history_db::Response) {
        use history_db::Response;
        if resp.invalidates_recording_list() {
            self.invalidate_history_listing();
        }
        match resp {
            Response::Listed { request, entries } => {
                self.apply_history_listing_response(request, entries);
            }
            Response::Opened {
                origin,
                db_ref,
                placement,
                settings,
                result,
            } => self
                .open_the_recording_read_from_history(origin, db_ref, placement, settings, result),
            Response::Mutated { op, result } => self.apply_mutation_outcome(&op, &result),
            Response::PrunePreview(Ok(refs)) => self.history_window.set_prune_preview(refs),
            Response::PrunePreview(Err(e)) => log::error!("Prune preview failed: {e}"),
            Response::AutoPruned(Ok(AutoPruneOutcome::NotNeeded)) => {}
            Response::AutoPruned(Ok(AutoPruneOutcome::DeleteAttempt(Ok(n)))) => {
                let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                self.toasts
                    .info(format!("Auto-pruned {n} {rec_label}"))
                    .duration(Some(std::time::Duration::from_secs(4)));
            }
            Response::AutoPruned(Ok(AutoPruneOutcome::NeedsConfirmation(candidates))) => {
                self.pending_auto_prune = Some(candidates);
            }
            Response::AutoPruned(Ok(AutoPruneOutcome::DeleteAttempt(Err(e))) | Err(e)) => {
                log::error!("Auto-prune failed: {e}");
            }
            // A failed cache write costs only the persisted copy - the
            // session stores keep working - so this logs the failure.
            Response::SnapRunsStored(result) => {
                if let Err(e) = result {
                    log::warn!("Storing snap runs failed: {e}");
                }
            }
            // A failed write costs only the stored copy: the session shows the
            // tracks the user hid whatever the database took.
            Response::RecordingUiStateStored(result) => {
                if let Err(e) = result {
                    log::warn!("Storing the display settings of a recording failed: {e}");
                }
            }
            Response::RecordingUiStateLoaded(Ok((observation, ui_state))) => {
                self.apply_the_ui_state_stored_with_a_recording(observation, Ok(ui_state));
                self.store_the_hidden_tracks_the_user_changed();
            }
            Response::RecordingUiStateLoaded(Err(error)) => {
                log::warn!("Reading the display settings of a recording failed: {error}");
            }
            Response::SessionRecordingRestored(restoration) => {
                if let Some(restoration) = restoration {
                    self.apply_session_restoration(*restoration);
                }
            }
            Response::StoredTrackTableLoaded { db_ref, tracks } => {
                self.history_window.set_shelf_track_table(&db_ref, tracks);
            }
            Response::SnapRunsLoaded { db_ref, blob } => match blob {
                Ok(Some(bytes)) => self.restore_snap_runs(&db_ref, &bytes),
                Ok(None) => {}
                Err(e) => log::warn!("Loading stored snap runs failed: {e}"),
            },
            Response::LogSaved(completion) => {
                self.apply_initial_log_save_outcome(completion);
            }
            Response::AttachedLogsLoaded {
                db_ref,
                attachments,
            } => self.restore_attached_logs(
                &db_ref,
                attachments,
                AttachedLogRequester::RecordingLoad,
            ),
            Response::AttachedLogSettingsStored(completion) => {
                self.apply_attached_log_settings_outcome(completion);
            }
            Response::AttachedLogLoaded {
                attachment,
                name,
                retry,
                log,
            } => {
                if let Some(attempt) = retry {
                    self.finish_attachment_retry_read(attempt, attachment, name, log);
                } else {
                    self.load_the_attachment_the_user_opened(attachment, &name, log);
                }
            }
            Response::LogDetached {
                attachment,
                log,
                name,
                result,
            } => {
                self.apply_log_detach_outcome(&attachment, log, &name, result);
            }
            Response::RecordingsFromDiskScreened(screened) => {
                self.load_screened_recordings(screened);
            }
            Response::DuplicateAttachmentFound {
                log,
                recording,
                existing,
            } => self.set_duplicate_log_attachment(log, &recording, existing),
            Response::WriteRejected { label, rejection } => {
                log::debug!("Did not run {label:?}: {rejection}");
            }
        }
    }

    fn apply_session_restoration(&mut self, restoration: SessionRestoration) {
        let SessionRestoration {
            file,
            observation,
            snap,
            attachments,
        } = restoration;
        let shared = self.shared.borrow();
        let files = shared.loaded_files.view();
        let current = files.entry_for_id(file).is_some_and(|entry| {
            entry.history().track_snapshot().is_some_and(|snapshot| {
                snapshot.same_observation(&observation) && observation.is_current_with_membership()
            })
        });
        drop(shared);
        if current {
            let recording = observation.recording();
            match snap {
                Ok(Some(bytes)) => self.restore_snap_runs(recording, &bytes),
                Ok(None) => {}
                Err(error) => log::warn!("Loading stored snap runs failed: {error}"),
            }
            if self.session_attachment_target_is_unambiguous(file, recording) {
                self.restore_attached_logs(
                    recording,
                    attachments,
                    AttachedLogRequester::SessionHistoryRecording(file),
                );
            }
        }
    }

    pub(super) fn session_attachment_target_is_unambiguous(
        &self,
        file: LoadedFileId,
        recording: &DatabaseRef,
    ) -> bool {
        let shared = self.shared.borrow();
        let files = shared.loaded_files.view();
        let mut copies = files
            .entries()
            .filter(|entry| entry.history().db_ref() == Some(recording));
        copies.next().is_some_and(|entry| entry.id() == file) && copies.next().is_none()
    }

    pub(super) fn invalidate_history_listing(&mut self) {
        self.history_listing.invalidate();
        self.history_window.invalidate_shelf_track_table();
    }

    fn apply_history_listing_response(
        &mut self,
        request: ListRequestId,
        entries: Result<HistoryEntries, DbError>,
    ) {
        if let Some(result) = self.history_listing.accept_response(request, entries) {
            self.history_window.apply_listing_result(result);
        }
    }

    pub(super) fn show_history_window(&mut self, ui: &egui::Ui) {
        if !self.history_window.open {
            return;
        }
        self.history_listing.request_if_missing(&self.history);
        let storage_before_edit = self.storage_settings;
        let loaded_metas: Vec<RecordingMeta> = {
            let s = self.shared.borrow();
            s.loaded_files.view().recording_metas()
        };
        self.history_window.show(
            ui.ctx(),
            HistoryWindowFrame {
                now: Instant::now(),
                worker: &self.history,
                listing: self.history_listing.view(),
                loaded_metas: &loaded_metas,
                storage: &mut self.storage_settings,
                databases_opening: self.storage_open.databases_pending().is_some(),
                write_access: self.pending_writes.write_access(),
            },
        );
        self.sync_db_path_if_auto_store_changed(storage_before_edit);
    }

    pub(super) fn show_history_failure_prompt(&mut self, ui: &egui::Ui) {
        // Prompts for a recordings database that would not open. Each
        // failure gets its own, because the remedies differ sharply: waiting,
        // a destructive lock clear, or a recreate.
        let Some(failure) = self.history_failure.clone() else {
            return;
        };
        let path = failure.path().to_owned();
        let taken_over = self.instance_taken_over_from;
        let mut keep_backup = self.keep_db_backup;
        let choice = match &failure {
            storage::HistoryFailure::Busy(_) => modals::anchored_confirmation_dialog(
                ui.ctx(),
                AnchoredDialogKind::HistoryDatabaseInUse,
                HISTORY_DATABASE_IN_USE_TITLE,
                HistoryFailureChoice::Dismiss,
                |ui, _regions| match taken_over {
                    Some(instance) => {
                        ui.label(format!(
                            "{} still has the recording history database open.",
                            instance.sentence_subject()
                        ));
                        ui.label("Recordings load here, but are not stored until it exits.");
                    }
                    None => {
                        ui.label(
                            "Another process has the recording history database open, most \
                             likely a second GeoTrace instance.",
                        );
                        ui.label(
                            "Close it and try again. Recordings still load in the meantime, they \
                             are just not stored.",
                        );
                    }
                },
                |ui| {
                    let mut choice = None;
                    if ui.button("Try again").clicked() {
                        choice = Some(HistoryFailureChoice::Reopen);
                    }
                    if ui.button("Continue without storing").clicked() {
                        choice = Some(HistoryFailureChoice::Dismiss);
                    }
                    choice
                },
            ),
            storage::HistoryFailure::Locked(_) => modals::anchored_confirmation_dialog(
                ui.ctx(),
                AnchoredDialogKind::HistoryDatabaseLocked,
                HISTORY_DATABASE_LOCKED_TITLE,
                HistoryFailureChoice::Dismiss,
                |ui, _regions| {
                    ui.label("The recording history is marked as open for write.");
                    ui.label(
                        "This usually means GeoTrace did not shut down cleanly, but another \
                         program may still have the database open.",
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Only continue if no other program is using the database - otherwise \
                             it could be corrupted.",
                        )
                        .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                    );
                },
                |ui| {
                    let mut choice = None;
                    let clear = ui.add_enabled(
                        taken_over.is_none(),
                        Button::new(
                            RichText::new(CLEAR_LOCK_BUTTON_LABEL)
                                .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                        ),
                    );
                    if clear.clicked() {
                        choice = Some(HistoryFailureChoice::ClearTheWriteLock);
                    }
                    if let Some(instance) = taken_over {
                        clear.on_disabled_hover_text(format!(
                            "{} still has the recording history database open: clearing the \
                             write lock while it writes can corrupt the database.",
                            instance.sentence_subject()
                        ));
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(HistoryFailureChoice::Dismiss);
                    }
                    choice
                },
            ),
            storage::HistoryFailure::Unreadable(_) => modals::anchored_confirmation_dialog(
                ui.ctx(),
                AnchoredDialogKind::HistoryDatabaseCorrupted,
                HISTORY_DATABASE_CORRUPTED_TITLE,
                HistoryFailureChoice::Dismiss,
                |ui, _regions| {
                    ui.label("The recording history database could not be opened.");
                    ui.label("You can try to recover it manually, or recreate a fresh one.");
                    ui.add_space(4.0);
                    ui.checkbox(&mut keep_backup, "Keep a backup of the original database");
                },
                |ui| {
                    let mut choice = None;
                    if ui
                        .button(
                            RichText::new("Recreate database")
                                .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                        )
                        .clicked()
                    {
                        choice = Some(HistoryFailureChoice::Recreate);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(HistoryFailureChoice::Dismiss);
                    }
                    choice
                },
            ),
        };
        self.keep_db_backup = keep_backup;

        let Some(choice) = choice else {
            return;
        };
        match choice {
            HistoryFailureChoice::Reopen => self.reopen_history_database(&path, ui.ctx()),
            HistoryFailureChoice::ClearTheWriteLock => {
                self.recover_history_database(&path, ui.ctx());
            }
            HistoryFailureChoice::Recreate => {
                self.recreate_history_database(&path, keep_backup, ui.ctx());
            }
            HistoryFailureChoice::Dismiss => {}
        }
        self.history_failure = None;
    }

    pub(super) fn show_resegment_prompt(&mut self, ui: &egui::Ui) {
        // Re-segment prompt: a recording opened from history was stored with a
        // different track setting than the current one.
        let Some(prompt) = self
            .pending_resegment
            .take()
            .or_else(|| self.queued_resegments.pop_front())
        else {
            return;
        };
        if let RecordingOperationOrigin::Retry(attempt) = &prompt.origin
            && !self.load_failures.retry_is_pending(*attempt)
        {
            return;
        }
        let operation = prompt.origin.session_history();
        if let Some(id) = operation
            && !(self.session_history.is_pending(id)
                && prompt.track_snapshot.is_current_with_membership())
        {
            if self.session_history.is_pending(id) {
                self.session_history.finish(id, OpenStop::Expired);
            }
            return;
        }
        let current = loader::stored_segmentation_from_config(&self.processing_config);
        let can_recalculate = operation.is_none()
            || (self.loader.db_path.is_some()
                && self.pending_writes.write_access().allows_writing());
        let stored = prompt.stored;
        let recalculation_warning = MarksDroppedByRecalculating::of_the_prompted_recording(
            &prompt,
            &self.shared.borrow().tree,
            operation.and_then(|id| self.session_history.saved_hidden_track_count(id)),
        )
        .warning_line();
        let fmt_gap = |us: i64| format!("{}s", us / 1_000_000);
        let fmt_split_rule = |rule: StoredTrackSplitRule| match rule {
            StoredTrackSplitRule::ForwardGapOnly => "forward only".to_owned(),
            StoredTrackSplitRule::StepInEitherDirection => "either direction".to_owned(),
            StoredTrackSplitRule::Unrecognized(value) => {
                format!("unknown rule {value}")
            }
        };
        let fmt_placement_rule = |rule: StoredFixPlacementRule| match rule {
            StoredFixPlacementRule::SynthesizedOrHeadinglessNoFix => {
                "synthesized or headingless with no fix".to_owned()
            }
            StoredFixPlacementRule::MissingHeading => "no heading".to_owned(),
            StoredFixPlacementRule::MissingHeadingAndNothingInFix => {
                "no heading, nothing in fix".to_owned()
            }
            StoredFixPlacementRule::Unrecognized(value) => {
                format!("unknown rule {value}")
            }
        };
        let stored_tracks_are_reproducible = loader::stored_tracks_are_reproducible(&stored);
        let intro = format!(
            "'{}' was stored with different track settings than the current ones.",
            prompt.filename
        );
        let choice = modals::anchored_confirmation_dialog(
            ui.ctx(),
            AnchoredDialogKind::TrackSettingsDiffer,
            TRACK_SETTINGS_DIFFER_TITLE,
            ResegmentChoice::Cancel,
            |ui, regions| {
                regions.freeze_at_open_ui(
                    ui,
                    RESEGMENT_INTRO_REGION,
                    HeldBodyLines::measured_content().and_at_most(RESEGMENT_INTRO_MOST_LINES),
                    |ui| {
                        ui.add(Label::new(intro).wrap());
                    },
                );
                ui.add_space(4.0);
                Grid::new("resegment_settings")
                    .num_columns(3)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("");
                        ui.strong("Stored");
                        ui.strong("Current");
                        ui.end_row();
                        ui.label("Split gap");
                        ui.label(fmt_gap(stored.track_split_gap_us));
                        ui.label(fmt_gap(current.track_split_gap_us));
                        ui.end_row();
                        ui.label("Split rule");
                        ui.label(fmt_split_rule(stored.track_split_rule));
                        ui.label(fmt_split_rule(current.track_split_rule));
                        ui.end_row();
                        ui.label("Position placement");
                        ui.label(fmt_placement_rule(stored.fix_placement_rule));
                        ui.label(fmt_placement_rule(current.fix_placement_rule));
                        ui.end_row();
                    });
                if let Some(warning) = &recalculation_warning {
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(
                            RichText::new(warning)
                                .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                        )
                        .wrap(),
                    );
                }
            },
            |ui| {
                let mut choice = None;
                if ui
                    .add_enabled(can_recalculate, Button::new("Recalculate with current settings"))
                    .on_hover_text(
                        "Re-split the recording with the current settings, replacing the stored \
                         tracks",
                    )
                    .on_disabled_hover_text("Recalculating a session recording requires writable History with storage enabled")
                    .clicked()
                {
                    choice = Some(ResegmentChoice::RecalculateWithCurrentSettings);
                }
                if ui
                    .add_enabled(
                        stored_tracks_are_reproducible,
                        Button::new("Use stored tracks"),
                    )
                    .on_hover_text("Open the tracks as stored, with their previous settings")
                    .on_disabled_hover_text(
                        "This version does not implement a rule the recording was stored with",
                    )
                    .clicked()
                {
                    choice = Some(ResegmentChoice::UseStoredTracks);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(ResegmentChoice::Cancel);
                }
                choice
            },
        );

        if let (Some(id), Some(choice)) = (operation, choice) {
            self.session_history.apply_settings_choice(id, choice);
        }
        match choice {
            Some(ResegmentChoice::RecalculateWithCurrentSettings) => {
                self.loader.spawn_gtd_bytes(
                    prompt.bytes,
                    prompt.filename,
                    self.processing_config,
                    prompt.mode,
                    Some(loader::HistoryOpen::Recalculate {
                        authority: if operation.is_some() {
                            RecalculationAuthority::SelectedLayout
                        } else {
                            RecalculationAuthority::StoredPayload
                        },
                        snapshot: prompt.track_snapshot,
                        applied_current_marker_settings: prompt.marker_settings_changed,
                        placement: prompt.placement,
                    }),
                    prompt.origin,
                );
            }
            Some(ResegmentChoice::UseStoredTracks) => {
                let config =
                    loader::config_from_stored_segmentation(&stored, self.processing_config);
                self.loader.spawn_gtd_bytes(
                    prompt.bytes,
                    prompt.filename,
                    config,
                    prompt.mode,
                    Some(loader::HistoryOpen::ApplyShelved {
                        track_table: loader::OpenTrackTable::Stored(prompt.track_snapshot),
                        applied_current_marker_settings: prompt.marker_settings_changed,
                        placement: prompt.placement,
                    }),
                    prompt.origin,
                );
            }
            Some(ResegmentChoice::Cancel) => {
                if let RecordingOperationOrigin::Retry(attempt) = prompt.origin {
                    self.load_failures.cancel_retry(attempt);
                }
            }
            // No choice yet: keep the prompt open for the next frame.
            None => self.pending_resegment = Some(prompt),
        }
    }

    pub(super) fn show_auto_prune_prompt(&mut self, ui: &egui::Ui) {
        let Some(refs) = &self.pending_auto_prune else {
            return;
        };
        let limit = gt_fmt::format_bytes(self.storage_settings.auto_prune_max_bytes);
        let n = refs.len();
        let choice = modals::anchored_confirmation_dialog(
            ui.ctx(),
            AnchoredDialogKind::AutoPrune,
            AUTO_PRUNE_TITLE,
            AutoPruneChoice::Cancel,
            |ui, regions| {
                let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                ui.add(
                    Label::new(format!(
                        "{n} {rec_label} will be deleted to keep storage under {limit}"
                    ))
                    .wrap(),
                );
                ui.add_space(4.0);
                regions.freeze_at_open_ui(
                    ui,
                    AUTO_PRUNE_RECORDINGS_REGION,
                    HeldBodyLines::measured_content().and_at_most(AUTO_PRUNE_RECORDINGS_MOST_LINES),
                    |ui| {
                        for r in refs {
                            let label = format!("{}/{}", r.identity, r.group_name);
                            ui.add(Label::new(label.as_str()).truncate());
                        }
                    },
                );
            },
            |ui| {
                let mut choice = None;
                if ui
                    .button(
                        RichText::new("Delete these recordings")
                            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                    )
                    .on_hover_text(history::DESTRUCTIVE_DELETE_HOVER)
                    .clicked()
                {
                    choice = Some(AutoPruneChoice::Delete);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(AutoPruneChoice::Cancel);
                }
                choice
            },
        );

        match choice {
            Some(AutoPruneChoice::Delete) => {
                let candidates = self.pending_auto_prune.take().unwrap_or_default();
                self.history
                    .delete_recordings(candidates, history_db::DeleteReason::AutoPrune);
            }
            Some(AutoPruneChoice::Cancel) => self.pending_auto_prune = None,
            None => {}
        }
    }
}

/// Backup path for a corrupted database: appends `.corrupt.bak` to the file name
/// (e.g. `geotrace.h5` -> `geotrace.h5.corrupt.bak`).
fn corrupt_backup_path(path: &std::path::Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(".corrupt.bak");
    path.with_file_name(name)
}

/// Build the completion toast for a finished history mutation.
///
/// `loading_the_recording_again` is true only where an unshelve requested a
/// loaded recording from the history worker again. Only the unshelve toast
/// reads it.
fn mutation_toast(op: &history_db::DbOp, loading_the_recording_again: bool) -> String {
    use history_db::{DbOp, DeleteReason};
    match op {
        DbOp::TracksShelved { count, .. } => {
            let tracks = gt_fmt::pluralize(*count, "track", "tracks");
            format!("Shelved {count} {tracks} in history")
        }
        DbOp::TracksUnshelved { count, .. } => {
            let tracks = gt_fmt::pluralize(*count, "track", "tracks");
            if loading_the_recording_again {
                format!("Unshelved {count} {tracks}, loading the recording again")
            } else {
                format!("Unshelved {count} {tracks} in history")
            }
        }
        DbOp::TracksDeleted { count, .. } => {
            let tracks = gt_fmt::pluralize(*count, "track", "tracks");
            format!("Permanently deleted {count} {tracks} from history")
        }
        DbOp::RecordingsDeleted { count, reason } => {
            let rec = gt_fmt::pluralize(*count, "recording", "recordings");
            match reason {
                DeleteReason::Manual => format!("Deleted {count} {rec} from history"),
                DeleteReason::Prune => format!("Pruned {count} {rec} from history"),
                DeleteReason::AutoPrune => format!("Auto-pruned {count} {rec}"),
            }
        }
        DbOp::IdentityRenamed { new, .. } => {
            let (name, _) = gt_loaded_files::display_identity(new);
            format!("Renamed identity to \"{name}\"")
        }
    }
}

/// The region showing the recording the re-segment prompt is about.
const RESEGMENT_INTRO_REGION: &str = "resegment_intro";

/// Lines the [`RESEGMENT_INTRO_REGION`] holds at most, however long the
/// recording's name: the rest of the name scrolls inside it. Three is the one
/// line the sentence takes on its own plus two for the name.
const RESEGMENT_INTRO_MOST_LINES: u8 = 3;

/// The region listing the recordings the prune deletes. A second set of
/// candidates that arrives while the prompt is open replaces that list.
const AUTO_PRUNE_RECORDINGS_REGION: &str = "auto_prune_recordings";

/// Lines the [`AUTO_PRUNE_RECORDINGS_REGION`] holds at most, however many
/// recordings the prune deletes: the rest of the rows scroll inside it. Twelve
/// is the room ten rows take, each a line of body text with the spacing under
/// it.
pub(in crate::app) const AUTO_PRUNE_RECORDINGS_MOST_LINES: u8 = 12;

const OPENING_THE_DATABASE: &str = "Opening the recording history database";

const CLEARING_THE_WRITE_LOCK: &str = "Clearing the recording history database's write lock";

const RECREATING_THE_DATABASE: &str = "Recreating the recording history database";

pub(in crate::app) const CLEAR_LOCK_BUTTON_LABEL: &str = "Clear lock and open";

pub(in crate::app) const HISTORY_DATABASE_IN_USE_TITLE: &str = "History database in use";

pub(in crate::app) const HISTORY_DATABASE_LOCKED_TITLE: &str = "History database locked";

pub(in crate::app) const HISTORY_DATABASE_CORRUPTED_TITLE: &str = "History database is corrupted";

pub(in crate::app) const TRACK_SETTINGS_DIFFER_TITLE: &str = "Track settings differ";

pub(in crate::app) const AUTO_PRUNE_TITLE: &str = "Auto-prune";
