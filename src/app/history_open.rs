use std::path::PathBuf;
use std::time::Instant;

use egui::{Button, Grid, Label, RichText};
use gt_pending_writes::{PendingWriteGuard, WriteKind};
use gt_store::{DbError, StoredFixPlacementRule, StoredTrackSplitRule};

use super::anchored_dialog::{AnchoredDialogKind, HeldBodyLines};
use super::{App, ResegmentPrompt, auto_prune, history, history_db, loader, modals, storage};

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

/// What the user chose in the prompt for a recordings database that would not
/// open. Each failure offers one of the three remedies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryFailureChoice {
    Reopen,
    ClearTheWriteLock,
    Recreate,
    /// Go on with the database left as it is: recordings load but are not
    /// stored.
    Dismiss,
}

/// What the user chose in the "Track settings differ" prompt.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ResegmentChoice {
    RecalculateWithCurrentSettings,
    UseStoredTracks,
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AutoPruneChoice {
    Delete,
    Cancel,
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

        let stored_tracks = stored.tracks;

        match stored.segmentation {
            // A stored track setting differs from the current one: let the user
            // choose before changing track ranges that the shelved state may
            // refer to, or the positions the fixes are drawn at.
            Some(stored_settings)
                if !loader::stored_tracks_match_config(
                    &stored_settings,
                    &self.processing_config,
                ) && !stored_tracks.is_empty() =>
            {
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
                self.pending_resegment = Some(ResegmentPrompt {
                    db_ref,
                    filename,
                    bytes: stored.bytes.into(),
                    stored: stored_settings,
                    stored_tracks,
                    marker_settings_changed,
                });
            }
            // Every stored track setting matches: reproduce the stored tracks,
            // leave the shelved ones out, and rebuild generated markers from
            // current settings.
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
                    loader::HistoryOpen::ApplyShelved {
                        db_ref,
                        stored_tracks,
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
                    loader::HistoryOpen::ApplyShelved {
                        db_ref,
                        stored_tracks,
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
            // session stores keep working - so this logs the failure.
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
            Response::AttachedLogLoaded {
                attachment,
                name,
                log,
            } => self.load_the_attachment_the_user_opened(attachment, &name, log),
            Response::LogDetached {
                attachment,
                log,
                name,
                result,
            } => {
                self.apply_log_detach_outcome(&attachment, log, &name, result);
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

    pub(super) fn show_history_window(&mut self, ui: &egui::Ui) {
        let storage_before_edit = self.storage_settings;
        let loaded_metas: Vec<gt_store::RecordingMeta> = {
            let s = self.shared.borrow();
            s.loaded_files.view().recording_metas()
        };
        self.history_window.show(
            ui.ctx(),
            history::HistoryWindowFrame {
                now: Instant::now(),
                worker: &self.history,
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
        let Some(prompt) = self.pending_resegment.take() else {
            return;
        };
        let current = loader::stored_segmentation_from_config(&self.processing_config);
        let stored = prompt.stored;
        let fmt_gap = |us: i64| format!("{}s", us / 1_000_000);
        let fmt_split_rule = |rule: StoredTrackSplitRule| match rule {
            StoredTrackSplitRule::ForwardGapOnly => "forward only".to_owned(),
            StoredTrackSplitRule::StepInEitherDirection => "either direction".to_owned(),
            StoredTrackSplitRule::Unrecognized(value) => {
                format!("unknown rule {value}")
            }
        };
        let fmt_placement_rule = |rule: StoredFixPlacementRule| match rule {
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
                regions.frozen_at_open(
                    ui,
                    RESEGMENT_INTRO_REGION,
                    HeldBodyLines::what_the_content_took().and_at_most(RESEGMENT_INTRO_MOST_LINES),
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
                        ui.label("Dead-reckoned fix");
                        ui.label(fmt_placement_rule(stored.fix_placement_rule));
                        ui.label(fmt_placement_rule(current.fix_placement_rule));
                        ui.end_row();
                    });
            },
            |ui| {
                let mut choice = None;
                if ui
                    .button("Recalculate with current settings")
                    .on_hover_text(
                        "Re-split the recording with the current settings, replacing the stored \
                         tracks",
                    )
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

        match choice {
            Some(ResegmentChoice::RecalculateWithCurrentSettings) => {
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
            }
            Some(ResegmentChoice::UseStoredTracks) => {
                let config =
                    loader::config_from_stored_segmentation(&stored, self.processing_config);
                self.loader.spawn_gtd_from_history(
                    prompt.bytes,
                    prompt.filename,
                    config,
                    loader::HistoryOpen::ApplyShelved {
                        db_ref: prompt.db_ref,
                        stored_tracks: prompt.stored_tracks,
                        applied_current_marker_settings: prompt.marker_settings_changed,
                    },
                );
            }
            Some(ResegmentChoice::Cancel) => {}
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
                regions.frozen_at_open(
                    ui,
                    AUTO_PRUNE_RECORDINGS_REGION,
                    HeldBodyLines::what_the_content_took()
                        .and_at_most(AUTO_PRUNE_RECORDINGS_MOST_LINES),
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
fn mutation_toast(op: &history_db::DbOp) -> String {
    use history_db::{DbOp, DeleteReason};
    match op {
        DbOp::TracksShelved { count } => {
            let tracks = gt_fmt::pluralize(*count, "track", "tracks");
            format!("Shelved {count} {tracks} in history")
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
