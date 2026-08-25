//! Renders the plot illustration of the TEC reference material into
//! `gt-ionex`'s assets.
//!
//! The document belongs to the crate holding its data, which cannot draw it:
//! this crate projects it, as `gt-map` projects the map illustrations.
//! `just generate-reference-tec-plot` runs the ignored test below.

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use egui_plot::{GridMark, Legend, LineStyle, PlotBounds, PlotPoint};
use gt_ionex::node_series::NodeSeriesCapture;
use gt_ionex::quiet_time;
use gt_types::MetricKind;
use gt_ui_types::{ContextLines, TecContextSample};

use super::chips::{MetricAvailability, MetricVisibility};
use super::context::{ContextLineGates, ContextPlotCaches, add_context_lines};
use super::levels::LineViewport;
use super::lines::{LineStroke, NearestHoverLabel, add_line};
use super::style::metric_line_color;

/// Where the rendered plot lands, resolved from this crate's manifest dir.
const ASSET_PATH: &str = "../gt-ionex/assets/tec_plot_2024_05_gannon_storm.png";

/// The same width as the map illustration beside it in the document, at the
/// height the plot pane opens with.
const CANVAS_SIZE: egui::Vec2 = egui::vec2(1024.0, 400.0);

/// First and last day drawn: the quiet days before the storm, its main phase
/// late on 10 May, and the recovery.
const DRAWN_DAYS: (NaiveDate, NaiveDate) = (
    match NaiveDate::from_ymd_opt(2024, 5, 6) {
        Some(day) => day,
        None => panic!("a calendar date"),
    },
    match NaiveDate::from_ymd_opt(2024, 5, 12) {
        Some(day) => day,
        None => panic!("a calendar date"),
    },
);

/// The node drawn: it carries both phases of the storm, the enhancement late
/// on 10 May and the depletion through 11 May.
const DRAWN_NODE: &str = gt_ionex::NORTH_AMERICA_NODE;

const TEC_LINE_WIDTH: f32 = 1.5;

/// Empty span left either side of the drawn days, so the axis labels of the
/// first and last day are not cut off by the edge of the canvas.
const X_MARGIN: TimeDelta = TimeDelta::hours(6);

/// Headroom above the highest value drawn, so the storm's peak is not against
/// the top edge.
const Y_HEADROOM: f64 = 1.15;

/// Space around the plot inside the canvas, which the axis labels sit in.
const CANVAS_MARGIN_PX: i8 = 6;

/// How far the median line's colour is dimmed from the metric's own, so the
/// quiet reference reads as the background the storm departs from.
const MEDIAN_DIM: f32 = 0.55;

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// The samples the context line is drawn from, one per published epoch of the
/// drawn days.
fn drawn_samples(capture: &NodeSeriesCapture) -> Vec<TecContextSample> {
    let (first, last) = DRAWN_DAYS;
    let end = midnight(last) + TimeDelta::days(1);
    capture
        .samples(DRAWN_NODE)
        .into_iter()
        .filter(|sample| sample.epoch >= midnight(first) && sample.epoch <= end)
        .map(|sample| TecContextSample {
            x_secs: sample.epoch.timestamp() as f64,
            tecu: sample.tecu,
        })
        .collect()
}

/// The quiet-time median behind each drawn epoch: the median of the same node
/// and time of day over the 27 days before that epoch's own day, which is the
/// reference the storm index measures against.
fn quiet_time_median_points(capture: &NodeSeriesCapture) -> Vec<PlotPoint> {
    let (first, last) = DRAWN_DAYS;
    let mut points = Vec::new();
    let mut day = first;
    while day <= last {
        let Some(captured) = capture.day(day) else {
            break;
        };
        for offset in captured.epoch_offsets() {
            let window = capture.background_window(DRAWN_NODE, day, offset);
            if let Some(median) = quiet_time::quiet_time_median(&window) {
                let epoch = midnight(day) + offset;
                points.push(PlotPoint::new(epoch.timestamp() as f64, median.tecu()));
            }
        }
        let Some(next) = day.succ_opt() else {
            break;
        };
        day = next;
    }
    points
}

