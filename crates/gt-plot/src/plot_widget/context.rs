//! The context metric lines: aircraft interference, the geomagnetic indices
//! and TEC.
//!
//! Each is drawn across the plot's whole visible span from what the archive
//! holds, so how a metric evolved and how abruptly it changed can be read
//! over spans no recording covers. The line breaks over days the archive
//! holds nothing for.

use std::sync::Arc;

use egui_plot::PlotPoint;
use gt_egui_mipmap::MipMap;
use gt_solar::GeomagneticIndex;
use gt_types::MetricKind;
use gt_ui_types::{
    ArcIdentity, ContextLines, IndexContextSample, JammingContextSample, TecContextSample,
};

use super::chips::{MetricAvailability, MetricKindUi, MetricVisibility};
use super::geomagnetic::GeomagneticHover;
use super::jamming::JammingHover;
use super::levels::LineViewport;
use super::lines::{HOVER_RADIUS_PX, LineStroke, NearestHoverLabel, PlotHoverLabel, add_line};
use super::tec::TecHover;

/// How long one interference sample holds: the dataset is published per whole
/// UTC day.
const INTERFERENCE_PERIOD_SECS: f64 = 24.0 * 60.0 * 60.0;

/// A context line's sample: where it starts, and what the line is worth
/// there.
trait ContextSample: Copy {
    fn start_secs(self) -> f64;
    fn value(self) -> Option<f64>;
}

impl ContextSample for JammingContextSample {
    fn start_secs(self) -> f64 {
        self.start_secs
    }

    fn value(self) -> Option<f64> {
        self.percent
    }
}

impl ContextSample for IndexContextSample {
    fn start_secs(self) -> f64 {
        self.start_secs
    }

    fn value(self) -> Option<f64> {
        self.value
    }
}

impl ContextSample for TecContextSample {
    fn start_secs(self) -> f64 {
        self.x_secs
    }

    fn value(self) -> Option<f64> {
        self.tecu
    }
}

/// One stretch of a context line the pointer can rest on: the x span it
/// covers, the line's y at each end of that span, and the sample the value
/// was read from.
#[derive(Debug, Clone, Copy)]
struct HoverSegment<S> {
    x_start: f64,
    x_end: f64,
    y_start: f64,
    y_end: f64,
    sample: S,
}

/// One context line's drawable runs and hover segments, rebuilt when the app
/// hands over new samples.
#[derive(Debug, Clone)]
struct ContextLineCache<S> {
    source: ArcIdentity,
    /// Mipmapped points, one entry per unbroken stretch of the line.
    runs: Vec<MipMap>,
    /// Every stretch the line has a value over, ascending by x.
    segments: Vec<HoverSegment<S>>,
}

impl<S> Default for ContextLineCache<S> {
    fn default() -> Self {
        Self {
            source: ArcIdentity::default(),
            runs: Vec::new(),
            segments: Vec::new(),
        }
    }
}

impl<S: ContextSample> ContextLineCache<S> {
    /// Rebuild from `samples` when the app replaced them.
    fn sync(&mut self, samples: &Arc<Vec<S>>, shape: LineShape) {
        let source = ArcIdentity::of(samples);
        if self.source == source {
            return;
        }
        let (runs, segments) = match shape {
            LineShape::Step { period_secs } => step_line(samples, period_secs),
            LineShape::Interpolated => interpolated_line(samples),
        };
        *self = Self {
            source,
            runs,
            segments,
        };
    }

    /// The point on the line at the pointer's own time, its pixel distance
    /// from the pointer, and the sample it was read from.
    fn under_pointer(
        &self,
        plot_ui: &egui_plot::PlotUi<'_>,
        pointer: egui::Pos2,
    ) -> Option<(f32, PlotPoint, S)> {
        let x = plot_ui.plot_from_screen(pointer).x;
        let index = self
            .segments
            .partition_point(|segment| segment.x_start <= x)
            .checked_sub(1)?;
        let segment = self.segments.get(index)?;
        if x > segment.x_end {
            return None;
        }
        let span = segment.x_end - segment.x_start;
        let fraction = if span > 0.0 {
            (x - segment.x_start) / span
        } else {
            0.0
        };
        let point = PlotPoint::new(
            x,
            segment.y_start + (segment.y_end - segment.y_start) * fraction,
        );
        let distance = (plot_ui.screen_from_plot(point).y - pointer.y).abs();
        (distance <= HOVER_RADIUS_PX).then_some((distance, point, segment.sample))
    }
}

/// How a line runs between its samples.
#[derive(Debug, Clone, Copy)]
enum LineShape {
    /// Each sample's value holds unchanged for `period_secs` from its start,
    /// which is what the source publishes.
    Step { period_secs: f64 },
    /// Values run linearly from one sample to the next.
    Interpolated,
}

