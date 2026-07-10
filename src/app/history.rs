use chrono::{DateTime, NaiveDate, Utc};
use gt_history::{DatabaseRef, PruneMode, RecordingEntry, RecordingMeta};
use gt_ui_theme::warning_amber;

use crate::app::history_db::{DeleteReason, HistoryManager};

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

    /// Show the Prune dialog. Sends preview/delete requests to `manager`. The
    /// results arrive asynchronously via [`HistoryWindow::set_prune_preview`].
    fn show(&mut self, ctx: &egui::Context, manager: &HistoryManager) {
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

        egui::Window::new("Prune History…")
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
                            ui.add(egui::DragValue::new(&mut self.age_days).range(1..=3650));
                            ui.label("days");
                        });
                        self.age_days != prev
                    }
                    PruneKind::TotalSize => {
                        let prev = self.size_limit_mb;
                        ui.horizontal(|ui| {
                            ui.label("Keep total size under");
                            ui.add(
                                egui::DragValue::new(&mut self.size_limit_mb).range(1..=100_000),
                            );
                            ui.label("MB");
                        });
                        self.size_limit_mb != prev
                    }
                    PruneKind::Count => {
                        let prev = self.keep_count;
                        ui.horizontal(|ui| {
                            ui.label("Keep at most");
                            ui.add(egui::DragValue::new(&mut self.keep_count).range(1..=10_000));
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
                        egui::ScrollArea::vertical()
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for r in refs {
                                    let label = format!("{}/{}", r.identity, r.group_name);
                                    ui.label(label);
                                }
                            });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let confirm_btn = ui
                                .button(
                                    egui::RichText::new("Delete these recordings")
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
            manager.prune_preview(self.to_prune_mode());
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
                manager.delete_recordings(refs, DeleteReason::Prune);
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

    /// Show the History window. All database work is sent to `manager`. Results
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
        manager: &HistoryManager,
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
        if self.entries.is_none() && !self.list_pending && manager.available() {
            manager.list();
            self.list_pending = true;
        }

        // Show Prune dialog (a separate window).
        self.prune.show(ctx, manager);

        // Hidden tracks live inside otherwise-visible recordings (there is no
        // recording-level hide). Count them across all recordings so the toolbar
        // can offer a "Delete hidden data" action that permanently drops them.
        let hidden_count: usize = self
            .entries
            .as_ref()
            .map(|entries| entries.iter().map(|e| e.hidden_tracks).sum())
            .unwrap_or_default();

        let mut open = self.open;

        // While the confirmation dialog is up, let Escape dismiss it rather than
        // the whole History window.
        if !self.confirm_delete_hidden
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            open = false;
        }

        egui::Window::new("History")
            .open(&mut open)
            .resizable(true)
            .default_width(640.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                if !manager.available() {
                    ui.label(
                        egui::RichText::new("History database is unavailable.")
                            .color(warning_amber(ui.visuals().dark_mode)),
                    );
                    return;
                }

                if let Some(err) = &self.error {
                    ui.label(egui::RichText::new(err).color(warning_amber(ui.visuals().dark_mode)));
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
                        egui::RichText::new("Identity"),
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
                            .add_enabled(hidden_count > 0, egui::Button::new(delete_hidden_label))
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
                        egui::TextEdit::singleline(&mut self.filter_min_points).desired_width(60.0),
                    );
                    ui.label("≤");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter_max_points).desired_width(60.0),
                    );
                    ui.separator();
                    ui.label("Date");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter_date_from)
                            .desired_width(90.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    ui.label("–");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter_date_to)
                            .desired_width(90.0)
                            .hint_text("YYYY-MM-DD"),
                    );
                    if filter_active && ui.small_button(egui_phosphor::regular::X).clicked() {
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
                        egui::Checkbox::new(auto_prune_enabled, "Auto-prune when over"),
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
                        egui::DragValue::new(&mut max_gb)
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
                        egui::Checkbox::new(auto_prune_confirm, "Confirm before pruning"),
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

                egui::ScrollArea::vertical()
                    .max_height(list_height)
                    .show(ui, |ui| {
                        egui::Grid::new("history_list")
                            .num_columns(6)
                            .striped(true)
                            .min_col_width(80.0)
                            .show(ui, |ui| {
                                crate::terms::term_label(
                                    ui,
                                    egui::RichText::new("Identity").strong(),
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
                                    render_row(ui, entry, already_loaded, manager);
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
                if let Some(path) = manager.path() {
                    ui.weak(path.display().to_string());
                }
            });

        // Confirmation for the destructive "delete hidden data" action, mirroring
        // the prune/auto-prune confirm flow (no permanent delete without a prompt).
        if self.confirm_delete_hidden {
            if hidden_count == 0 {
                self.confirm_delete_hidden = false;
            } else {
                let mut do_delete = false;
                let mut cancel =
                    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                egui::Window::new("Delete hidden data?")
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
                                    egui::RichText::new("Delete hidden tracks")
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
                    manager.delete_hidden_tracks();
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
    manager: &HistoryManager,
) {
    let identity = &entry.db_ref.identity;
    let (display_name, is_auto) = identity_display_parts(identity);

    ui.horizontal(|ui| {
        if is_auto {
            ui.label(
                egui::RichText::new("auto")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
        ui.label(display_name);
    })
    .response
    .on_hover_text(identity.as_str());

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
        let open = ui.add_enabled(!already_loaded, egui::Button::new("Open").small());
        if already_loaded {
            open.on_hover_text("Already loaded");
        } else if open.clicked() {
            manager.open(entry.db_ref.clone());
        }
        if ui.small_button("Delete").clicked() {
            manager.delete_recordings(vec![entry.db_ref.clone()], DeleteReason::Manual);
        }
    });

    ui.end_row();
}

fn identity_display_parts(identity: &str) -> (&str, bool) {
    match identity.strip_prefix("auto:") {
        Some(name) => (name, true),
        None => (identity, false),
    }
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
    use super::identity_display_parts;

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
