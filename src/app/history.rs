use chrono::{DateTime, NaiveDate, Utc};
use egui::{Button, Checkbox, DragValue, Grid, Label, RichText, ScrollArea, TextEdit, Window};
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PENCIL_SIMPLE as ICON_PENCIL_SIMPLE;
use egui_phosphor::regular::X as ICON_X;
use gt_history::{DatabaseRef, PruneMode, RecordingEntry, RecordingMeta};
use gt_side_panel::widgets::{MetadataView, has_metadata_details, metadata_detail_rows};
use gt_types::TravelMode;
use gt_ui_theme::warning_amber;

use crate::app::history_db::{DeleteReason, HistoryWorker};

/// Which pruning mode is selected in the Prune dialog.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PruneKind {
    Age,
    TotalSize,
    Count,
}

struct PruneDialog {
    open: bool,
    mode: PruneKind,
    /// By-age input: number of days.
    age_days: u32,
    /// By-total-size input: limit in MB.
    size_limit_mb: u32,
    /// By-count input: recordings to keep per identity.
    keep_count: u32,
    /// Preview of which refs would be pruned.
    preview: Option<Vec<DatabaseRef>>,
    /// Whether a preview has been requested and is still being computed.
    preview_pending: bool,
}

impl PruneDialog {
    fn new() -> Self {
        Self {
            open: false,
            mode: PruneKind::Age,
            age_days: 90,
            size_limit_mb: 500,
            keep_count: 10,
            preview: None,
            preview_pending: false,
        }
    }

    fn reset(&mut self) {
        self.preview = None;
        self.preview_pending = false;
    }

    /// Apply a preview result that arrived from the worker.
    fn set_preview(&mut self, refs: Vec<DatabaseRef>) {
        self.preview = Some(refs);
        self.preview_pending = false;
    }

    fn to_prune_mode(&self) -> PruneMode {
        match self.mode {
            PruneKind::Age => PruneMode::ByAge {
                max_age_secs: self.age_days as u64 * 86_400,
            },
            PruneKind::TotalSize => PruneMode::ByTotalSize {
                max_bytes: self.size_limit_mb as u64 * 1_024 * 1_024,
            },
            PruneKind::Count => PruneMode::ByCount {
                keep: self.keep_count as usize,
            },
        }
    }

    /// Show the Prune dialog. Sends preview/delete requests to `worker`. The
    /// results arrive asynchronously via [`HistoryWindow::set_prune_preview`].
    fn show(&mut self, ctx: &egui::Context, worker: &HistoryWorker) {
        if !self.open {
            return;
        }

        let mut open = self.open;
        let mut do_prune = false;
        let mut do_preview = false;
        let mut do_cancel_preview = false;

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            open = false;
        }

        Window::new("Prune History…")
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Mode");
                    let old = self.mode;
                    ui.selectable_value(&mut self.mode, PruneKind::Age, "By age");
                    ui.selectable_value(&mut self.mode, PruneKind::TotalSize, "By total size");
                    ui.selectable_value(&mut self.mode, PruneKind::Count, "By count");
                    if self.mode != old {
                        self.reset();
                    }
                });

                ui.add_space(4.0);

                let params_changed = match self.mode {
                    PruneKind::Age => {
                        let prev = self.age_days;
                        ui.horizontal(|ui| {
                            ui.label("Remove recordings older than");
                            ui.add(DragValue::new(&mut self.age_days).range(1..=3650));
                            ui.label("days");
                        });
                        self.age_days != prev
                    }
                    PruneKind::TotalSize => {
                        let prev = self.size_limit_mb;
                        ui.horizontal(|ui| {
                            ui.label("Keep total size under");
                            ui.add(
                                DragValue::new(&mut self.size_limit_mb).range(1..=100_000),
                            );
                            ui.label("MB");
                        });
                        self.size_limit_mb != prev
                    }
                    PruneKind::Count => {
                        let prev = self.keep_count;
                        ui.horizontal(|ui| {
                            ui.label("Keep at most");
                            ui.add(DragValue::new(&mut self.keep_count).range(1..=10_000));
                            ui.label("recordings per identity");
                        });
                        self.keep_count != prev
                    }
                };

                if params_changed {
                    // A preview for the old parameters is now stale. Drop any
                    // in-flight request so its result is ignored.
                    self.preview = None;
                    self.preview_pending = false;
                }

                ui.add_space(4.0);
                ui.separator();

                // Preview button / spinner / computed preview
                if let Some(refs) = &self.preview {
                    if refs.is_empty() {
                        ui.label("Nothing to prune");
                    } else {
                        let n = refs.len();
                        let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                        ui.label(format!("{n} {rec_label} will be deleted"));
                        ScrollArea::vertical()
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for r in refs {
                                    let label = format!("{}/{}", r.identity, r.group_name);
                                    ui.add(Label::new(label.as_str()).truncate())
                                        .on_hover_text(label.as_str());
                                }
                            });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let confirm_btn = ui
                                .button(
                                    RichText::new("Delete these recordings")
                                        .color(warning_amber(ui.visuals().dark_mode)),
                                )
                                .on_hover_text(
                                    "This cannot be undone. The original source files are unaffected.",
                                );
                            if confirm_btn.clicked() {
                                do_prune = true;
                            }
                            if ui.button("Cancel").clicked() {
                                do_cancel_preview = true;
                            }
                        });
                    }
                } else if self.preview_pending {
                    ui.spinner();
                } else if ui.button("Preview").clicked() {
                    do_preview = true;
                }
            });

        self.open = open;

        if do_preview {
            worker.prune_preview(self.to_prune_mode());
            self.preview_pending = true;
        }
        if do_cancel_preview {
            self.reset();
        }
        if do_prune {
            let refs = self.preview.take().unwrap_or_default();
            self.open = false;
            self.reset();
            if !refs.is_empty() {
                worker.delete_recordings(refs, DeleteReason::Prune);
            }
        }
    }
}

