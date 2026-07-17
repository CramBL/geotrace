use std::cell::Cell;

use chrono::{DateTime, NaiveDate, Utc};
use egui::{Button, Checkbox, DragValue, Label, RichText, ScrollArea, TextEdit, Window};
use egui_extras::{Column, TableBuilder, TableRow};
use egui_phosphor::regular::NOTE as ICON_NOTE;
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

                // Toolbar row: identity filter on the left, actions on the
                // right. The right-side controls are laid out right-to-left so
                // they claim their width first; the filter field then fills only
                // the space between the label and them. Adding the field in the
                // outer left-to-right layout instead lets it grow into the
                // right-side controls and overlap them once the window narrows.
                ui.horizontal(|ui| {
                    crate::terms::term_label(
                        ui,
                        RichText::new("Identity"),
                        crate::terms::IDENTITY,
                    );
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
                        ui.add(
                            TextEdit::singleline(&mut self.filter_text)
                                .desired_width(ui.available_width()),
                        );
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

                history_table(ui, list_height, &visible, loaded_metas, worker, &mut rename);

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

/// The scrolling recordings table, laid out like a file manager's list: the
/// metadata columns (date, duration, points, size, actions) size to their
/// fixed-format content, and the identity column fills whatever width is left,
/// clipping long names. Its width is a function of the window's width, so the
/// table is always exactly as wide as the window - resize the window to give
/// identity more or less room.
///
/// Identity is sized as a [`Column::exact`] recomputed each frame rather than a
/// [`Column::remainder`] on purpose. A remainder that is not the *last* column
/// ratchets in egui_extras: it feeds its clipped width back into its own minimum
/// every frame, so it can never shrink again, which stops the window from being
/// made narrower and lets it creep wider. Computing the width ourselves sidesteps
/// that: the table always fits the window, so the window stays freely resizable.
fn history_table(
    ui: &mut egui::Ui,
    list_height: f32,
    visible: &[&RecordingEntry],
    loaded_metas: &[gt_history::RecordingMeta],
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
) {
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;

    // Identity fills the width the metadata columns leave over. We size it as an
    // exact column ourselves (window width minus last frame's metadata width)
    // rather than a `Column::remainder`, whose ratcheting breaks window shrink -
    // see this function's doc comment for the full rationale.
    let available_width = ui.available_width();
    let metadata_width_id = ui.id().with("history_metadata_width");
    let identity_width = ui
        .data(|d| d.get_temp::<f32>(metadata_width_id))
        .map_or(IDENTITY_DEFAULT_WIDTH, |metadata| {
            (available_width - metadata).max(IDENTITY_MIN_WIDTH)
        });

    // Right edges of the identity and last (action) columns, captured from the
    // header this frame to measure the metadata width for the next one. The
    // measurement is only trusted outside the table's sizing pass: during it the
    // auto columns have not yet grown to their content, so the reserve reads too
    // small and identity briefly blows up (window sticks wide).
    let identity_right = Cell::new(0.0_f32);
    let last_column_right = Cell::new(0.0_f32);
    let measured_while_sizing = Cell::new(false);

    TableBuilder::new(ui)
        .id_salt("history_list")
        .striped(true)
        // Cells lay out in a row (no vertical wrapping): dates stay on one
        // line and the action buttons sit side by side.
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        // Don't shrink to content: the table fills the window's width, which the
        // computed identity column already accounts for.
        .auto_shrink([false, true])
        .max_scroll_height(list_height)
        // Identity fills the leftover width (see above) and clips long names
        // rather than growing to fit them.
        .column(Column::exact(identity_width).clip(true))
        // Metadata columns size to their fixed-format content. They are not
        // resizable: there is nothing to gain from resizing a date or a byte
        // count, and it keeps the table's width fully determined by the window.
        .columns(Column::auto().resizable(false), 4)
        .column(Column::auto().resizable(false))
        .header(row_height, |mut header| {
            header.col(|ui| {
                identity_right.set(ui.max_rect().right());
                measured_while_sizing.set(ui.is_sizing_pass());
                crate::terms::term_label(
                    ui,
                    RichText::new("Identity").strong(),
                    crate::terms::IDENTITY,
                );
            });
            for title in ["Date", "Duration", "Points", "Size"] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
            header.col(|ui| {
                last_column_right.set(ui.max_rect().right());
            });
        })
        .body(|body| {
            body.rows(row_height, visible.len(), |mut row| {
                // In-range by construction: `rows` hands out indices below
                // `visible.len()`; skip defensively rather than unwrap.
                let Some(entry) = visible.get(row.index()) else {
                    return;
                };
                let already_loaded = loaded_metas.iter().any(|m| m.same_recording(&entry.meta));
                render_row(&mut row, entry, already_loaded, worker, rename);
            });
        });

    // Record the metadata columns' total width (everything right of identity)
    // for next frame's fill calculation. The header always renders - unlike the
    // virtualized body rows - so these edges are always fresh.
    let metadata_width = last_column_right.get() - identity_right.get();
    if metadata_width > 0.0 && !measured_while_sizing.get() {
        ui.data_mut(|d| d.insert_temp(metadata_width_id, metadata_width));
    }
}

/// Identity never collapses below a readable width, even in a narrow window.
const IDENTITY_MIN_WIDTH: f32 = 160.0;

/// Identity's width on the very first frame, before the metadata columns have
/// been measured (see [`history_table`]); from then on it fills the leftover
/// width. Kept above [`IDENTITY_MIN_WIDTH`] so this bootstrap value is already a
/// readable width without needing the same clamp the measured path applies.
const IDENTITY_DEFAULT_WIDTH: f32 = 280.0;

fn render_row(
    row: &mut TableRow<'_, '_>,
    entry: &RecordingEntry,
    already_loaded: bool,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
) {
    // Identity column: the inline editor when this row is being renamed,
    // otherwise the normal cell.
    row.col(|ui| {
        if rename
            .as_ref()
            .is_some_and(|r| r.identity == entry.db_ref.identity)
        {
            render_rename_editor(ui, rename, worker);
        } else {
            identity_cell(ui, entry, worker, rename);
        }
    });

    row.col(|ui| {
        let ts = DateTime::<Utc>::from_timestamp_micros(entry.meta.start_us)
            .unwrap_or_default()
            .format("%Y-%m-%d %H:%M")
            .to_string();
        ui.label(ts);
    });

    row.col(|ui| {
        let dur_us = entry.meta.end_us.saturating_sub(entry.meta.start_us).max(0);
        let dur = chrono::Duration::microseconds(dur_us);
        ui.label(format_duration(dur));
    });

    row.col(|ui| {
        ui.label(gt_history::format_count_suffix(entry.meta.nav_point_count));
        if entry.hidden_tracks > 0 {
            ui.weak(format!(
                "({}/{} hidden)",
                entry.hidden_tracks, entry.total_tracks
            ))
            .on_hover_text(
                "Tracks hidden by 'remove filtered data'; use 'Delete hidden data' to drop them permanently",
            );
        }
    });

    row.col(|ui| {
        ui.label(format_size(entry.meta.gtd_size_bytes));
    });

    row.col(|ui| {
        let open = ui.add_enabled(!already_loaded, Button::new("Open").small());
        if already_loaded {
            open.on_hover_text("Already loaded");
        } else if open.clicked() {
            worker.open(entry.db_ref.clone());
        }
        if ui.small_button("Delete").clicked() {
            worker.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
        }
    });
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

/// Open the inline rename editor for a recording's identity.
fn begin_rename(rename: &mut Option<RenameEdit>, entry: &RecordingEntry) {
    let identity = entry.db_ref.identity.clone();
    let buffer = identity_display_parts(&identity).0.to_owned();
    *rename = Some(RenameEdit { identity, buffer });
}

/// The identity column of a History row. Double-clicking the cell opens the
/// inline rename editor; right-clicking offers Rename and Delete.
fn identity_cell(
    ui: &mut egui::Ui,
    entry: &RecordingEntry,
    worker: &HistoryWorker,
    rename: &mut Option<RenameEdit>,
) {
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
    let label = ui
        .horizontal(|ui| {
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
            // The label itself senses clicks: it is the rename target.
            ui.add(
                Label::new(display_name)
                    .truncate()
                    .sense(egui::Sense::click()),
            )
        })
        .inner
        .on_hover_ui(|ui| {
            ui.label(identity);
            metadata_detail_rows(ui, &meta);
            ui.label(
                RichText::new("Double-click to rename")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
    if label.double_clicked() {
        begin_rename(rename, entry);
    }
    label.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            begin_rename(rename, entry);
            ui.close();
        }
        if ui.button("Delete").clicked() {
            worker.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
            ui.close();
        }
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
        DatabaseRef, HistoryWindow, HistoryWorker, ICON_NOTE, RecordingEntry, RecordingMeta,
        identity_display_parts, travel_mode_display,
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

        // Open the inline editor through the identity's context menu.
        // `request_focus` applies the frame after the editor first renders,
        // so settle a couple of frames before typing.
        h.inner.get_by_label_contains("ride.gtd").click_secondary();
        h.step();
        h.inner.get_by_label("Rename").click_accesskit();
        h.step();
        h.step();
        assert!(
            h.inner.query_all_by_value("ride.gtd").next().is_some(),
            "probe: editor not open after Rename click"
        );

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

    /// The recordings table: identity takes the remaining width (long names
    /// get the room), the value columns stay compact, headers carry the
    /// resize handles.
    #[test]
    fn snapshot_history_window_table() {
        let mut harness = history_harness(vec![
            entry_with_identity("auto:ride.gtd"),
            entry_with_identity("a much longer recording identity that needs the room"),
            entry_with_identity("survey_flight_2026_07_15.gtd"),
        ]);
        // The temporary database path differs every run; keep it out of the
        // image.
        harness.worker.hide_path();
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .ui_state(show_history, harness);
        // Auto columns measure their content over the first frames; settle
        // before snapshotting.
        for _ in 0..4 {
            h.run();
        }
        h.snapshot("history_window_table");
    }

    #[test]
    fn double_clicking_identity_opens_inline_editor() {
        let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
        // Frames at 60 fps: kittest's default 0.25 s/frame clock (one frame
        // per queued event) spaces the two clicks beyond egui's 0.3 s
        // double-click window.
        let mut h = TestHarness::builder()
            .size(egui::vec2(900.0, 500.0))
            .step_dt(1.0 / 60.0)
            .ui_state(show_history, harness);
        h.run();
        // Two quick clicks on the identity label register as a double click
        // and swap the cell for the inline text editor (seeded with the
        // `auto:`-stripped name).
        h.inner.get_by_label_contains("ride.gtd").click();
        h.inner.get_by_label_contains("ride.gtd").click();
        h.run();
        assert!(
            h.inner.query_all_by_value("ride.gtd").next().is_some(),
            "inline editor should show the stripped identity as its value"
        );
        // The editor holds keyboard focus: typing extends its buffer.
        h.step();
        h.inner.event(egui::Event::Text(" v2".to_owned()));
        h.step();
        h.step();
        assert!(
            h.inner.query_all_by_value("ride.gtd v2").next().is_some(),
            "typed text should reach the freshly opened editor"
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

    /// Settled width of the History window, through the real rendering path
    /// ([`HistoryWindow::show`]). A resizable window runs a sizing pass over
    /// its content, the path where an un-clipped column would report its
    /// full text width and stretch the window.
    fn history_window_width(identity: &str) -> f32 {
        let harness = history_harness(vec![entry_with_identity(identity)]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(1600.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..6 {
            h.run();
        }
        let window = h
            .inner
            .get_by_role_and_label(egui::accesskit::Role::Window, "History");
        window.rect().width()
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

    /// The metadata-width measurement is ignored during the table's sizing pass:
    /// on the first frame the auto columns have not grown to their content, so
    /// the reserve reads far too small and, if cached, would inflate identity and
    /// stick the window permanently wide. A freshly opened window must therefore
    /// settle to its content width, not a bloated one.
    #[test]
    fn fresh_window_settles_to_content_width_not_a_bloated_one() {
        // Room to bloat into: the screen is 1600px, the content needs well under
        // half that. A leaked sizing-pass measurement pushed this past 900px.
        let width = history_window_width("auto:ride.gtd");
        assert!(
            width < 750.0,
            "the History window settled far wider than its content ({width:.0}px); \
             the sizing-pass metadata measurement likely leaked into the identity fill",
        );
    }

    /// The identity filter field fills the toolbar space to the left of the
    /// action controls and must yield as the window narrows, never growing into
    /// them. Previously the field kept a fixed width and the "Auto-store
    /// recordings" checkbox slid left underneath it, overlapping.
    #[test]
    fn filter_field_does_not_overlap_the_toolbar_controls() {
        let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(1200.0, 500.0))
            .ui_state(show_history, harness);
        for _ in 0..8 {
            h.step();
        }
        // Shrink toward the window's minimum, where the overlap used to appear.
        let w = window_rect(&h);
        drag(
            &mut h,
            egui::pos2(w.right() - 1.0, w.bottom() - 1.0),
            egui::vec2(-500.0, 0.0),
            10,
        );
        for _ in 0..3 {
            h.step();
        }

        let checkbox_left = h.inner.get_by_label("Auto-store recordings").rect().left();
        // The first text input in the window is the identity filter field.
        let filter_right = h
            .inner
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .map(|n| n.rect())
            .next()
            .expect("identity filter field")
            .right();
        assert!(
            filter_right <= checkbox_left + 1.0,
            "the identity filter field (right edge {filter_right:.0}px) overlaps the \
             Auto-store checkbox (left edge {checkbox_left:.0}px)",
        );
    }

    /// A History window sized to a wide screen, populated with long identities
    /// (they clip in the identity column), settled so the auto columns have
    /// measured their content.
    fn resize_harness() -> TestHarness<'static, HistoryHarness> {
        let long = "a/very/long/recording/identity/that/needs/lots/of/room/".repeat(2);
        let harness = history_harness(vec![
            entry_with_identity(&long),
            entry_with_identity(&format!("{long}/2")),
            entry_with_identity(&format!("{long}/3")),
        ]);
        let mut h = TestHarness::builder()
            .size(egui::vec2(1400.0, 600.0))
            .ui_state(show_history, harness);
        // Settle the sizing pass and let the window finish auto-positioning.
        for _ in 0..10 {
            h.step();
        }
        h
    }

    /// The rightmost content (the Delete button) relative to the window's right
    /// edge. Identity fills the leftover width, so this "gap" is only the
    /// window's frame padding - at every window size.
    fn content_gap_to_window_edge(h: &TestHarness<HistoryHarness>) -> f32 {
        let win = window_rect(h);
        let delete = h
            .inner
            .get_all_by_label("Delete")
            .last()
            .expect("delete button")
            .rect();
        win.right() - delete.right()
    }

    /// Identity fills the window at every size: the metadata columns keep their
    /// content width and identity takes the rest. Growing or shrinking the
    /// window leaves no gap on the right and traps no content off-screen - the
    /// table is always exactly as wide as the window.
    #[test]
    fn identity_fills_the_window_at_every_size() {
        let mut h = resize_harness();
        let settled_gap = content_gap_to_window_edge(&h);

        // Grow the window from its bottom-right corner.
        let before = window_rect(&h);
        drag(
            &mut h,
            egui::pos2(before.right() - 1.0, before.bottom() - 1.0),
            egui::vec2(300.0, 0.0),
            8,
        );
        for _ in 0..3 {
            h.step();
        }
        assert!(
            window_rect(&h).width() > before.width() + 200.0,
            "the window did not grow: {:.0}px -> {:.0}px",
            before.width(),
            window_rect(&h).width(),
        );
        assert!(
            (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
            "growing the window left a gap on the right - identity did not fill it",
        );

        // Shrink it back down.
        let grown = window_rect(&h);
        drag(
            &mut h,
            egui::pos2(grown.right() - 1.0, grown.bottom() - 1.0),
            egui::vec2(-400.0, 0.0),
            8,
        );
        for _ in 0..3 {
            h.step();
        }
        assert!(
            window_rect(&h).width() < grown.width() - 200.0,
            "the window did not shrink: {:.0}px -> {:.0}px",
            grown.width(),
            window_rect(&h).width(),
        );
        assert!(
            (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
            "shrinking the window left a gap on the right - identity did not fill it",
        );
    }

    fn window_rect(h: &TestHarness<HistoryHarness>) -> egui::Rect {
        h.inner
            .get_by_role_and_label(egui::accesskit::Role::Window, "History")
            .rect()
    }

    /// Press-drag-release the pointer from `from` by `delta` over `steps` frames.
    fn drag(h: &mut TestHarness<HistoryHarness>, from: egui::Pos2, delta: egui::Vec2, steps: u32) {
        h.inner.event(egui::Event::PointerMoved(from));
        h.step();
        h.inner.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        for i in 1..=steps {
            h.inner.event(egui::Event::PointerMoved(
                from + delta * (i as f32 / steps as f32),
            ));
            h.step();
        }
        h.inner.event(egui::Event::PointerButton {
            pos: from + delta,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }

    /// The window can be dragged narrower than its settled width. Identity
    /// yields as the window shrinks, so the table follows the window down
    /// instead of pinning it at a content minimum that snaps it back to full
    /// width (the old "can't shrink the window" bug).
    #[test]
    fn the_window_can_be_shrunk_narrower() {
        let mut h = resize_harness();
        let before = window_rect(&h);
        // Drag the bottom-right resize corner inward.
        let corner = egui::pos2(before.right() - 1.0, before.bottom() - 1.0);
        drag(&mut h, corner, egui::vec2(-200.0, 0.0), 8);
        for _ in 0..3 {
            h.step();
        }
        let after = window_rect(&h);
        assert!(
            after.width() < before.width() - 50.0,
            "the window did not shrink: {:.1}px -> {:.1}px",
            before.width(),
            after.width(),
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
