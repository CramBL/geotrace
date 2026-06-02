use chrono::{DateTime, Utc};
use gt_db::{DatabaseRef, PruneMode, RecordingEntry};
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
        db: Option<&gt_db::Database>,
    ) -> Option<Vec<DatabaseRef>> {
        if !self.open {
            return None;
        }

        let mut open = self.open;
        let mut do_prune = false;

        egui::Window::new("Prune History…")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
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
                            ui.label("recording(s) per identity");
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
                        ui.label(format!("{} recording(s) will be deleted", refs.len()));
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
    /// Cached recording list — `None` until the window is first shown.
    entries: Option<Vec<RecordingEntry>>,
    /// Identity filter text — applied client-side against the cached list.
    filter_text: String,
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
            error: None,
            prune: PruneDialog::new(),
        }
    }

    /// Call after a delete or successful open to force a list refresh.
    pub fn invalidate(&mut self) {
        self.entries = None;
    }

    /// Show the History window and return any action the user triggered.
    ///
    /// `db` is `None` if the database failed to open at startup.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        db: Option<&gt_db::Database>,
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

                let Some(entries) = &self.entries else {
                    ui.spinner();
                    return;
                };

                // Toolbar row: filter bar + Prune button
                ui.horizontal(|ui| {
                    ui.label("Filter");
                    ui.text_edit_singleline(&mut self.filter_text);
                    if !self.filter_text.is_empty()
                        && ui.small_button(egui_phosphor::regular::X).clicked()
                    {
                        self.filter_text.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Prune…").clicked() {
                            self.prune.open = true;
                            self.prune.reset();
                        }
                        ui.checkbox(storage_enabled, "Auto-store recordings");
                    });
                });

                // Auto-prune settings row (only relevant when storage is enabled)
                if *storage_enabled {
                    ui.horizontal(|ui| {
                        ui.checkbox(auto_prune_enabled, "Auto-prune when over");
                        if *auto_prune_enabled {
                            let mut max_gb =
                                *auto_prune_max_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                            ui.add(
                                egui::DragValue::new(&mut max_gb)
                                    .range(0.1..=1_000.0)
                                    .speed(0.1),
                            );
                            ui.label("GB");
                            #[expect(
                                clippy::cast_sign_loss,
                                reason = "DragValue range is 0.1..=1000 so value is always positive"
                            )]
                            let bytes = (max_gb * 1024.0 * 1024.0 * 1024.0).round() as u64;
                            *auto_prune_max_bytes = bytes;
                            ui.separator();
                            ui.checkbox(auto_prune_confirm, "Confirm before pruning");
                        }
                    });
                }
                ui.add_space(4.0);

                let filter = self.filter_text.to_lowercase();
                let visible: Vec<&RecordingEntry> = entries
                    .iter()
                    .filter(|e| {
                        filter.is_empty()
                            || e.db_ref.identity.to_lowercase().contains(filter.as_str())
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
                                ui.strong("Identity");
                                ui.strong("Date");
                                ui.strong("Duration");
                                ui.strong("Points");
                                ui.strong("Size");
                                ui.label("");
                                ui.end_row();

                                for entry in &visible {
                                    render_row(ui, entry, &mut action);
                                }
                            });
                    });

                ui.separator();
                let total_count = entries.len();
                let total_size: u64 = entries.iter().map(|e| e.meta.nvd_size_bytes).sum();
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{total_count} recording(s) — {}",
                        format_size(total_size)
                    ));
                    if !filter.is_empty() && visible.len() != total_count {
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

fn render_row(ui: &mut egui::Ui, entry: &RecordingEntry, action: &mut Option<HistoryAction>) {
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

    let count = gt_db::format_count_suffix(entry.meta.nav_point_count);
    ui.label(count);

    ui.label(format_size(entry.meta.nvd_size_bytes));

    ui.horizontal(|ui| {
        if ui.small_button("Open").clicked() {
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
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if h > 0 {
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