pub struct HistoryWindow {
    pub open: bool,
    /// Cached recording list - `None` until the window is first shown.
    entries: Option<Vec<RecordingEntry>>,
    /// Identity substring filter (case-insensitive).
    filter_text: String,
    /// Minimum nav-point count filter (empty = no filter).
    filter_min_points: String,
    /// Maximum nav-point count filter (empty = no filter).
    filter_max_points: String,
    /// Start-date lower bound in `YYYY-MM-DD` (empty = no filter).
    filter_date_from: String,
    /// Start-date upper bound in `YYYY-MM-DD`, inclusive (empty = no filter).
    filter_date_to: String,
    /// Error from the last operation, if any.
    error: Option<String>,
    prune: PruneDialog,
    /// Whether the "delete hidden data" confirmation dialog is open.
    confirm_delete_hidden: bool,
    /// Whether a recording-list request is in flight (drives the spinner and
    /// prevents re-requesting every frame while waiting).
    list_pending: bool,
    /// In-progress inline identity rename, if any.
    rename: Option<RenameEdit>,
}

/// State for the inline identity-rename editor on one History row.
struct RenameEdit {
    /// The current (old) identity of the row being edited - identifies the row.
    identity: String,
    /// The editable buffer, seeded with the identity's display form.
    buffer: String,
}

impl HistoryWindow {
    pub fn new() -> Self {
        Self {
            open: false,
            entries: None,
            filter_text: String::new(),
            filter_min_points: String::new(),
            filter_max_points: String::new(),
            filter_date_from: String::new(),
            filter_date_to: String::new(),
            error: None,
            prune: PruneDialog::new(),
            confirm_delete_hidden: false,
            list_pending: false,
            rename: None,
        }
    }

    fn any_filter_active(&self) -> bool {
        !self.filter_text.is_empty()
            || !self.filter_min_points.is_empty()
            || !self.filter_max_points.is_empty()
            || !self.filter_date_from.is_empty()
            || !self.filter_date_to.is_empty()
    }

    /// Call after a mutation to force a list refresh next time the window shows.
    pub fn invalidate(&mut self) {
        self.entries = None;
        self.list_pending = false;
    }

    /// Apply a recording list that arrived from the worker.
    pub fn set_entries(&mut self, entries: Vec<RecordingEntry>) {
        self.entries = Some(entries);
        self.list_pending = false;
        self.error = None;
    }

    /// Record an error from a failed list request.
    pub fn set_error(&mut self, message: String) {
        self.error = Some(message);
        self.list_pending = false;
    }

    /// Apply a prune-preview result that arrived from the worker.
    pub fn set_prune_preview(&mut self, refs: Vec<DatabaseRef>) {
        self.prune.set_preview(refs);
    }