/// The staircase a stepped source draws: each valued sample holds for
/// `period_secs`, and a sample with no value ends the stretch.
fn step_line<S: ContextSample>(
    samples: &[S],
    period_secs: f64,
) -> (Vec<MipMap>, Vec<HoverSegment<S>>) {
    let mut runs = Vec::new();
    let mut segments = Vec::new();
    let mut points: Vec<[f64; 2]> = Vec::new();
    for &sample in samples {
        let Some(value) = sample.value() else {
            flush_run(&mut points, &mut runs);
            continue;
        };
        let (x_start, x_end) = (sample.start_secs(), sample.start_secs() + period_secs);
        points.push([x_start, value]);
        points.push([x_end, value]);
        segments.push(HoverSegment {
            x_start,
            x_end,
            y_start: value,
            y_end: value,
            sample,
        });
    }
    flush_run(&mut points, &mut runs);
    (runs, segments)
}

/// The polyline an interpolated source draws: one point per valued sample,
/// and a sample with no value ends the stretch.
fn interpolated_line<S: ContextSample>(samples: &[S]) -> (Vec<MipMap>, Vec<HoverSegment<S>>) {
    let mut runs = Vec::new();
    let mut segments = Vec::new();
    let mut points: Vec<[f64; 2]> = Vec::new();
    let mut previous: Option<[f64; 2]> = None;
    for &sample in samples {
        let Some(value) = sample.value() else {
            flush_run(&mut points, &mut runs);
            previous = None;
            continue;
        };
        let x = sample.start_secs();
        if let Some([previous_x, previous_y]) = previous {
            segments.push(HoverSegment {
                x_start: previous_x,
                x_end: x,
                y_start: previous_y,
                y_end: value,
                sample,
            });
        }
        points.push([x, value]);
        previous = Some([x, value]);
    }
    flush_run(&mut points, &mut runs);
    (runs, segments)
}

/// Close the stretch being collected. One point draws no visible geometry, so
/// only a stretch of two or more becomes a run.
fn flush_run(points: &mut Vec<[f64; 2]>, runs: &mut Vec<MipMap>) {
    if points.len() >= 2 {
        runs.push(MipMap::build(std::mem::take(points)));
    } else {
        points.clear();
    }
}

/// Every context line's mipmaps, held across frames and rebuilt when the app
/// replaces the samples behind one of them.
#[derive(Debug, Clone, Default)]
pub(super) struct ContextPlotCaches {
    jamming: ContextLineCache<JammingContextSample>,
    hp30: ContextLineCache<IndexContextSample>,
    kp: ContextLineCache<IndexContextSample>,
    tec: ContextLineCache<TecContextSample>,
}

impl ContextPlotCaches {
    pub(super) fn sync(&mut self, lines: &ContextLines) {
        self.jamming.sync(
            &lines.jamming,
            LineShape::Step {
                period_secs: INTERFERENCE_PERIOD_SECS,
            },
        );
        self.hp30.sync(
            &lines.geomagnetic.hp30,
            LineShape::Step {
                period_secs: GeomagneticIndex::Hp30.period_length().num_seconds() as f64,
            },
        );
        self.kp.sync(
            &lines.geomagnetic.kp,
            LineShape::Step {
                period_secs: GeomagneticIndex::Kp.period_length().num_seconds() as f64,
            },
        );
        self.tec.sync(&lines.tec, LineShape::Interpolated);
    }
}

/// What one context line needs besides its own samples.
#[derive(Clone, Copy)]
struct ContextLineDraw {
    kind: MetricKind,
    stroke: LineStroke,
    viewport: LineViewport,
    pointer: Option<egui::Pos2>,
}

/// Draw the context lines whose chip is enabled and toggled on, and hit-test
/// the pointer against each so the closest one can label itself.
pub(super) fn add_context_lines<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    caches: &'a ContextPlotCaches,
    shown: ContextLineGates<'_>,
    stroke_of: impl Fn(MetricKind) -> LineStroke,
    viewport: LineViewport,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
) {
    let line = |kind| ContextLineDraw {
        kind,
        stroke: stroke_of(kind),
        viewport,
        pointer,
    };

    if shown.draws(MetricKind::Jamming) {
        add_context_line(
            plot_ui,
            &caches.jamming,
            line(MetricKind::Jamming),
            nearest,
            |_, sample| PlotHoverLabel::Jamming(JammingHover::of_archived_day(sample)),
        );
    }
    for (index, cache, kind) in [
        (GeomagneticIndex::Hp30, &caches.hp30, MetricKind::Hp30),
        (GeomagneticIndex::Kp, &caches.kp, MetricKind::Kp),
    ] {
        if !shown.draws(kind) {
            continue;
        }
        add_context_line(plot_ui, cache, line(kind), nearest, move |_, sample| {
            PlotHoverLabel::Geomagnetic(GeomagneticHover::of_archived_period(index, sample))
        });
    }
    if shown.draws(MetricKind::Tec) {
        add_context_line(
            plot_ui,
            &caches.tec,
            line(MetricKind::Tec),
            nearest,
            |point, _| PlotHoverLabel::Tec(TecHover::of_line_point(point)),
        );
    }
}