/// Writes the plot illustration the TEC reference material shows. Ignored so
/// it runs only when the asset is regenerated, which the just recipe does.
#[test]
#[ignore = "writes a committed asset"]
fn generate_tec_reference_plot() {
    let capture = gt_ionex::captured_node_series().expect("the node-series capture");
    let samples = Arc::new(drawn_samples(&capture));
    let median = quiet_time_median_points(&capture);
    let (first, last) = DRAWN_DAYS;
    let x_min = (midnight(first) - X_MARGIN).timestamp() as f64;
    let x_max = (midnight(last) + TimeDelta::days(1) + X_MARGIN).timestamp() as f64;
    let y_max = samples
        .iter()
        .filter_map(|sample| sample.tecu)
        .chain(median.iter().map(|point| point.y))
        .fold(0.0_f64, f64::max)
        * Y_HEADROOM;

    let mut caches = ContextPlotCaches::default();
    caches.sync(&ContextLines {
        tec: Arc::clone(&samples),
        ..ContextLines::default()
    });
    let mut metric_vis = MetricVisibility::default();
    metric_vis.set(MetricKind::Tec, true);
    let available = MetricAvailability {
        snap_error: false,
        jamming: false,
        hp30: false,
        kp: false,
        tec: true,
    };

    let mut harness = gt_test_utils::TestHarness::builder()
        .size(CANVAS_SIZE)
        .theme(true)
        .ui(move |ui| {
            let dark_mode = ui.visuals().dark_mode;
            egui::Frame::NONE
                .inner_margin(egui::Margin::same(CANVAS_MARGIN_PX))
                .show(ui, |ui| {
                    egui_plot::Plot::new("tec_reference_plot")
                        .legend(Legend::default())
                        .y_axis_label(gt_ionex::text::LEGEND_UNIT)
                        .x_grid_spacer(|input| day_and_six_hour_marks(input.bounds))
                        .x_axis_formatter(|mark, _range| day_or_hour_label(mark.value))
                        .show(ui, |plot_ui| {
                            plot_ui.set_plot_bounds(PlotBounds::from_min_max(
                                [x_min, 0.0],
                                [x_max, y_max],
                            ));
                            add_line(
                                plot_ui,
                                &median,
                                MEDIAN_LINE_LABEL.to_owned(),
                                LineStroke {
                                    color: metric_line_color(MetricKind::Tec, 0, dark_mode)
                                        .gamma_multiply(MEDIAN_DIM),
                                    style: LineStyle::Dashed { length: 6.0 },
                                    width: TEC_LINE_WIDTH,
                                    highlighted: false,
                                },
                            );
                            add_context_lines(
                                plot_ui,
                                &caches,
                                ContextLineGates {
                                    metric_vis: &metric_vis,
                                    available,
                                },
                                |kind| LineStroke {
                                    color: metric_line_color(kind, 0, dark_mode),
                                    style: LineStyle::Solid,
                                    width: TEC_LINE_WIDTH,
                                    highlighted: false,
                                },
                                LineViewport {
                                    x_min,
                                    x_max,
                                    width: CANVAS_SIZE.x,
                                    cap: samples.len(),
                                },
                                None,
                                &mut NearestHoverLabel::default(),
                            );
                        });
                });
        });
    harness.run();
    let rendered = harness.inner.render().expect("the frame renders");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(ASSET_PATH);
    rendered.save(&path).expect("the asset is written");
    println!("wrote {}", path.display());
}

/// Names the quiet reference in the plot's legend.
const MEDIAN_LINE_LABEL: &str = "27-day median";

/// Grid marks landing on every UTC day and every six hours between them, so
/// the axis of a week-long span is labelled in days.
fn day_and_six_hour_marks(bounds: (f64, f64)) -> Vec<GridMark> {
    const SIX_HOURS_SECS: f64 = 6.0 * 60.0 * 60.0;
    const DAY_SECS: f64 = 24.0 * 60.0 * 60.0;
    let (start, end) = bounds;
    let first = (start / SIX_HOURS_SECS).ceil() * SIX_HOURS_SECS;
    let mut marks = Vec::new();
    let mut value = first;
    while value <= end {
        let step_size = if value.rem_euclid(DAY_SECS) == 0.0 {
            DAY_SECS
        } else {
            SIX_HOURS_SECS
        };
        marks.push(GridMark { value, step_size });
        value += SIX_HOURS_SECS;
    }
    marks
}

/// A tick label: the date at midnight, the hour anywhere else, so a span of
/// days reads as days without losing the diurnal cycle.
fn day_or_hour_label(seconds: f64) -> String {
    let Some(instant) = DateTime::from_timestamp(seconds as i64, 0) else {
        return String::new();
    };
    if instant.time() == NaiveTime::MIN {
        return instant.format("%m-%d").to_string();
    }
    instant.format("%H:%M").to_string()
}