    /// Show the History window. All database work is sent to `worker`. Results
    /// arrive asynchronously and are applied via [`HistoryWindow::set_entries`]
    /// and friends.
    ///
    /// `loaded_metas` are the content fingerprints of the files currently loaded
    /// in the app, used to disable re-opening a recording that is already open.
    #[expect(
        clippy::too_many_arguments,
        reason = "the window drives several independent pieces of persisted app state plus the loaded-file set; bundling them would obscure rather than clarify"
    )]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        worker: &HistoryWorker,
        loaded_metas: &[RecordingMeta],
        storage_enabled: &mut bool,
        auto_prune_enabled: &mut bool,
        auto_prune_max_bytes: &mut u64,
        auto_prune_confirm: &mut bool,
    ) {
        if !self.open {
            return;
        }

        // Request the recording list once when it is missing. The worker replies
        // via `set_entries`. A spinner shows until then.
        if self.entries.is_none() && !self.list_pending && worker.available() {
            worker.list();
            self.list_pending = true;
        }

        // Show Prune dialog (a separate window).
        self.prune.show(ctx, worker);

        // Hidden tracks live inside otherwise-visible recordings (there is no
        // recording-level hide). Count them across all recordings so the toolbar
        // can offer a "Delete hidden data" action that permanently drops them.
        let hidden_count: usize = self
            .entries
            .as_ref()
            .map(|entries| entries.iter().map(|e| e.hidden_tracks).sum())
            .unwrap_or_default();

        let mut open = self.open;

        // Escape closes the whole window only when nothing more local wants it:
        // while the confirmation dialog is up it dismisses that, and while an
        // inline rename is open it must reach the editor to cancel it (so the
        // key is left unconsumed here in that case, via short-circuit).
        if self.rename.is_none()
            && !self.confirm_delete_hidden
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }

        // Take the inline-rename state out so `render_row` can mutate it while
        // `self.entries` is borrowed immutably for the list; restored after.
        let mut rename = std::mem::take(&mut self.rename);

        Window::new("History")
            .open(&mut open)
            .resizable(true)
            .default_width(640.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                if !worker.available() {
                    ui.label(
                        RichText::new("History database is unavailable.")
                            .color(warning_amber(ui.visuals().dark_mode)),
                    );
                    return;
                }

                if let Some(err) = &self.error {
                    ui.label(RichText::new(err).color(warning_amber(ui.visuals().dark_mode)));
                    ui.add_space(4.0);
                }

                if self.entries.is_none() {
                    ui.spinner();
                    return;
                }

                // Snapshot filter active state before the closures that mutably
                // borrow individual filter fields - avoids whole-self method calls
                // inside closures where `entries` also holds an immutable borrow.
                let filter_active = self.any_filter_active();

                // Toolbar row: identity filter + Prune button
                ui.horizontal(|ui| {
                    crate::terms::term_label(
                        ui,
                        RichText::new("Identity"),
                        crate::terms::IDENTITY,
                    );
                    ui.text_edit_singleline(&mut self.filter_text);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let delete_hidden_label = if hidden_count > 0 {
                            format!("Delete hidden data ({hidden_count})…")
                        } else {
                            "Delete hidden data…".to_owned()
                        };
                        let delete_hidden = ui
                            .add_enabled(hidden_count > 0, Button::new(delete_hidden_label))
                            .on_hover_text(if hidden_count > 0 {
                                "Permanently delete every hidden track from the original recordings"
                            } else {
                                "No hidden tracks to delete"
                            });
                        if delete_hidden.clicked() {
                            self.confirm_delete_hidden = true;
                        }
                        if ui.button("Prune…").clicked() {
                            self.prune.open = true;
                            self.prune.reset();
                        }
                        ui.checkbox(storage_enabled, "Auto-store recordings");
                    });
                });

                // Advanced filter row: points + date range
                ui.horizontal(|ui| {
                    ui.label("Points ≥");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_min_points).desired_width(60.0),
                    );
                    ui.label("≤");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_max_points).desired_width(60.0),
                    );
                    ui.separator();
                    ui.label("Date");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_date_from)
                            .desired_width(90.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    ui.label("–");
                    ui.add(
                        TextEdit::singleline(&mut self.filter_date_to)
                            .desired_width(90.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    if filter_active && ui.small_button(ICON_X).clicked() {
                        self.filter_text.clear();
                        self.filter_min_points.clear();
                        self.filter_max_points.clear();
                        self.filter_date_from.clear();
                        self.filter_date_to.clear();
                    }
                });

                // Auto-prune settings - separated because this is a persistent
                // setting, not a filter or list entry.  Always rendered so the
                // layout stays stable, controls are grayed out when inactive,
                // with hover text explaining what to enable first.
                ui.separator();
                ui.horizontal(|ui| {
                    let storage_on = *storage_enabled;
                    let prune_on = *auto_prune_enabled && storage_on;

                    ui.add_enabled(
                        storage_on,
                        Checkbox::new(auto_prune_enabled, "Auto-prune when over"),
                    )
                    .on_hover_text(if storage_on {
                        "Automatically delete the oldest recordings when storage exceeds the threshold"
                    } else {
                        "Enable 'Auto-store recordings' to use auto-pruning"
                    });

                    let mut max_gb =
                        *auto_prune_max_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    ui.add_enabled(
                        prune_on,
                        DragValue::new(&mut max_gb)
                            .range(0.1..=1_000.0)
                            .speed(0.1),
                    )
                    .on_hover_text(if prune_on {
                        "Storage limit - oldest recordings are pruned when this is exceeded"
                    } else if storage_on {
                        "Tick 'Auto-prune when over' to set a threshold"
                    } else {
                        "Enable 'Auto-store recordings' to use auto-pruning"
                    });

                    if prune_on {
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "DragValue range is 0.1..=1000 so value is always positive"
                        )]
                        let bytes = (max_gb * 1024.0 * 1024.0 * 1024.0).round() as u64;
                        *auto_prune_max_bytes = bytes;
                    }

                    ui.label("GB");

                    ui.separator();

                    ui.add_enabled(
                        prune_on,
                        Checkbox::new(auto_prune_confirm, "Confirm before pruning"),
                    )
                    .on_hover_text(if prune_on {
                        "Show a confirmation dialog before auto-pruning deletes recordings"
                    } else if storage_on {
                        "Tick 'Auto-prune when over' to configure this"
                    } else {
                        "Enable 'Auto-store recordings' to use auto-pruning"
                    });
                });
                ui.add_space(4.0);

                let Some(entries) = &self.entries else {
                    return;
                };

                let filter_identity = self.filter_text.to_lowercase();
                let filter_min_points: Option<u64> = self.filter_min_points.parse().ok();
                let filter_max_points: Option<u64> = self.filter_max_points.parse().ok();
                let filter_from_us = date_to_start_us(&self.filter_date_from);
                let filter_to_us = date_to_end_us(&self.filter_date_to);

                let visible: Vec<&RecordingEntry> = entries
                    .iter()
                    .filter(|e| {
                        if !filter_identity.is_empty()
                            && !e
                                .db_ref
                                .identity
                                .to_lowercase()
                                .contains(filter_identity.as_str())
                        {
                            return false;
                        }
                        if filter_min_points.is_some_and(|min| e.meta.nav_point_count < min) {
                            return false;
                        }
                        if filter_max_points.is_some_and(|max| e.meta.nav_point_count > max) {
                            return false;
                        }
                        if filter_from_us.is_some_and(|from| e.meta.start_us < from) {
                            return false;
                        }
                        if filter_to_us.is_some_and(|to| e.meta.start_us > to) {
                            return false;
                        }
                        true
                    })
                    .collect();

                if entries.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label("No recordings in history yet");
                    });
                    return;
                }

                // Reserve space for stats footer
                let footer_height = ui.text_style_height(&egui::TextStyle::Body) + 8.0;
                let available = ui.available_size();
                let list_height = (available.y - footer_height - 8.0).max(100.0);

                ScrollArea::vertical()
                    .max_height(list_height)
                    .show(ui, |ui| {
                        Grid::new("history_list")
                            .num_columns(6)
                            .striped(true)
                            .min_col_width(80.0)
                            .show(ui, |ui| {
                                crate::terms::term_label(
                                    ui,
                                    RichText::new("Identity").strong(),
                                    crate::terms::IDENTITY,
                                );
                                ui.strong("Date");
                                ui.strong("Duration");
                                ui.strong("Points");
                                ui.strong("Size");
                                ui.label("");
                                ui.end_row();

                                for entry in &visible {
                                    let already_loaded = loaded_metas
                                        .iter()
                                        .any(|m| m.same_recording(&entry.meta));
                                    render_row(ui, entry, already_loaded, worker, &mut rename);
                                }
                            });
                    });

                ui.separator();
                // Footer stats cover every stored recording. Hidden tracks are
                // reported separately since they are pending permanent deletion.
                let stored_count = entries.len();
                let total_size: u64 = entries.iter().map(|e| e.meta.gtd_size_bytes).sum();
                ui.horizontal(|ui| {
                    let rec_label = gt_fmt::pluralize(stored_count, "recording", "recordings");
                    ui.label(format!(
                        "{stored_count} {rec_label} - {}",
                        format_size(total_size)
                    ));
                    if filter_active && visible.len() != stored_count {
                        ui.weak(format!("({} shown)", visible.len()));
                    }
                    if hidden_count > 0 {
                        let track_label = gt_fmt::pluralize(hidden_count, "track", "tracks");
                        ui.weak(format!("- {hidden_count} hidden {track_label}"));
                    }
                });
                if let Some(path) = worker.path() {
                    ui.weak(path.display().to_string());
                }
            });

        self.rename = rename;

        // Confirmation for the destructive "delete hidden data" action, mirroring
        // the prune/auto-prune confirm flow (no permanent delete without a prompt).
        if self.confirm_delete_hidden {
            if hidden_count == 0 {
                self.confirm_delete_hidden = false;
            } else {
                let mut do_delete = false;
                let mut cancel =
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                Window::new("Delete hidden data?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .show(ctx, |ui| {
                        let track_label = gt_fmt::pluralize(hidden_count, "track", "tracks");
                        ui.label(format!(
                            "{hidden_count} hidden {track_label} will be permanently removed from their recordings."
                        ));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button(
                                    RichText::new("Delete hidden tracks")
                                        .color(warning_amber(ui.visuals().dark_mode)),
                                )
                                .on_hover_text(
                                    "This cannot be undone. The original source files are unaffected.",
                                )
                                .clicked()
                            {
                                do_delete = true;
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                if do_delete {
                    worker.delete_hidden_tracks();
                    self.confirm_delete_hidden = false;
                } else if cancel {
                    self.confirm_delete_hidden = false;
                }
            }
        }

        self.open = open;
    }
}

