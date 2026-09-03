//! The panel the parse summary expands into: the derived figures, the boot
//! sessions as a timeline, what the exporter's summary block stated, and the
//! order anomalies the parse found.

use chrono::Duration;
use egui::{Button, Frame, Grid, Label, RichText, ScrollArea};
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

const FILTERED_OUT_HOVER: &str = "The filters show no line of this";

/// Column and row spacing shared by the panel's grids, so their rows sit
/// tighter than the window's default.
const GRID_SPACING: egui::Vec2 = egui::vec2(8.0, 2.0);

impl LogViewerWindow {
    pub(super) fn summary_panel_ui(&mut self, ui: &mut egui::Ui, log: &LoadedLog) {
        let parsed = log.parsed();
        let rows = LineTableRows::of(log);
        Frame::group(ui.style()).show(ui, |ui| {
            ScrollArea::vertical()
                .id_salt("log_viewer_summary_panel")
                .max_height(MAX_PANEL_HEIGHT_PX)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    parse_figures_ui(ui, parsed);
                    self.boot_sessions_ui(ui, parsed, &rows);
                    service_summary_ui(ui, parsed);
                    self.order_anomalies_ui(ui, parsed, &rows);
                });
        });
    }

    /// One row per boot session, with a bar proportional to its uptime.
    fn boot_sessions_ui(
        &mut self,
        ui: &mut egui::Ui,
        parsed: &ParsedLog,
        rows: &LineTableRows<'_>,
    ) {
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
                    let boot_divider_row = rows.row_of_boot_divider(session_index);
                    if ui
                        .add_enabled(
                            boot_divider_row.is_some(),
                            Button::new(format!("Boot {}", session.boot_number)),
                        )
                        .on_hover_text("Scroll the table to this boot session")
                        .on_disabled_hover_text(FILTERED_OUT_HOVER)
                        .clicked()
                    {
                        self.scroll_to_row = boot_divider_row;
                    }
                    let uptime = session.uptime();
                    ui.add(
                        Label::new(session.anchored.map_or_else(
                            || EM_DASH.to_owned(),
                            |anchored| anchored.first.format(TIMESTAMP_FORMAT).to_string(),
                        ))
                        .selectable(true),
                    );
                    ui.add(
                        Label::new(uptime.map_or_else(
                            || EM_DASH.to_owned(),
                            gt_fmt::format_human_terse_duration,
                        ))
                        .selectable(true),
                    );
                    let entries = session.entry_count();
                    ui.add(
                        Label::new(format!(
                            "{} {}",
                            gt_fmt::format_count(entries),
                            gt_fmt::pluralize(entries, "entry", "entries")
                        ))
                        .selectable(true),
                    );
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
        rows: &LineTableRows<'_>,
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
                    let entry_index = parsed
                        .entries()
                        .partition_point(|entry| entry.line_number < anomaly.line_number);
                    let anomaly_row = rows.row_of_entry(entry_index);
                    if ui
                        .add_enabled(
                            anomaly_row.is_some(),
                            Button::new(format!(
                                "Line {}",
                                gt_fmt::format_count_u64(u64::from(anomaly.line_number))
                            )),
                        )
                        .on_hover_text("Scroll the table to this line")
                        .on_disabled_hover_text(FILTERED_OUT_HOVER)
                        .clicked()
                    {
                        self.scroll_to_row = anomaly_row;
                    }
                    ui.add(
                        Label::new(
                            RichText::new(format!(
                                "steps back {}",
                                gt_fmt::format_human_terse_duration(anomaly.timestamp_step.abs())
                            ))
                            .color(amber),
                        )
                        .selectable(true),
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
            ui.add(Label::new(parsed.format().display_name()).selectable(true));
            ui.end_row();

            ui.label("Time span");
            ui.add(
                Label::new(parsed.time_range().map_or_else(
                    || EM_DASH.to_owned(),
                    |range| gt_fmt::format_time_range(range.start, range.end),
                ))
                .selectable(true),
            );
            ui.end_row();

            ui.label("Anchored entries");
            ui.add(
                Label::new(gt_fmt::format_count(parsed.anchored_entry_count())).selectable(true),
            );
            ui.end_row();

            ui.label("Interpolated entries");
            ui.add(
                Label::new(gt_fmt::format_count(parsed.interpolated_entry_count()))
                    .selectable(true),
            );
            ui.end_row();

            ui.label("Structural lines");
            ui.add(
                Label::new(gt_fmt::format_count(parsed.structural_lines().len())).selectable(true),
            );
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
            ui.add(Label::new(RichText::new(device_type).strong()).selectable(true));
        });
    }
    if let Some(mismatch) = parsed.exporter_entry_count_mismatch() {
        ui.add(
            Label::new(
                RichText::new(format!(
                    "The exporter counted {} entries, this parse anchored {}",
                    gt_fmt::format_count_u64(mismatch.stated_by_exporter),
                    gt_fmt::format_count_u64(mismatch.anchored_by_parse)
                ))
                .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
            )
            .selectable(true),
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
                ui.add(Label::new(row.service).selectable(true));
                ui.add(Label::new(gt_fmt::format_count_u64(row.errors)).selectable(true));
                ui.add(Label::new(gt_fmt::format_count_u64(row.warnings)).selectable(true));
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
