use std::path::PathBuf;

use egui::{Button, Grid, Label, RichText, ScrollArea, Window};
use gt_pending_writes::{PendingWriteGuard, WriteKind};
use gt_store::DbError;

use super::{App, ResegmentPrompt, auto_prune, history_db, loader, storage};

const OPENING_THE_DATABASE: &str = "Opening the recording history database";

const CLEARING_THE_WRITE_LOCK: &str = "Clearing the recording history database's write lock";

const RECREATING_THE_DATABASE: &str = "Recreating the recording history database";

pub(in crate::app) const CLEAR_LOCK_BUTTON_LABEL: &str = "Clear lock and open";

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
        let previous = std::mem::replace(&mut self.history, worker);
        previous.shutdown();
        self.sync_db_path();
        self.history_window.invalidate();
    }

    /// Put a freshly opened database behind the history worker.
    fn adopt_history_database(
        &mut self,
        db: gt_store::Recordings,
        ctx: &egui::Context,
        toast: &str,
    ) {
        self.install_history_worker(history_db::HistoryWorker::spawn(
            db,
            ctx.clone(),
            self.pending_writes.clone(),
        ));
        self.toasts.info(toast);
    }

    /// Registers a write to the recordings database, logging a refusal at
    /// debug level.
    fn try_begin_recording_history_write(&self, label: &'static str) -> Option<PendingWriteGuard> {
        match self
            .pending_writes
            .try_begin(label, WriteKind::RecordingDatabase)
        {
            Ok(write) => Some(write),
            Err(refusal) => {
                log::debug!("Did not run {label:?}: {refusal}");
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
    /// re-applies the hidden ones, and regenerates markers with current settings.
    /// When the stored track-splitting setting differs from the current one it
    /// raises a prompt instead (recalculate vs. use the stored tracks).
    fn begin_history_open(
        &mut self,
        db_ref: gt_store::DatabaseRef,
        stored: gt_store::StoredRecording,
    ) {
        // Reuse the original filename: the identity is the filename (with an
        // "auto:" prefix for auto-derived ones).
        let filename = db_ref
            .identity
            .strip_prefix("auto:")
            .unwrap_or(&db_ref.identity)
            .to_owned();

        let hidden_positions: Vec<usize> = stored
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.hidden)
            .map(|(i, _)| i)
            .collect();

        match stored.segmentation {
            // Stored track splitting differs from the current setting: let the
            // user choose before changing track ranges that hidden-track state
            // may refer to.
            Some(stored_settings)
                if !loader::track_split_matches_config(
                    &stored_settings,
                    &self.processing_config,
                ) && !stored.tracks.is_empty() =>
            {
                let marker_settings_changed = !loader::marker_settings_match_config(
                    &stored_settings,
                    &self.processing_config,
                );
                self.pending_resegment = Some(ResegmentPrompt {
                    db_ref,
                    filename,
                    bytes: stored.bytes.into(),
                    stored: stored_settings,
                    hidden_positions,
                    marker_settings_changed,
                });
            }
            // Track splitting matches: reproduce the stored tracks, re-apply
            // hidden ones, and rebuild generated markers from current settings.
            Some(stored_settings) => {
                let marker_settings_changed = !loader::marker_settings_match_config(
                    &stored_settings,
                    &self.processing_config,
                );
                let config = loader::config_from_stored_segmentation(
                    &stored_settings,
                    self.processing_config,
                );
                self.loader.spawn_gtd_from_history(
                    stored.bytes.into(),
                    filename,
                    config,
                    loader::HistoryOpen::ApplyHidden {
                        db_ref,
                        positions: hidden_positions,
                        applied_current_marker_settings: marker_settings_changed,
                    },
                );
            }
            // Older recording with no stored settings: load with current settings.
            None => {
                self.loader.spawn_gtd_from_history(
                    stored.bytes.into(),
                    filename,
                    self.processing_config,
                    loader::HistoryOpen::ApplyHidden {
                        db_ref,
                        positions: hidden_positions,
                        applied_current_marker_settings: false,
                    },
                );
            }
        }
    }

    /// Refresh the history list and toast a finished mutation, or report the
    /// failure it came back with.
    fn apply_mutation_outcome(&mut self, op: &history_db::DbOp, result: &Result<(), DbError>) {
        match result {
            Ok(()) => {
                // Keep loaded recordings pointing at the renamed identity so
                // later history operations on them still resolve.
                if let history_db::DbOp::IdentityRenamed { old, new } = op {
                    self.shared
                        .borrow_mut()
                        .loaded_files
                        .rename_identity(old, new);
                }
                self.history_window.invalidate();
                self.toasts.info(mutation_toast(op));
            }
            Err(e) => {
                log::error!("History update failed: {e}");
                self.toasts.error(format!("History update failed: {e}"));
            }
        }
    }

    /// Apply a result delivered by the history worker thread.
    pub(super) fn handle_history_response(&mut self, resp: history_db::Response) {
        use history_db::Response;
        match resp {
            Response::Listed(Ok(entries)) => self.history_window.set_entries(entries),
            Response::Listed(Err(e)) => {
                self.history_window
                    .set_error(format!("Failed to load history: {e}"));
            }
            Response::Opened { db_ref, result } => match result {
                Ok(stored) => self.begin_history_open(db_ref, stored),
                Err(e) => {
                    log::error!("Failed to load recording from history: {e}");
                    self.toasts.error(format!("Could not open recording: {e}"));
                }
            },
            Response::Mutated { op, result } => self.apply_mutation_outcome(&op, &result),
            Response::PrunePreview(Ok(refs)) => self.history_window.set_prune_preview(refs),
            Response::PrunePreview(Err(e)) => log::error!("Prune preview failed: {e}"),
            Response::AutoPruned(Ok(auto_prune::AutoPruneOutcome::NotNeeded)) => {}
            Response::AutoPruned(Ok(auto_prune::AutoPruneOutcome::PrunedSilently(n))) => {
                self.history_window.invalidate();
                let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                self.toasts
                    .info(format!("Auto-pruned {n} {rec_label}"))
                    .duration(Some(std::time::Duration::from_secs(4)));
            }
            Response::AutoPruned(Ok(auto_prune::AutoPruneOutcome::NeedsConfirmation(
                candidates,
            ))) => {
                self.pending_auto_prune = Some(candidates);
            }
            Response::AutoPruned(Err(e)) => log::error!("Auto-prune failed: {e}"),
            // A failed cache write costs only the persisted copy - the
            // session stores keep working - so this logs instead of toasting.
            Response::SnapRunsStored(result) => {
                if let Err(e) = result {
                    log::warn!("Storing snap runs failed: {e}");
                }
            }
            Response::SnapRunsLoaded { db_ref, blob } => match blob {
                Ok(Some(bytes)) => self.restore_snap_runs(&db_ref, &bytes),
                Ok(None) => {}
                Err(e) => log::warn!("Loading stored snap runs failed: {e}"),
            },
            Response::LogAttached { log, name, result } => {
                self.apply_log_attach_outcome(log, &name, result);
            }
            Response::AttachedLogsLoaded {
                db_ref,
                attachments,
            } => self.restore_attached_logs(&db_ref, attachments),
            // A failed write costs only the stored copy: the loaded log keeps
            // the stack the user is looking at.
            Response::AttachedLogFiltersStored(result) => {
                if let Err(e) = result {
                    log::warn!("Storing an attached log's filters failed: {e}");
                }
            }
            Response::LogDetached { log, name, result } => {
                self.apply_log_detach_outcome(log, &name, result);
            }
            Response::DuplicateAttachmentFound {
                log,
                recording,
                existing,
            } => self.set_duplicate_log_attachment(log, &recording, existing),
            Response::WriteRefused { label, refusal } => {
                log::debug!("Did not run {label:?}: {refusal}");
            }
        }
    }

    pub(super) fn show_history_window(&mut self, ui: &egui::Ui) {
        let storage_before_edit = self.storage_settings;
        let loaded_metas: Vec<gt_store::RecordingMeta> = {
            let s = self.shared.borrow();
            s.loaded_files.view().recording_metas()
        };
        self.history_window.show(
            ui.ctx(),
            &self.history,
            &loaded_metas,
            &mut self.storage_settings,
            self.storage_open.databases_pending().is_some(),
            self.pending_writes.write_access(),
        );
        self.sync_db_path_if_auto_store_changed(storage_before_edit);
    }

    pub(super) fn show_history_failure_prompt(&mut self, ui: &egui::Ui) {
        // Prompts for a recordings database that would not open. Each
        // failure gets its own, because the remedies differ sharply: waiting,
        // a destructive lock clear, or a recreate.
        if let Some(failure) = self.history_failure.clone() {
            let path = failure.path().to_owned();
            let taken_over = self.instance_taken_over_from;
            let mut resolve = None;
            let dismissed = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            match &failure {
                storage::HistoryFailure::Busy(_) => {
                    Window::new("History database in use")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ui.ctx(), |ui| {
                            match taken_over {
                                Some(instance) => {
                                    ui.label(format!(
                                        "{} still has the recording history database open.",
                                        instance.sentence_subject()
                                    ));
                                    ui.label(
                                        "Recordings load here, but are not stored until it exits.",
                                    );
                                }
                                None => {
                                    ui.label(
                                        "Another process has the recording history database open, most likely a second GeoTrace instance.",
                                    );
                                    ui.label(
                                        "Close it and try again. Recordings still load in the meantime, they are just not stored.",
                                    );
                                }
                            }
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui.button("Try again").clicked() {
                                    resolve = Some(true);
                                }
                                if ui.button("Continue without storing").clicked() {
                                    resolve = Some(false);
                                }
                            });
                        });
                    if resolve == Some(true) {
                        self.reopen_history_database(&path, ui.ctx());
                    }
                }
                storage::HistoryFailure::Locked(_) => {
                    Window::new("History database locked")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ui.ctx(), |ui| {
                            ui.label("The recording history is marked as open for write.");
                            ui.label(
                                "This usually means GeoTrace did not shut down cleanly, but another program may still have the database open.",
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "Only continue if no other program is using the database - otherwise it could be corrupted.",
                                )
                                .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                let clear = ui.add_enabled(
                                    taken_over.is_none(),
                                    Button::new(RichText::new(CLEAR_LOCK_BUTTON_LABEL).color(
                                        gt_ui_theme::warning_amber(ui.visuals().dark_mode),
                                    )),
                                );
                                if clear.clicked() {
                                    resolve = Some(true);
                                }
                                if let Some(instance) = taken_over {
                                    clear.on_disabled_hover_text(format!(
                                        "{} still has the recording history database open: clearing the write lock while it writes can corrupt the database.",
                                        instance.sentence_subject()
                                    ));
                                }
                                if ui.button("Cancel").clicked() {
                                    resolve = Some(false);
                                }
                            });
                        });
                    if resolve == Some(true) {
                        self.recover_history_database(&path, ui.ctx());
                    }
                }
                storage::HistoryFailure::Unreadable(_) => {
                    let mut keep_backup = self.keep_db_backup;
                    Window::new("History database is corrupted")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ui.ctx(), |ui| {
                            ui.label("The recording history database could not be opened.");
                            ui.label(
                                "You can try to recover it manually, or recreate a fresh one.",
                            );
                            ui.add_space(4.0);
                            ui.checkbox(&mut keep_backup, "Keep a backup of the original database");
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .button(
                                        RichText::new("Recreate database").color(
                                            gt_ui_theme::warning_amber(ui.visuals().dark_mode),
                                        ),
                                    )
                                    .clicked()
                                {
                                    resolve = Some(true);
                                }
                                if ui.button("Cancel").clicked() {
                                    resolve = Some(false);
                                }
                            });
                        });
                    self.keep_db_backup = keep_backup;
                    if resolve == Some(true) {
                        self.recreate_history_database(&path, keep_backup, ui.ctx());
                    }
                }
            }
            if resolve.is_some() || dismissed {
                self.history_failure = None;
            }
        }
    }

    pub(super) fn show_resegment_prompt(&mut self, ui: &egui::Ui) {
        // Re-segment prompt: a recording opened from history was stored with a
        // different track-splitting setting than the current one.
        if let Some(prompt) = self.pending_resegment.take() {
            let current = loader::stored_segmentation_from_config(&self.processing_config);
            let mut recalculate = false;
            let mut use_stored = false;
            let mut cancel = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let fmt_gap = |us: i64| format!("{} s", us / 1_000_000);
            Window::new("Track splitting differs")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    // Bound the width so a long recording name wraps.
                    ui.set_max_width(460.0);
                    ui.add(Label::new(format!(
                        "'{}' was stored with a different track-splitting setting than the current one.",
                        prompt.filename
                    )).wrap());
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
                            ui.label(fmt_gap(prompt.stored.track_split_gap_us));
                            ui.label(fmt_gap(current.track_split_gap_us));
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button("Use stored tracks")
                            .on_hover_text("Open the tracks as stored, with their previous settings")
                            .clicked()
                        {
                            use_stored = true;
                        }
                        if ui
                            .button("Recalculate with current settings")
                            .on_hover_text(
                                "Re-split the recording with the current settings, replacing the stored tracks",
                            )
                            .clicked()
                        {
                            recalculate = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if recalculate {
                self.loader.spawn_gtd_from_history(
                    prompt.bytes,
                    prompt.filename,
                    self.processing_config,
                    loader::HistoryOpen::Recalculate {
                        db_ref: prompt.db_ref,
                        applied_current_marker_settings: prompt.marker_settings_changed,
                    },
                );
                self.history_window.invalidate();
            } else if use_stored {
                let config =
                    loader::config_from_stored_segmentation(&prompt.stored, self.processing_config);
                self.loader.spawn_gtd_from_history(
                    prompt.bytes,
                    prompt.filename,
                    config,
                    loader::HistoryOpen::ApplyHidden {
                        db_ref: prompt.db_ref,
                        positions: prompt.hidden_positions,
                        applied_current_marker_settings: prompt.marker_settings_changed,
                    },
                );
            } else if !cancel {
                // No choice yet: keep the prompt open for the next frame.
                self.pending_resegment = Some(prompt);
            }
        }
    }

    pub(super) fn show_auto_prune_prompt(&mut self, ui: &egui::Ui) {
        // Auto-prune confirmation dialog.
        if let Some(refs) = &self.pending_auto_prune {
            let limit = gt_fmt::format_bytes(self.storage_settings.auto_prune_max_bytes);
            let n = refs.len();
            let mut do_prune = false;
            let mut cancel = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            Window::new("Auto-prune")
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    // Bound the width so a long recording identity truncates.
                    ui.set_max_width(460.0);
                    let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                    ui.label(format!(
                        "{n} {rec_label} will be deleted to keep storage under {limit}"
                    ));
                    ui.add_space(4.0);
                    ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        for r in refs {
                            let label = format!("{}/{}", r.identity, r.group_name);
                            ui.add(Label::new(label.as_str()).truncate());
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                RichText::new("Delete these recordings")
                                    .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                            )
                            .on_hover_text(
                                "This cannot be undone. The original source files are unaffected.",
                            )
                            .clicked()
                        {
                            do_prune = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if do_prune {
                let candidates = self.pending_auto_prune.take().unwrap_or_default();
                self.history
                    .delete_recordings(candidates, history_db::DeleteReason::AutoPrune);
            } else if cancel {
                self.pending_auto_prune = None;
            }
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
fn mutation_toast(op: &history_db::DbOp) -> String {
    use history_db::{DbOp, DeleteReason};
    match op {
        DbOp::TracksHidden { count } => {
            let tracks = gt_fmt::pluralize(*count, "track", "tracks");
            format!("Hid {count} {tracks} in history")
        }
        DbOp::TracksDeleted { count } => {
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