fn render_row(
    ui: &mut egui::Ui,
    entry: &RecordingEntry,
    already_loaded: bool,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
) {
    // Identity column: the inline editor when this row is being renamed,
    // otherwise the normal cell.
    if rename
        .as_ref()
        .is_some_and(|r| r.identity == entry.db_ref.identity)
    {
        render_rename_editor(ui, rename, worker);
    } else {
        identity_cell(ui, entry);
    }

    let ts = DateTime::<Utc>::from_timestamp_micros(entry.meta.start_us)
        .unwrap_or_default()
        .format("%Y-%m-%d %H:%M")
        .to_string();
    ui.label(&ts);

    let dur_us = entry.meta.end_us.saturating_sub(entry.meta.start_us).max(0);
    let dur = chrono::Duration::microseconds(dur_us);
    ui.label(format_duration(dur));

    let count = gt_history::format_count_suffix(entry.meta.nav_point_count);
    ui.horizontal(|ui| {
        ui.label(count);
        if entry.hidden_tracks > 0 {
            ui.weak(format!("({}/{} hidden)", entry.hidden_tracks, entry.total_tracks))
                .on_hover_text(
                    "Tracks hidden by 'remove filtered data'; use 'Delete hidden data' to drop them permanently",
                );
        }
    });

    ui.label(format_size(entry.meta.gtd_size_bytes));

    ui.horizontal(|ui| {
        let open = ui.add_enabled(!already_loaded, Button::new("Open").small());
        if already_loaded {
            open.on_hover_text("Already loaded");
        } else if open.clicked() {
            worker.open(entry.db_ref.clone());
        }
        if ui
            .small_button(ICON_PENCIL_SIMPLE)
            .on_hover_text("Rename identity")
            .clicked()
        {
            let identity = entry.db_ref.identity.clone();
            let buffer = identity_display_parts(&identity).0.to_owned();
            *rename = Some(RenameEdit { identity, buffer });
        }
        if ui.small_button("Delete").clicked() {
            worker.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
        }
    });

    ui.end_row();
}

