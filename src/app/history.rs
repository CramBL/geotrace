use chrono::{DateTime, Utc};
use gt_db::{DatabaseRef, RecordingEntry};
use gt_ui_theme::WARNING_AMBER;

pub enum HistoryAction {
    Open(DatabaseRef),
    Delete(DatabaseRef),
}

pub struct HistoryWindow {
    pub open: bool,
    /// Cached recording list — `None` until the window is first shown.
    entries: Option<Vec<RecordingEntry>>,
    /// Error from the last operation, if any.
    error: Option<String>,
}

impl HistoryWindow {
    pub fn new() -> Self {
        Self {
            open: false,
            entries: None,
            error: None,
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

                if entries.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label("No recordings in history yet.");
                    });
                    return;
                }

                let available = ui.available_size();
                egui::ScrollArea::vertical()
                    .max_height(available.y - 4.0)
                    .show(ui, |ui| {
                        egui::Grid::new("history_list")
                            .num_columns(5)
                            .striped(true)
                            .min_col_width(80.0)
                            .show(ui, |ui| {
                                // Header row
                                ui.strong("Identity");
                                ui.strong("Date");
                                ui.strong("Duration");
                                ui.strong("Points");
                                ui.strong("Size");
                                ui.label(""); // action column
                                ui.end_row();

                                for entry in entries {
                                    render_row(ui, entry, &mut action);
                                }
                            });
                    });
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

    // Identity cell — name with optional "auto" badge
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

    // Start timestamp
    let ts = DateTime::<Utc>::from_timestamp_micros(entry.meta.start_us)
        .unwrap_or_default()
        .format("%Y-%m-%d %H:%M")
        .to_string();
    ui.label(&ts);

    // Duration
    let dur_us = entry.meta.end_us.saturating_sub(entry.meta.start_us).max(0);
    let dur = chrono::Duration::microseconds(dur_us);
    ui.label(format_duration(dur));

    // Nav point count
    let count = gt_db::format_count_suffix(entry.meta.nav_point_count);
    ui.label(count);

    // Size
    ui.label(format_size(entry.meta.nvd_size_bytes));

    // Action buttons
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
