use chrono::{DateTime, NaiveDate, Utc};
use gt_history::{DatabaseRef, HistoryDatabase, PruneMode, RecordingEntry, RecordingMeta};
use gt_ui_theme::WARNING_AMBER;

pub enum HistoryAction {
    Open(DatabaseRef),
    Delete(DatabaseRef),
    Prune(Vec<DatabaseRef>),
}

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
    /// Whether the user has confirmed and the prune should proceed.
    confirmed: bool,
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
            confirmed: false,
        }
    }

    fn reset(&mut self) {
        self.preview = None;
        self.confirmed = false;
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

    /// Show the Prune dialog.  Returns `Some(refs)` when the user confirms.
    fn show(
        &mut self,
        ctx: &egui::Context,
        db: Option<&gt_history::Database>,
    ) -> Option<Vec<DatabaseRef>> {
        if !self.open {
            return None;
        }

        let mut open = self.open;
        let mut do_prune = false;

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
                    self.preview = None;
                }

                ui.add_space(4.0);
                ui.separator();

                // Preview button / computed preview
                if self.preview.is_none() && ui.button("Preview").clicked() {
                    if let Some(db) = db {
                        match db.prune_candidates(&self.to_prune_mode()) {
                            Ok(refs) => self.preview = Some(refs),
                            Err(e) => log::error!("Prune preview failed: {e}"),
                        }
                    }
                } else if let Some(refs) = &self.preview {
                    if refs.is_empty() {
                        ui.label("Nothing to prune");
                    } else {
                        let n = refs.len();
                        let rec_label = if n == 1 { "recording" } else { "recordings" };
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
                                        .color(WARNING_AMBER),
                                )
                                .on_hover_text(
                                    "This cannot be undone. The original source files are unaffected.",
                                );
                            if confirm_btn.clicked() {
                                do_prune = true;
                            }
                            if ui.button("Cancel").clicked() {
                                self.reset();
                            }
                        });
                    }
                }
            });

        self.open = open;

        if do_prune {
            let refs = self.preview.take().unwrap_or_default();
            self.open = false;
            return Some(refs);
        }

        None
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
        }
    }

    fn any_filter_active(&self) -> bool {
        !self.filter_text.is_empty()
            || !self.filter_min_points.is_empty()
            || !self.filter_max_points.is_empty()
            || !self.filter_date_from.is_empty()
            || !self.filter_date_to.is_empty()
    }

    /// Call after a delete or successful open to force a list refresh.
    pub fn invalidate(&mut self) {
        self.entries = None;
    }

    /// Show the History window and return any action the user triggered.
    ///
    /// `db` is `None` if the database failed to open at startup.
    /// `loaded_metas` are the content fingerprints of the files currently loaded
    /// in the app, used to disable re-opening a recording that is already open.
    #[expect(
        clippy::too_many_arguments,
        reason = "the window drives several independent pieces of persisted app state plus the loaded-file set; bundling them would obscure rather than clarify"
    )]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        db: Option<&gt_history::Database>,
        loaded_metas: &[RecordingMeta],
        storage_enabled: &mut bool,
        auto_prune_enabled: &mut bool,
        auto_prune_max_bytes: &mut u64,
        auto_prune_confirm: &mut bool,
    ) -> Option<HistoryAction> {
        if !self.open {
            return None;
        }

        if let (None, Some(db)) = (&self.entries, db) {
            match db.list_recordings() {
                Ok(entries) => self.entries = Some(entries),
                Err(e) => self.error = Some(format!("Failed to load history: {e}")),
            }
        }

        // Show Prune dialog (a separate window).
        if let Some(refs) = self.prune.show(ctx, db)
            && !refs.is_empty()
        {
            self.invalidate();
            return Some(HistoryAction::Prune(refs));
        }

        let mut action = None;
        let mut open = self.open;

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            open = false;
        }

        egui::Window::new("History")
            .open(&mut open)
            .resizable(true)
            .default_width(640.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                if db.is_none() {
                    ui.label(
                        egui::RichText::new("History database is unavailable.")
                            .color(WARNING_AMBER),
                    );
                    return;
                }

                if let Some(err) = &self.error {
                    ui.label(egui::RichText::new(err).color(WARNING_AMBER));
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
                // layout stays stable; controls are grayed out when inactive,
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
                                    render_row(ui, entry, already_loaded, &mut action);
                                }
                            });
                    });

                ui.separator();
                let total_count = entries.len();
                let total_size: u64 = entries.iter().map(|e| e.meta.gtd_size_bytes).sum();
                ui.horizontal(|ui| {
                    let rec_label = if total_count == 1 {
                        "recording"
                    } else {
                        "recordings"
                    };
                    ui.label(format!(
                        "{total_count} {rec_label} - {}",
                        format_size(total_size)
                    ));
                    if filter_active && visible.len() != total_count {
                        ui.weak(format!("({} shown)", visible.len()));
                    }
                });
                if let Some(path) = db.map(|d| d.path().display().to_string()) {
                    ui.weak(path);
                }
            });

        self.open = open;
        action
    }
}

fn render_row(
    ui: &mut egui::Ui,
    entry: &RecordingEntry,
    already_loaded: bool,
    action: &mut Option<HistoryAction>,
) {
    let identity = &entry.db_ref.identity;
    let (display_name, is_auto) = match identity.strip_prefix("auto:") {
        Some(name) => (name, true),
        None => (identity.as_str(), false),
    };

    ui.horizontal(|ui| {
        if is_auto {
            ui.label(
                egui::RichText::new("auto")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
        ui.label(display_name);
    });

    let ts = DateTime::<Utc>::from_timestamp_micros(entry.meta.start_us)
        .unwrap_or_default()
        .format("%Y-%m-%d %H:%M")
        .to_string();
    ui.label(&ts);

    let dur_us = entry.meta.end_us.saturating_sub(entry.meta.start_us).max(0);
    let dur = chrono::Duration::microseconds(dur_us);
    ui.label(format_duration(dur));

    let count = gt_history::format_count_suffix(entry.meta.nav_point_count);
    ui.label(count);

    ui.label(format_size(entry.meta.gtd_size_bytes));

    ui.horizontal(|ui| {
        let open = ui.add_enabled(!already_loaded, egui::Button::new("Open").small());
        if already_loaded {
            open.on_hover_text("Already loaded");
        } else if open.clicked() {
            *action = Some(HistoryAction::Open(entry.db_ref.clone()));
        }
        if ui.small_button("Delete").clicked() {
            *action = Some(HistoryAction::Delete(entry.db_ref.clone()));
        }
    });

    ui.end_row();
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
