//! The panel the parse summary expands into: the derived figures, the boot
//! sessions as a timeline, what the exporter's summary block stated, and the
//! order anomalies the parse found.

use chrono::Duration;
use egui::{Frame, Grid, RichText, ScrollArea};
use gt_log_view::LoadedLog;
use gt_logfile::{BootSession, ParsedLog};
use gt_ui_theme::EM_DASH;

use super::line_table::LineTableRows;
use super::{LogViewerWindow, TIMESTAMP_FORMAT};

/// Height the panel scrolls past: a log of many boots must still leave the
/// table its share of the window.
const MAX_PANEL_HEIGHT_PX: f32 = 260.0;

/// Width of the column the per-boot uptime bars are drawn across.
const UPTIME_BAR_WIDTH_PX: f32 = 90.0;

const UPTIME_BAR_HEIGHT_PX: f32 = 6.0;

/// Column and row spacing shared by the panel's grids, so their rows sit
/// tighter than the window's default.
const GRID_SPACING: egui::Vec2 = egui::vec2(8.0, 2.0);

impl LogViewerWindow {
    pub(super) fn summary_panel_ui(&mut self, ui: &mut egui::Ui, log: &LoadedLog) {
        let parsed = log.parsed();
        let rows = LineTableRows::of(parsed);
        Frame::group(ui.style()).show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("log_viewer_summary_panel")
                .max_height(MAX_PANEL_HEIGHT_PX)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    parse_figures_ui(ui, parsed);
                    self.boot_sessions_ui(ui, parsed, rows);
                    service_summary_ui(ui, parsed);
                    self.order_anomalies_ui(ui, parsed, rows);
                });
        });
    }

    /// One row per boot session, with a bar proportional to its uptime: the
    /// boot history reads as a timeline.
    fn boot_sessions_ui(&mut self, ui: &mut egui::Ui, parsed: &ParsedLog, rows: LineTableRows<'_>) {
        let longest_uptime = parsed
            .boot_sessions()
            .iter()
            .filter_map(BootSession::uptime)
            .max();

        ui.add_space(4.0);
        ui.strong("Boots");
        Grid::new("log_viewer_boot_sessions")
            .num_columns(5)
            .spacing(GRID_SPACING)
            .show(ui, |ui| {
                for (session_index, session) in parsed.boot_sessions().iter().enumerate() {
                    if ui
                        .button(format!("Boot {}", session.boot_number))
                        .on_hover_text("Scroll the table to this boot session")
                        .clicked()
                    {
                        self.scroll_to_row = rows.row_of_boot_separator(session_index);
                    }
                    let uptime = session.uptime();
                    ui.label(session.anchored.map_or_else(
                        || EM_DASH.to_owned(),
                        |anchored| anchored.first.format(TIMESTAMP_FORMAT).to_string(),
                    ));
                    ui.label(
                        uptime.map_or_else(
                            || EM_DASH.to_owned(),
                            gt_fmt::format_human_terse_duration,
                        ),
                    );
                    let entries = session.entry_count();
                    ui.label(format!(
                        "{} {}",
                        gt_fmt::format_count(entries),
                        gt_fmt::pluralize(entries, "entry", "entries")
                    ));
                    UptimeBar {
                        session: uptime,
                        longest_in_log: longest_uptime,
                    }
                    .ui(ui);
                    ui.end_row();
                }
            });
    }

    /// The backwards timestamp steps no logged clock adjustment explains. Drawn
    /// only when the parse found any: a clean log says nothing here.
    fn order_anomalies_ui(
        &mut self,
        ui: &mut egui::Ui,
        parsed: &ParsedLog,
        rows: LineTableRows<'_>,
    ) {
        if parsed.order_anomalies().is_empty() {
            return;
        }
        let amber = gt_ui_theme::warning_amber(ui.visuals().dark_mode);
        ui.add_space(4.0);
        ui.label(RichText::new("Order anomalies").strong().color(amber));
        Grid::new("log_viewer_order_anomalies")
            .num_columns(2)
            .spacing(GRID_SPACING)
            .show(ui, |ui| {
                for anomaly in parsed.order_anomalies() {
                    if ui
                        .button(format!(
                            "Line {}",
                            gt_fmt::format_count_u64(u64::from(anomaly.line_number))
                        ))
                        .on_hover_text("Scroll the table to this line")
                        .clicked()
                    {
                        let entry_index = parsed
                            .entries()
                            .partition_point(|entry| entry.line_number < anomaly.line_number);
                        self.scroll_to_row = rows.row_of_entry(entry_index);
                    }
                    ui.label(
                        RichText::new(format!(
                            "steps back {}",
                            gt_fmt::format_human_terse_duration(anomaly.timestamp_step.abs())
                        ))
                        .color(amber),
                    );
                    ui.end_row();
                }
            });
    }
}