/// Render the inline identity-rename editor in the identity column. Commits on
/// Enter, cancels on focus loss (click-away or Escape); either way the editor
/// closes. A no-op commit (empty, or unchanged from the displayed name) does not
/// send a rename. `rename` is guaranteed `Some` by the caller.
fn render_rename_editor(
    ui: &mut egui::Ui,
    rename: &mut Option<RenameEdit>,
    worker: &HistoryWorker,
) {
    let Some(edit) = rename.as_mut() else {
        return;
    };
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let resp = ui.add(
        TextEdit::singleline(&mut edit.buffer)
            .desired_width(f32::INFINITY)
            .hint_text("Identity"),
    );
    if resp.lost_focus() {
        let old = std::mem::take(&mut edit.identity);
        let new = edit.buffer.trim().to_owned();
        let unchanged = new == identity_display_parts(&old).0;
        *rename = None;
        if enter && !new.is_empty() && !unchanged {
            worker.rename_identity(old, new);
        }
    } else {
        // Keep focus in the freshly-opened editor until the user commits or
        // clicks away.
        resp.request_focus();
    }
}

/// Render the identity column cell: an optional `auto` badge plus the identity,
/// truncated so a long name clips itself rather than growing the window. A note
/// icon marks recordings that carry SDK metadata. The full identity, and any
/// title/device/notes, stay available on hover.
fn identity_cell(ui: &mut egui::Ui, entry: &RecordingEntry) {
    let identity = entry.db_ref.identity.as_str();
    let (display_name, is_auto) = identity_display_parts(identity);
    // The full identity is the hover's first line, so leave it out of the view:
    // the note icon and rows are for the SDK's title/device/notes only. Every
    // recording has an identity, so including it would badge every row.
    let travel_mode = entry.travel_mode.as_deref().map(travel_mode_display);
    let meta = MetadataView {
        title: entry.title.as_deref(),
        device: entry.device.as_deref(),
        travel_mode: travel_mode.as_deref(),
        identity: None,
        notes: entry.notes.as_deref(),
    };
    let has_metadata = has_metadata_details(&meta);
    ui.horizontal(|ui| {
        if is_auto {
            ui.label(
                RichText::new("auto")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
        if has_metadata {
            ui.label(RichText::new(ICON_NOTE).weak());
        }
        ui.add(Label::new(display_name).truncate());
    })
    .response
    .on_hover_ui(|ui| {
        ui.label(identity);
        metadata_detail_rows(ui, &meta);
    });
}

fn identity_display_parts(identity: &str) -> (&str, bool) {
    gt_loaded_files::display_identity(identity)
}

/// Display form of a History entry's raw travel-mode wire value (the DB stores
/// the `meta_travel_mode` attribute verbatim): known modes get their human
/// spelling, unknown wire values pass through verbatim.
fn travel_mode_display(wire: &str) -> String {
    TravelMode::from_wire(wire).display_name().to_owned()
}

fn format_duration(dur: chrono::Duration) -> String {
    let total_secs = dur.num_seconds().max(0);
    let d = total_secs / 86400;
    let h = (total_secs % 86400) / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "—".to_owned();
    }
    if bytes < 1_024 {
        return format!("{bytes} B");
    }
    if bytes < 1_024 * 1_024 {
        let kb = bytes as f64 / 1_024.0;
        return format!("{kb:.1} KB");
    }
    let mb = bytes as f64 / (1_024.0 * 1_024.0);
    format!("{mb:.1} MB")
}

/// Parse a `YYYY-MM-DD` string into microseconds-since-epoch at the start of that day (UTC).
/// Returns `None` if the string is empty or not a valid date.
fn date_to_start_us(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let dt = date.and_hms_opt(0, 0, 0)?.and_utc();
    Some(dt.timestamp_micros())
}

/// Parse a `YYYY-MM-DD` string into microseconds-since-epoch at the end of that day (UTC),
/// so the "to" bound is inclusive.
/// Returns `None` if the string is empty or not a valid date.
fn date_to_end_us(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let dt = date.and_hms_opt(23, 59, 59)?.and_utc();
    Some(dt.timestamp_micros())
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable as _;
    use gt_history::HistoryDatabase as _;
    use gt_test_utils::TestHarness;

    use crate::app::history_db::Response;

    use super::{
        DatabaseRef, Grid, HistoryWindow, HistoryWorker, ICON_NOTE, ICON_PENCIL_SIMPLE,
        RecordingEntry, RecordingMeta, ScrollArea, Window, identity_cell, identity_display_parts,
        travel_mode_display,
    };

    /// Harness state for driving the History window: the window, a live (empty)
    /// worker so the list branch renders, and the settings toggles `show` needs.
    struct HistoryHarness {
        window: HistoryWindow,
        worker: HistoryWorker,
        storage_enabled: bool,
        auto_prune_enabled: bool,
        auto_prune_max_bytes: u64,
        auto_prune_confirm: bool,
        _dir: tempfile::TempDir,
    }

    fn history_harness(entries: Vec<RecordingEntry>) -> HistoryHarness {
        let dir = tempfile::tempdir().expect("temp dir");
        let db =
            gt_history::Database::open_or_create(&dir.path().join("history.h5")).expect("open db");
        let worker = HistoryWorker::spawn(db, egui::Context::default());
        let mut window = HistoryWindow::new();
        window.open = true;
        // Populate directly so the list renders without a worker round-trip.
        window.set_entries(entries);
        HistoryHarness {
            window,
            worker,
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 0,
            auto_prune_confirm: true,
            _dir: dir,
        }
    }

    fn show_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
        s.window.show(
            ui.ctx(),
            &s.worker,
            &[],
            &mut s.storage_enabled,
            &mut s.auto_prune_enabled,
            &mut s.auto_prune_max_bytes,
            &mut s.auto_prune_confirm,
        );
    }

    /// A harness backed by a real database holding one recording, with no
    /// pre-seeded entries - the list arrives from the worker (see [`pump_history`]).
    fn history_harness_with_recording(identity: &str) -> HistoryHarness {
        use gt_history::{StoredSegmentation, TrackRange};

        let dir = tempfile::tempdir().expect("temp dir");
        let mut db =
            gt_history::Database::open_or_create(&dir.path().join("history.h5")).expect("open db");
        let bytes = gt_test_utils::GOLD_BYTES;
        let meta = gt_history::extract_meta(bytes).expect("meta");
        let tracks = [TrackRange {
            start: 0,
            end: meta.nav_point_count,
            hidden: false,
        }];
        let settings = StoredSegmentation {
            track_split_gap_us: 300_000_000,
            detect_clock_discontinuities: true,
            clock_discontinuity_sigmas: 5.0,
        };
        db.insert(identity, &meta, &tracks, settings, bytes)
            .expect("insert recording");
        let worker = HistoryWorker::spawn(db, egui::Context::default());
        let mut window = HistoryWindow::new();
        window.open = true;
        HistoryHarness {
            window,
            worker,
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 0,
            auto_prune_confirm: true,
            _dir: dir,
        }
    }

    /// Drive one frame like the app does: drain the worker's responses into the
    /// window (list refresh, mutation acknowledgements) and then render it.
    fn pump_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
        for resp in s.worker.poll() {
            match resp {
                Response::Listed(Ok(entries)) => s.window.set_entries(entries),
                Response::Mutated { result: Ok(()), .. } => s.window.invalidate(),
                _ => {}
            }
        }
        show_history(ui, s);
    }

    /// Run frames (yielding to the worker thread) until `pred` holds or the
    /// budget is exhausted; returns whether it held.
    fn run_until(
        h: &mut TestHarness<HistoryHarness>,
        pred: impl Fn(&mut TestHarness<HistoryHarness>) -> bool,
    ) -> bool {
        for _ in 0..100 {
            // Single-frame `step` (not `run`): the History window paints a spinner
            // while a list request is in flight, so it never reaches a quiescent
            // state for `run` to converge on.
            h.step();
            if pred(h) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        false
    }

    #[test]
    fn rename_workflow_updates_the_listed_identity_end_to_end() {
        // Full workflow against a real worker + database: the row lists, the user
        // edits the identity inline, and after the async rename the list shows the
        // new name.
        let harness = history_harness_with_recording("auto:ride.gtd");
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(pump_history, harness);

        // The recording lists under its stripped identity.
        assert!(
            run_until(&mut h, |h| h
                .inner
                .query_by_label_contains("ride.gtd")
                .is_some()),
            "recording should appear in the History list"
        );

        // Open the inline editor. `request_focus` applies the frame after the
        // editor first renders, so settle a couple of frames before typing.
        h.inner.get_by_label(ICON_PENCIL_SIMPLE).click();
        h.step();
        h.step();
        // Append to the seeded name and commit with Enter.
        h.inner.event(egui::Event::Text(" v2".to_owned()));
        h.step();
        h.inner.key_press(egui::Key::Enter);
        h.step();

        // After the worker renames and the window re-lists, the new identity shows.
        assert!(
            run_until(&mut h, |h| h
                .inner
                .query_by_label_contains("ride.gtd v2")
                .is_some()),
            "the renamed identity should appear in the refreshed list"
        );
    }

    #[test]
    fn clicking_rename_opens_inline_identity_editor() {
        let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        h.run();
        // The rename pencil is present; clicking it swaps the identity cell for
        // the inline text editor (seeded with the `auto:`-stripped name).
        h.inner.get_by_label(ICON_PENCIL_SIMPLE).click();
        h.run();
        assert!(
            h.inner.query_all_by_value("ride.gtd").next().is_some(),
            "inline editor should show the stripped identity as its value"
        );
    }

    /// A listing entry for `identity` with no tracks and no SDK metadata, for the
    /// identity-cell layout tests.
    fn entry_with_identity(identity: &str) -> RecordingEntry {
        RecordingEntry {
            db_ref: DatabaseRef {
                identity: identity.to_owned(),
                group_name: "rec0".to_owned(),
            },
            meta: RecordingMeta {
                start_us: 0,
                end_us: 0,
                nav_point_count: 0,
                sat_report_count: 0,
                marker_count: 0,
                event_marker_count: 0,
                gtd_size_bytes: 0,
            },
            total_tracks: 0,
            hidden_tracks: 0,
            title: None,
            device: None,
            notes: None,
            travel_mode: None,
        }
    }

    /// The DB hands the listing the raw `meta_travel_mode` wire value; the
    /// hover must show the human spelling for known modes and the preserved
    /// wire value verbatim for unknown ones.
    #[rstest::rstest]
    #[case("bicycle", "Bicycle")]
    #[case("hovercraft", "hovercraft")]
    fn travel_mode_display_humanizes_the_wire_value(#[case] wire: &str, #[case] expected: &str) {
        assert_eq!(travel_mode_display(wire), expected);
    }

    /// A travel mode alone must badge the row with the note icon, proving
    /// `identity_cell` feeds the field into the shared metadata presence check.
    #[test]
    fn travel_mode_alone_shows_the_metadata_note_icon() {
        let mut entry = entry_with_identity("auto:ride.gtd");
        entry.travel_mode = Some("bicycle".to_owned());
        let harness = history_harness(vec![entry]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        h.run();
        assert!(
            h.inner.query_by_label(ICON_NOTE).is_some(),
            "the note icon should appear for an entry whose only metadata is a travel mode"
        );
    }

    /// Settled width of the History window, mirroring the real container: a
    /// resizable [`Window`] holding the identity grid in a vertical scroll
    /// area. A resizable window runs a sizing pass over its content, the path
    /// where an un-truncated label reports its full text width and stretches the
    /// window.
    fn history_window_width(identity: &str) -> f32 {
        let width = std::rc::Rc::new(std::cell::Cell::new(-1.0_f32));
        let probe = std::rc::Rc::clone(&width);
        let entry = entry_with_identity(identity);
        let mut harness = TestHarness::builder()
            .size(egui::vec2(1600.0, 500.0))
            .ui(move |ui| {
                let resp = Window::new("History")
                    .resizable(true)
                    .default_width(640.0)
                    .show(ui.ctx(), |ui| {
                        ScrollArea::vertical().show(ui, |ui| {
                            Grid::new("history_list")
                                .num_columns(6)
                                .min_col_width(80.0)
                                .show(ui, |ui| {
                                    identity_cell(ui, &entry);
                                    ui.label("2026-01-15 12:00");
                                    ui.label("1h 02m");
                                    ui.label("12.3k");
                                    ui.label("4.5 MB");
                                    ui.label("");
                                    ui.end_row();
                                });
                        });
                    });
                if let Some(resp) = resp {
                    probe.set(resp.response.rect.width());
                }
            });
        for _ in 0..6 {
            harness.run();
        }
        width.get()
    }

    /// A long recording identity truncates in the History window rather than
    /// stretching it: a short, a long, and a much longer identity all settle the
    /// resizable window at the same width. Without the truncation the identity
    /// column would size to its full text and the window would grow with it.
    #[test]
    fn long_identity_does_not_widen_history_window() {
        let short = history_window_width("auto:ride.gtd");
        let long = history_window_width(&"a/very/long/recording/identity/".repeat(4));
        let longer = history_window_width(&"a/very/long/recording/identity/".repeat(12));
        assert!(
            (long - short).abs() < 1.0 && (longer - short).abs() < 1.0,
            "identity length changed the history window width: \
             short={short}px long={long}px longer={longer}px",
        );
    }

    #[test]
    fn identity_display_keeps_full_manual_identity_visible() {
        let identity = "/example.invalid/history/identity/with/slashes/";

        assert_eq!(identity_display_parts(identity), (identity, false));
    }

    #[test]
    fn identity_display_marks_auto_identity_without_losing_original() {
        let identity = "auto:recording-2026-07-09.gtd";

        assert_eq!(
            identity_display_parts(identity),
            ("recording-2026-07-09.gtd", true)
        );
    }
}
