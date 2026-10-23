use egui::Color32;
use egui_plot::{Line, LineStyle, PlotPoint, PlotPoints, Points};

#[derive(Clone, Copy, Debug)]
pub(super) struct LineStroke {
    pub(super) color: Color32,
    pub(super) style: LineStyle,
    pub(super) width: f32,
    pub(super) highlighted: bool,
}

#[derive(Clone, Copy)]
pub(super) enum MetricHoverPolicy {
    Native,
    SourceAware,
}

/// This line-only entry point omits singleton samples.
/// Borrowed points must outlive the invariant `PlotUi` lifetime.
pub(super) fn add_line<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    data: &'a [PlotPoint],
    name: String,
    stroke: LineStroke,
) {
    if data.len() < 2 {
        return;
    }
    add_metric_run(plot_ui, data, name, stroke, MetricHoverPolicy::Native);
}

pub(super) fn add_metric_run<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    points: &'a [PlotPoint],
    name: String,
    stroke: LineStroke,
    hover: MetricHoverPolicy,
) {
    let allow_hover = matches!(hover, MetricHoverPolicy::Native);
    if points.is_empty() && allow_hover {
        return;
    }
    if points.len() == 1 {
        plot_ui.points(
            Points::new(name, PlotPoints::Borrowed(points))
                .color(stroke.color)
                .radius(ISOLATED_SAMPLE_RADIUS_PX)
                .highlight(stroke.highlighted)
                .allow_hover(allow_hover),
        );
    } else {
        plot_ui.line(
            Line::new(name, PlotPoints::Borrowed(points))
                .color(stroke.color)
                .style(stroke.style)
                .width(stroke.width)
                .highlight(stroke.highlighted)
                .allow_hover(allow_hover),
        );
    }
}

pub(super) const ANOMALY_MARKER_RADIUS: f32 = 4.0;
const ISOLATED_SAMPLE_RADIUS_PX: f32 = 3.0;