/// The figures the parse derived: the format it read the log in, the span its
/// entries cover, and how each line was kept.
fn parse_figures_ui(ui: &mut egui::Ui, parsed: &ParsedLog) {
    Grid::new("log_viewer_parse_figures")
        .num_columns(2)
        .spacing(GRID_SPACING)
        .show(ui, |ui| {
            ui.label("Format");
            ui.label(parsed.format().display_name());
            ui.end_row();

            ui.label("Time span");
            ui.label(parsed.time_range().map_or_else(
                || EM_DASH.to_owned(),
                |range| gt_fmt::format_time_range(range.start, range.end),
            ));
            ui.end_row();

            ui.label("Anchored entries");
            ui.label(gt_fmt::format_count(parsed.anchored_entry_count()));
            ui.end_row();

            ui.label("Interpolated entries");
            ui.label(gt_fmt::format_count(parsed.interpolated_entry_count()));
            ui.end_row();

            ui.label("Structural lines");
            ui.label(gt_fmt::format_count(parsed.structural_lines().len()));
            ui.end_row();
        });
}

/// What the exporter's own summary block stated, shown only for a log that
/// carried one.
fn service_summary_ui(ui: &mut egui::Ui, parsed: &ParsedLog) {
    let Some(summary) = parsed.summary_block() else {
        return;
    };
    ui.add_space(4.0);
    ui.strong("Service summary");
    if let Some(device_type) = &summary.device_type {
        ui.horizontal(|ui| {
            ui.label("Device type");
            ui.label(RichText::new(device_type).strong());
        });
    }
    if let Some(mismatch) = parsed.exporter_entry_count_mismatch() {
        ui.label(
            RichText::new(format!(
                "The exporter counted {} entries, this parse anchored {}",
                gt_fmt::format_count_u64(mismatch.stated_by_exporter),
                gt_fmt::format_count_u64(mismatch.anchored_by_parse)
            ))
            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
        );
    }

    let counts = summary.service_counts_by_errors_descending();
    if counts.is_empty() {
        return;
    }
    Grid::new("log_viewer_service_counts")
        .num_columns(3)
        .spacing(GRID_SPACING)
        .show(ui, |ui| {
            ui.label(RichText::new("Service").weak());
            ui.label(RichText::new("Errors").weak());
            ui.label(RichText::new("Warnings").weak());
            ui.end_row();
            for row in counts {
                ui.label(row.service);
                ui.label(gt_fmt::format_count_u64(row.errors));
                ui.label(gt_fmt::format_count_u64(row.warnings));
                ui.end_row();
            }
        });
}

/// One boot session's uptime set against the longest uptime in the log, so the
/// bars read as one timeline down the column.
struct UptimeBar {
    session: Option<Duration>,
    longest_in_log: Option<Duration>,
}

impl UptimeBar {
    fn ui(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(UPTIME_BAR_WIDTH_PX, UPTIME_BAR_HEIGHT_PX),
            egui::Sense::hover(),
        );
        let (Some(session), Some(longest)) = (self.session, self.longest_in_log) else {
            return;
        };
        let longest_secs = longest.num_seconds();
        if longest_secs <= 0 {
            return;
        }
        let fraction = session.num_seconds() as f32 / longest_secs as f32;
        let bar = egui::Rect::from_min_size(
            rect.left_top(),
            egui::vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
        );
        ui.painter()
            .rect_filled(bar, 1.0, ui.visuals().selection.bg_fill);
    }
}