/// Which context lines draw: the chip's own toggle, gated on the chip being
/// enabled at all.
#[derive(Clone, Copy)]
pub(super) struct ContextLineGates<'a> {
    pub(super) metric_vis: &'a MetricVisibility,
    pub(super) available: MetricAvailability,
}

impl ContextLineGates<'_> {
    fn draws(self, kind: MetricKind) -> bool {
        self.metric_vis.field(kind) && self.available.has_data(kind)
    }
}

fn add_context_line<'a, S: ContextSample>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    cache: &'a ContextLineCache<S>,
    draw: ContextLineDraw,
    nearest: &mut NearestHoverLabel,
    hover: impl FnOnce(PlotPoint, S) -> PlotHoverLabel,
) {
    for (run, selection) in cache
        .runs
        .iter()
        .zip(draw.viewport.select_run_levels(&cache.runs))
    {
        add_line(
            plot_ui,
            run.slice_at(selection),
            draw.kind.label().to_owned(),
            draw.stroke,
        );
    }

    let Some(pointer) = draw.pointer else {
        return;
    };
    if let Some((distance, point, sample)) = cache.under_pointer(plot_ui, pointer) {
        nearest.offer(distance, || hover(point, sample));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_sample(start_secs: f64, value: Option<f64>) -> IndexContextSample {
        IndexContextSample { start_secs, value }
    }

    fn tec_sample(x_secs: f64, tecu: Option<f64>) -> TecContextSample {
        TecContextSample { x_secs, tecu }
    }

    /// A stepped line holds each value for its whole period and rises at the
    /// boundary, so the drawn points come in pairs.
    #[test]
    fn a_stepped_sample_holds_for_its_period() {
        let samples = [
            index_sample(0.0, Some(3.0)),
            index_sample(1800.0, Some(5.0)),
        ];

        let (runs, segments) = step_line(&samples, 1800.0);

        assert_eq!(
            runs.iter().map(MipMap::original_len).collect::<Vec<_>>(),
            [4]
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.x_start, segment.x_end, segment.y_start))
                .collect::<Vec<_>>(),
            [(0.0, 1800.0, 3.0), (1800.0, 3600.0, 5.0)]
        );
    }

    /// A sample with no value ends the run, so the line breaks over what the
    /// archive does not cover instead of bridging it.
    #[test]
    fn a_sample_without_a_value_breaks_a_stepped_line() {
        let samples = [
            index_sample(0.0, Some(3.0)),
            index_sample(1800.0, None),
            index_sample(3600.0, Some(5.0)),
            index_sample(5400.0, Some(6.0)),
        ];

        let (runs, segments) = step_line(&samples, 1800.0);

        assert_eq!(
            runs.iter().map(MipMap::x_range).collect::<Vec<_>>(),
            [Some((0.0, 1800.0)), Some((3600.0, 7200.0))]
        );
        assert_eq!(segments.len(), 3);
    }

    /// A lone valued sample of an interpolated line draws nothing: one point
    /// is no visible geometry.
    #[test]
    fn a_lone_interpolated_sample_draws_no_run() {
        let samples = [
            tec_sample(0.0, None),
            tec_sample(3600.0, Some(12.0)),
            tec_sample(7200.0, None),
        ];

        let (runs, segments) = interpolated_line(&samples);

        assert!(runs.is_empty());
        assert!(segments.is_empty());
    }

    /// An interpolated line runs from one epoch to the next, and a storm
    /// value is drawn as published.
    #[test]
    fn an_interpolated_line_spans_its_epochs() {
        let samples = [
            tec_sample(0.0, Some(12.0)),
            tec_sample(7200.0, Some(175.5)),
            tec_sample(14400.0, None),
            tec_sample(21600.0, Some(20.0)),
        ];

        let (runs, segments) = interpolated_line(&samples);

        assert_eq!(
            runs.iter().map(MipMap::x_range).collect::<Vec<_>>(),
            [Some((0.0, 7200.0))]
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.x_start, segment.y_start, segment.y_end))
                .collect::<Vec<_>>(),
            [(0.0, 12.0, 175.5)]
        );
    }

    /// The cache keys on `Arc` identity: unchanged samples are left alone,
    /// and replaced ones rebuild.
    #[test]
    fn the_cache_rebuilds_on_a_new_allocation() {
        let mut cache = ContextLineCache::default();
        let samples = Arc::new(vec![
            index_sample(0.0, Some(3.0)),
            index_sample(1800.0, Some(5.0)),
        ]);
        let shape = LineShape::Step {
            period_secs: 1800.0,
        };

        cache.sync(&samples, shape);
        let first = cache.source;
        assert_eq!(cache.segments.len(), 2);

        cache.sync(&samples, shape);
        assert_eq!(cache.source, first);

        cache.sync(&Arc::new(vec![index_sample(0.0, Some(3.0))]), shape);
        assert_ne!(cache.source, first);
        assert_eq!(cache.segments.len(), 1);
    }
}
