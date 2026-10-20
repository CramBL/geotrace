use chrono::Duration;
use egui::{Align, Grid, Label, Layout, RichText, Sense, Stroke, StrokeKind};
use gt_fmt::NavigationSummary;
use gt_types::{FixStats, SolutionState};
use gt_ui_theme::NavigationStatus;

pub(super) struct NavigationStatistics(pub FixStats);

impl NavigationStatistics {
    pub(super) fn ui(self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(SUMMARY_MAX_WIDTH_PX));
            let summary = NavigationSummary(self.0);
            let color = summary.availability_percentage().map_or_else(
                || ui.visuals().weak_text_color(),
                |percent| gt_ui_theme::fix_quality_color(percent, ui.visuals().dark_mode),
            );
            let stacked = ui.available_width() < HEADLINE_INLINE_WIDTH_PX;
            let font = egui::TextStyle::Body.resolve(ui.style());
            let label_width = super::text_width(ui, "Availability", &font);
            Grid::new("navigation_statistics_headline")
                .num_columns(if stacked { 2 } else { 3 })
                .show(ui, |ui| {
                    for (label, value, hint, description, value_color) in [
                        (
                            "Availability",
                            summary.availability_value(),
                            "Fix / known",
                            "Fix duration divided by the sum of Fix, No fix and Estimated only durations",
                            color,
                        ),
                        (
                            "Coverage",
                            summary.coverage_value(),
                            "Known / total",
                            "Known duration divided by total estimated interval duration",
                            ui.visuals().text_color(),
                        ),
                    ] {
                        ui.vertical(|ui| {
                            ui.set_width(label_width);
                            ui.add(Label::new(label).extend());
                            if stacked {
                                ui.add(Label::new(RichText::new(hint).small()).extend());
                            }
                        });
                        let response = ui.colored_label(
                            if value == gt_ui_theme::EM_DASH {
                                ui.visuals().weak_text_color()
                            } else {
                                value_color
                            },
                            RichText::new(&value).strong(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Label,
                                true,
                                format!("{label} {value}. {description}."),
                            )
                        });
                        if !stacked {
                            ui.add(Label::new(RichText::new(hint).small()).extend());
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(SECTION_GAP_PX);
            self.composition_ui(ui);
            if self.0.total_duration() <= Duration::zero() {
                ui.weak("No positive navigation intervals");
                return;
            }
            Grid::new("navigation_statistics_legend")
                .num_columns(2)
                .show(ui, |ui| {
                    for (state, duration) in summary
                        .duration_buckets()
                        .into_iter()
                        .filter(|(_, duration)| *duration > Duration::zero())
                    {
                        ui.horizontal(|ui| {
                            NavigationStatus {
                                state,
                                provenance: None,
                            }
                            .ui(ui);
                            ui.label(state.to_string());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(gt_fmt::format_human_terse_duration(duration));
                        });
                        ui.end_row();
                    }
                    ui.label("Losses");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(self.0.fix_loss_count.to_string()).widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Losses {}", self.0.fix_loss_count))
                        });
                    });
                    ui.end_row();
                    ui.label("Longest");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let duration =
                            gt_fmt::format_human_terse_duration(self.0.max_continuous_no_fix);
                        ui.label(&duration).widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Label,
                                true,
                                format!("Longest {duration}. Continuous known No fix or Estimated only duration."),
                            )
                        });
                    });
                    ui.end_row();
                });
        });
    }

    fn composition_ui(&self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), COMPOSITION_HEIGHT_PX),
            Sense::hover(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                "Navigation duration composition by state. Segments group intervals by state.",
            )
        });
        let total = self.0.total_duration().as_seconds_f64();
        if total <= 0.0 {
            ui.painter().rect_stroke(
                rect,
                0.0,
                ui.visuals().widgets.noninteractive.bg_stroke,
                StrokeKind::Inside,
            );
            return;
        }
        let mut fraction = 0.0;
        for (state, duration) in NavigationSummary(self.0).duration_buckets() {
            if duration <= Duration::zero() {
                continue;
            }
            let start = fraction;
            fraction += duration.as_seconds_f64() / total;
            let segment = egui::Rect::from_x_y_ranges(
                rect.left() + rect.width() * start as f32
                    ..=rect.left() + rect.width() * fraction as f32,
                rect.y_range(),
            );
            let color = NavigationStatus {
                state,
                provenance: None,
            }
            .color(ui.visuals().dark_mode);
            ui.painter().rect_filled(segment, 0.0, color);
            if state == SolutionState::Unknown {
                let painter = ui
                    .painter()
                    .with_clip_rect(segment.intersect(ui.clip_rect()));
                let pattern_color = ui.visuals().extreme_bg_color;
                let mut x = segment.left() - segment.height();
                while x < segment.right() {
                    painter.line_segment(
                        [
                            egui::pos2(x, segment.bottom()),
                            egui::pos2(x + segment.height(), segment.top()),
                        ],
                        Stroke::new(UNKNOWN_PATTERN_WIDTH_PX, pattern_color),
                    );
                    x += UNKNOWN_PATTERN_SPACING_PX;
                }
                painter.rect_stroke(
                    segment,
                    0.0,
                    Stroke::new(UNKNOWN_OUTLINE_WIDTH_PX, pattern_color),
                    StrokeKind::Inside,
                );
            }
            ui.interact(segment, response.id.with(state.to_string()), Sense::hover())
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Label,
                        true,
                        format!(
                            "{} duration {} ({})",
                            state,
                            gt_fmt::format_human_terse_duration(duration),
                            gt_fmt::format_fraction_percent(duration.as_seconds_f64() / total),
                        ),
                    )
                });
        }
    }
}

const SUMMARY_MAX_WIDTH_PX: f32 = 300.0;
const HEADLINE_INLINE_WIDTH_PX: f32 = 260.0;
const COMPOSITION_HEIGHT_PX: f32 = 12.0;
const SECTION_GAP_PX: f32 = 4.0;
const UNKNOWN_PATTERN_SPACING_PX: f32 = 6.0;
const UNKNOWN_PATTERN_WIDTH_PX: f32 = 1.0;
const UNKNOWN_OUTLINE_WIDTH_PX: f32 = 1.0;
