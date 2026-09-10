//! The off-scale clock offset overlay: an indicator for each sample the plot
//! holds off its clock offset line, plus its hover text.
//!
//! A sample is held off the line (see
//! [`crate::series::TrackSeries::clock_delta_ms`]) when its offset departed
//! from the track's baseline, or when the track's baseline itself lies outside
//! [`gt_analysis::clock_offset::MAX_PLOTTED_OFFSET_S`]. Either
//! offset would set the auto-bounds of the y-axis every metric shares. Each
//! held-back sample is marked here at the edge it ran off, with its true offset
//! on hover, and drawn in place once the view is zoomed out far enough to hold
//! it.
//!
//! A connector joins each run of held-back samples to the clock offset line:
//! from the line point before the run up to the base of the first marker, from
//! marker to marker along the edge, and from the last marker back down to the
//! line point after the run.  At the end of a recording, and for a track whose
//! baseline is off-scale over its whole length, the markers join to each other
//! alone.  The plot draws a connector only between two points the view holds.

use std::ops::Range;

use chrono::DateTime;
use egui::epaint::{Shape, Stroke};
use egui::{Color32, Pos2, Vec2};
use egui_plot::{PlotPoint, PlotTransform};
use gt_analysis::clock_offset::{ClockOffsetExcursion, ClockOffsetPlacement, ExcursionSample};
use gt_types::MetricKind;

use super::chips::MetricVisibility;
use super::lines::{
    self, ANOMALY_HOVER_RADIUS_PX, ANOMALY_MARKER_RADIUS, NearestHoverLabel, PlotHoverLabel,
};
use super::overlay::{EDGE_MARKER_INSET, OverlayItem, OverlayPainter};
use crate::series::TrackSeries;

/// Half-width of a marker glyph, in points.
const MARKER_HALF_WIDTH: f32 = ANOMALY_MARKER_RADIUS;

/// Width of a connector between a marker and what it joins.
const CONNECTOR_WIDTH: f32 = 1.0;

/// Screen distance below which two markers overlap into one glyph, in points.
/// The drawn markers are spaced by this much: a whole track goes off-scale at
/// once.  Every sample stays a hover target.
const MARKER_MIN_SPACING_PX: f32 = 2.0 * MARKER_HALF_WIDTH;

/// The y range the plot currently shows.
#[derive(Debug, Clone, Copy, PartialEq)]
struct VisibleYRange {
    min: f64,
    max: f64,
}

/// Where an excursion sample is drawn, once the current view is taken into
/// account.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Placement {
    /// Below the visible range - the marker points down at the bottom edge.
    OffScaleBelow,
    /// Above the visible range - the marker points up at the top edge.
    OffScaleAbove,
    /// Inside the visible range, so the sample is drawn where it belongs.
    InView,
}

impl Placement {
    fn resolve(value: f64, y: VisibleYRange) -> Self {
        if value < y.min {
            Self::OffScaleBelow
        } else if value > y.max {
            Self::OffScaleAbove
        } else {
            Self::InView
        }
    }

    /// Y in plot coordinates for a sample worth `value`, given the visible
    /// range: the near edge when it runs off, the value itself when it fits.
    fn place(self, value: f64, y: VisibleYRange) -> f64 {
        let inset = (y.max - y.min) * EDGE_MARKER_INSET;
        match self {
            Self::OffScaleBelow => y.min + inset,
            Self::OffScaleAbove => y.max - inset,
            Self::InView => value,
        }
    }

    /// The glyph, centred on `at`: a triangle pointing the way the value ran
    /// off, or a diamond where the value sits in view.
    fn glyph(self, at: Pos2) -> Vec<Pos2> {
        let w = MARKER_HALF_WIDTH;
        match self {
            Self::OffScaleBelow => vec![
                at + Vec2::new(-w, -w),
                at + Vec2::new(w, -w),
                at + Vec2::new(0.0, w),
            ],
            Self::OffScaleAbove => vec![
                at + Vec2::new(-w, w),
                at + Vec2::new(w, w),
                at + Vec2::new(0.0, -w),
            ],
            Self::InView => vec![
                at + Vec2::new(0.0, -w),
                at + Vec2::new(w, 0.0),
                at + Vec2::new(0.0, w),
                at + Vec2::new(-w, 0.0),
            ],
        }
    }

    /// Where a connector meets the glyph drawn at `center`: the flat base of
    /// the triangle, or the centre of the diamond drawn at a real value.
    fn connector_base(self, center: Pos2) -> Pos2 {
        match self {
            Self::OffScaleBelow => center + Vec2::new(0.0, -MARKER_HALF_WIDTH),
            Self::OffScaleAbove => center + Vec2::new(0.0, MARKER_HALF_WIDTH),
            Self::InView => center,
        }
    }
}

/// What put one sample off the shared y-axis.
#[derive(Clone, Copy)]
enum OffScaleCause<'a> {
    /// The track's baseline lies outside
    /// [`gt_analysis::clock_offset::MAX_PLOTTED_OFFSET_S`].
    Baseline,
    /// The offset departed from the track's baseline.
    Excursion(&'a ClockOffsetExcursion),
}

/// One held-back sample, placed against the view the plot draws this frame.
struct PlacedMarker<'a> {
    placement: Placement,
    at: PlotPoint,
    sample: ExcursionSample,
    cause: OffScaleCause<'a>,
}

impl<'a> PlacedMarker<'a> {
    fn new(sample: ExcursionSample, cause: OffScaleCause<'a>, y: VisibleYRange) -> Self {
        let (placement, at) = placed_point(&sample, y);
        Self {
            placement,
            at,
            sample,
            cause,
        }
    }
}

/// One run of consecutive held-back samples against the view the plot draws
/// this frame, and the clock offset line beside it.
struct RunPlacement<'a> {
    /// Every sample of the run, in ascending index order.
    samples: &'a [ExcursionSample],
    /// The part of [`Self::samples`] inside the visible x range.
    visible: Range<usize>,
    /// The track's clock offset line points, ascending by x.
    line: &'a [PlotPoint],
    y: VisibleYRange,
}

impl RunPlacement<'_> {
    /// The samples of the run that the plot marks this frame.
    fn visible_samples(&self) -> &[ExcursionSample] {
        self.samples.get(self.visible.clone()).unwrap_or_default()
    }

    /// Where the connector into the run's first marker starts: the run's own
    /// sample left of the visible x range, or the line point that the run
    /// departed from.
    fn entry(&self) -> Option<PlotPoint> {
        let first = self.visible_samples().first()?;
        if let Some(index) = self.visible.start.checked_sub(1)
            && let Some(before) = self.samples.get(index)
        {
            let (_, at) = placed_point(before, self.y);
            return Some(at);
        }
        let index = self.line.partition_point(|p| p.x <= first.t);
        self.shown(*self.line.get(index.checked_sub(1)?)?)
    }

    /// Where the connector out of the run's last marker ends: the run's own
    /// sample right of the visible x range, or the line point that the run
    /// returned to.
    fn exit(&self) -> Option<PlotPoint> {
        let last = self.visible_samples().last()?;
        if let Some(after) = self.samples.get(self.visible.end) {
            let (_, at) = placed_point(after, self.y);
            return Some(at);
        }
        let index = self.line.partition_point(|p| p.x < last.t);
        self.shown(*self.line.get(index)?)
    }

    /// `point` where the visible y range holds it, and [`None`] where it lies
    /// outside, which leaves out the connector to it.
    fn shown(&self, point: PlotPoint) -> Option<PlotPoint> {
        (self.y.min..=self.y.max)
            .contains(&point.y)
            .then_some(point)
    }
}

/// What the plot draws for one run of held-back samples: its markers, and the
/// two points its connectors reach beyond them.
struct DrawnRun {
    /// The markers the plot draws, in ascending x, spaced by
    /// [`MARKER_MIN_SPACING_PX`].
    markers: Vec<(Placement, PlotPoint)>,
    /// Where the connector into the first marker starts, from
    /// [`RunPlacement::entry`].
    entry: Option<PlotPoint>,
    /// Where the connector out of the last marker ends, from
    /// [`RunPlacement::exit`].
    exit: Option<PlotPoint>,
}

/// The off-scale indicators of one track, one entry per run of consecutive
/// held-back samples.
struct OffScaleMarkers {
    runs: Vec<DrawnRun>,
    color: Color32,
}

impl OffScaleMarkers {
    /// Draw one run: its connectors first, then the glyphs over them.
    fn paint_run(&self, run: &DrawnRun, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let drawn: Vec<(Placement, Pos2)> = run
            .markers
            .iter()
            .map(|&(placement, at)| (placement, glyph_center(transform, at)))
            .collect();
        if drawn.is_empty() {
            return;
        }

        let mut connected: Vec<Pos2> = Vec::with_capacity(drawn.len() + 2);
        connected.extend(run.entry.map(|at| transform.position_from_point(&at)));
        connected.extend(
            drawn
                .iter()
                .map(|&(placement, center)| placement.connector_base(center)),
        );
        connected.extend(run.exit.map(|at| transform.position_from_point(&at)));
        let stroke = Stroke::new(CONNECTOR_WIDTH, self.color);
        for pair in connected.windows(2) {
            if let [from, to] = *pair {
                shapes.push(Shape::line_segment([from, to], stroke));
            }
        }

        for (placement, center) in drawn {
            shapes.push(Shape::convex_polygon(
                placement.glyph(center),
                self.color,
                Stroke::NONE,
            ));
        }
    }
}

impl OverlayPainter for OffScaleMarkers {
    fn legend_color(&self) -> Color32 {
        self.color
    }

    fn paint(&self, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        for run in &self.runs {
            self.paint_run(run, transform, shapes);
        }
    }
}

/// The frame-level inputs the off-scale overlay needs beyond the track itself:
/// the visible x range it clips to, the metric visibility it gates on, and the
/// theme.
#[derive(Clone, Copy)]
pub(super) struct ClockOffsetViewport<'v> {
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    pub(super) metric_vis: &'v MetricVisibility,
    pub(super) dark_mode: bool,
}

/// Draw the off-scale clock offset indicators for one track and, when the
/// pointer is within [`ANOMALY_HOVER_RADIUS_PX`] of one, record the nearest in
/// `nearest` so the caller can show its tooltip.
///
/// The indicators follow the clock offset line's chip: with the metric hidden,
/// this draws nothing.
pub(super) fn add_off_scale_clock_offsets(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    series: &TrackSeries,
    track_label: Option<&str>,
    viewport: ClockOffsetViewport<'_>,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
) {
    if !viewport.metric_vis.field(MetricKind::ClockDeltaMs) {
        return;
    }
    let bounds = plot_ui.plot_bounds();
    let y = VisibleYRange {
        min: bounds.min()[1],
        max: bounds.max()[1],
    };
    let (title, sample_runs): (&str, Vec<(OffScaleCause<'_>, &[ExcursionSample])>) =
        match &series.clock_offset_placement {
            ClockOffsetPlacement::BaselineOffScale(off_scale) => (
                OFF_SCALE_BASELINE_TITLE,
                vec![(OffScaleCause::Baseline, off_scale.samples.as_slice())],
            ),
            ClockOffsetPlacement::BaselineOnScale(excursions) => (
                EXCURSION_TITLE,
                excursions
                    .iter()
                    .map(|excursion| {
                        (
                            OffScaleCause::Excursion(excursion),
                            excursion.samples.as_slice(),
                        )
                    })
                    .collect(),
            ),
        };

    let line = series.clock_delta_ms.finest_level();
    let mut placed: Vec<PlacedMarker<'_>> = Vec::new();
    let mut runs: Vec<DrawnRun> = Vec::new();
    for (cause, samples) in sample_runs {
        let run = RunPlacement {
            samples,
            visible: lines::visible_x_range(
                samples,
                |s: &ExcursionSample| s.t,
                viewport.x_min,
                viewport.x_max,
            ),
            line,
            y,
        };
        let markers: Vec<PlacedMarker<'_>> = run
            .visible_samples()
            .iter()
            .map(|&sample| PlacedMarker::new(sample, cause, y))
            .collect();
        if markers.is_empty() {
            continue;
        }
        runs.push(DrawnRun {
            markers: spaced_markers(&markers, |at| plot_ui.screen_from_plot(at).x),
            entry: run.entry(),
            exit: run.exit(),
        });
        placed.extend(markers);
    }
    if placed.is_empty() {
        return;
    }

    plot_ui.add(OverlayItem::new(
        title,
        OffScaleMarkers {
            runs,
            color: gt_ui_theme::warning_amber(viewport.dark_mode),
        },
    ));

    let Some(pointer) = pointer else {
        return;
    };
    if let Some((distance, marker)) = lines::nearest_fix_under_pointer(
        plot_ui,
        &placed,
        |marker| marker.at,
        pointer,
        ANOMALY_HOVER_RADIUS_PX,
    ) {
        nearest.offer(distance, || {
            PlotHoverLabel::ClockOffset(ClockOffsetHover::new(
                track_label,
                marker.sample,
                marker.cause,
            ))
        });
    }
}

/// Pre-formatted tooltip contents for one off-scale clock offset sample.
pub(super) struct ClockOffsetHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    gps_time: String,
    sys_time: String,
    offset: String,
    cause: FormattedCause,
}

/// What put the sample off the shared y-axis.
#[derive(Debug, PartialEq)]
enum FormattedCause {
    /// The track's baseline lies outside what the shared y-axis shows.
    Baseline,
    /// The offset departed from the track's baseline.
    Excursion {
        /// The track's baseline offset.
        baseline: String,
        /// How many samples the excursion this sample belongs to spans.
        samples: usize,
        /// `true` where a later sample came back to the baseline, `false`
        /// where the recording ends inside the excursion.
        returned_to_baseline: bool,
    },
}

impl ClockOffsetHover {
    fn new(track_label: Option<&str>, sample: ExcursionSample, cause: OffScaleCause<'_>) -> Self {
        // The time of day alone does not say which instant either clock
        // stamped: the two clocks of an off-scale baseline are a day or more
        // apart.
        let (gps_format, sys_format) = match cause {
            OffScaleCause::Baseline => ("%Y-%m-%d %H:%M:%S UTC", "%Y-%m-%d %H:%M:%S%.3f UTC"),
            OffScaleCause::Excursion(_) => ("%H:%M:%S", "%H:%M:%S%.3f"),
        };
        let gps_ms = (sample.t * 1000.0) as i64;
        Self {
            track: track_label.map(ToOwned::to_owned),
            gps_time: format_ms(gps_ms, gps_format),
            // The offset is GPS−system, so the host stamp is the GPS epoch
            // less the offset.
            sys_time: format_ms(gps_ms.saturating_sub(sample.offset_ms), sys_format),
            offset: gt_fmt::format_signed_delta(sample.offset_ms),
            cause: match cause {
                OffScaleCause::Baseline => FormattedCause::Baseline,
                OffScaleCause::Excursion(excursion) => FormattedCause::Excursion {
                    baseline: gt_fmt::format_signed_delta(excursion.baseline_ms),
                    samples: excursion.samples.len(),
                    returned_to_baseline: excursion.returned_to_baseline,
                },
            },
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(match self.cause {
            FormattedCause::Baseline => OFF_SCALE_BASELINE_TITLE,
            FormattedCause::Excursion { .. } => EXCURSION_TITLE,
        });
        if let Some(track) = &self.track {
            ui.label(track);
        }
        ui.label(format!("GPS epoch {}", self.gps_time));
        ui.label(format!("System timestamp {}", self.sys_time));
        ui.separator();
        ui.label(format!("Offset {}", self.offset));
        let FormattedCause::Excursion {
            baseline,
            samples,
            returned_to_baseline,
        } = &self.cause
        else {
            ui.separator();
            ui.label(
                "Every fix of this recording is marked at the edge. Zoom the y-axis out to \
                 draw the offset in place.",
            );
            return;
        };
        ui.label(format!("Track baseline {baseline}"));
        ui.separator();
        let span = format!(
            "{samples} {}",
            gt_fmt::pluralize(*samples, "sample", "samples")
        );
        ui.label(if *returned_to_baseline {
            format!("The offset left the track's baseline for {span} and returned.")
        } else {
            format!(
                "The offset left the track's baseline for {span}, and the recording ends there."
            )
        });
    }
}

/// Where the plot draws `sample`: at its own offset while the visible range
/// holds it, at the near edge while it does not.
fn placed_point(sample: &ExcursionSample, y: VisibleYRange) -> (Placement, PlotPoint) {
    let value = sample.offset_ms as f64;
    let placement = Placement::resolve(value, y);
    (
        placement,
        PlotPoint::new(sample.t, placement.place(value, y)),
    )
}

/// Where the glyph for a marker at `at` is drawn whole: the marker's own
/// position, held a half glyph inside the left and the right boundary of the
/// plot rect, as [`Placement::place`] insets one at the top or the bottom
/// edge.
///
/// Written as a maximum and then a minimum because `f32::clamp` panics where
/// the plot rect is narrower than a glyph.
fn glyph_center(transform: &PlotTransform, at: PlotPoint) -> Pos2 {
    let frame = transform.frame();
    let center = transform.position_from_point(&at);
    Pos2::new(
        center
            .x
            .max(frame.left() + MARKER_HALF_WIDTH)
            .min(frame.right() - MARKER_HALF_WIDTH),
        center.y,
    )
}

/// The first marker of every run of markers within [`MARKER_MIN_SPACING_PX`] of
/// each other, given `screen_x` for one marker's position. `placed` is in
/// ascending x.
fn spaced_markers(
    placed: &[PlacedMarker<'_>],
    screen_x: impl Fn(PlotPoint) -> f32,
) -> Vec<(Placement, PlotPoint)> {
    let mut last_drawn_x: Option<f32> = None;
    placed
        .iter()
        .filter(|marker| {
            let x = screen_x(marker.at);
            if last_drawn_x.is_some_and(|last| x - last < MARKER_MIN_SPACING_PX) {
                return false;
            }
            last_drawn_x = Some(x);
            true
        })
        .map(|marker| (marker.placement, marker.at))
        .collect()
}

/// Format a Unix-millisecond timestamp, or an empty string when it is out of
/// range for a date.
fn format_ms(ms: i64, fmt: &str) -> String {
    DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format(fmt).to_string())
        .unwrap_or_default()
}

const EXCURSION_TITLE: &str = "Clock offset excursion";
const OFF_SCALE_BASELINE_TITLE: &str = "Clock offset off the plot's scale";

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2};
    use egui_plot::PlotBounds;
    use vec1::vec1;

    use super::*;

    /// 2024-01-15 12:00:00 UTC, the x of the excursion sample below.
    const T: f64 = 1_705_320_000.0;

    /// The plot area the paint cases draw into, in points.
    const FRAME: Vec2 = Vec2::new(700.0, 400.0);

    /// The y range the paint cases show, in milliseconds of offset.
    const VISIBLE_Y: VisibleYRange = VisibleYRange {
        min: -100.0,
        max: 100.0,
    };

    /// The `gnss.h5.gtd` sample: a steady −234 ms offset, and this one carrying
    /// the whole 1 h 09 m recording gap.
    fn excursion() -> ClockOffsetExcursion {
        ClockOffsetExcursion {
            samples: vec1![ExcursionSample {
                index: 8207,
                t: T,
                offset_ms: -4_127_054,
            }],
            baseline_ms: -234,
            returned_to_baseline: true,
        }
    }

    /// A sample `index` seconds after [`T`], with `offset_ms` of clock offset.
    fn sample(index: usize, offset_ms: i64) -> ExcursionSample {
        ExcursionSample {
            index,
            t: T + index as f64,
            offset_ms,
        }
    }

    /// A run of `samples`, all of it inside the visible x range, against a
    /// clock offset line of `line` points at the baseline, one per second
    /// from [`T`].
    fn run_over_a_line<'a>(
        samples: &'a [ExcursionSample],
        line: &'a [PlotPoint],
    ) -> RunPlacement<'a> {
        RunPlacement {
            samples,
            visible: 0..samples.len(),
            line,
            y: VISIBLE_Y,
        }
    }

    /// The line the tracks below hold at their baseline, one point per second
    /// from [`T`] over `secs`, skipping the seconds `run` covers.
    fn baseline_line(secs: Range<usize>, run: Range<usize>) -> Vec<PlotPoint> {
        secs.filter(|index| !run.contains(index))
            .map(|index| PlotPoint::new(T + index as f64, 0.0))
            .collect()
    }

    /// The view the paint cases draw under: [`FRAME`] points showing ten
    /// seconds from [`T`] over [`VISIBLE_Y`].
    fn paint_transform() -> PlotTransform {
        PlotTransform::new(
            Rect::from_min_size(Pos2::ZERO, FRAME),
            PlotBounds::from_min_max([T, VISIBLE_Y.min], [T + 10.0, VISIBLE_Y.max]),
            false,
        )
    }

    fn painted(runs: Vec<DrawnRun>) -> Vec<Shape> {
        let mut shapes = Vec::new();
        OffScaleMarkers {
            runs,
            color: Color32::WHITE,
        }
        .paint(&paint_transform(), &mut shapes);
        shapes
    }

    fn connectors(shapes: &[Shape]) -> Vec<[Pos2; 2]> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::LineSegment { points, .. } => Some(*points),
                _ => None,
            })
            .collect()
    }

    /// The bounding box of each glyph the paint drew, in the order it drew
    /// them.
    fn glyph_bounds(shapes: &[Shape]) -> Vec<Rect> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Path(path) => Some(Rect::from_points(&path.points)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_run_is_joined_to_the_line_point_on_either_side_of_it() {
        let samples = [sample(2, 500_000), sample(3, 500_000)];
        let line = baseline_line(0..6, 2..4);

        let run = run_over_a_line(&samples, &line);

        assert_eq!(run.entry(), Some(PlotPoint::new(T + 1.0, 0.0)));
        assert_eq!(run.exit(), Some(PlotPoint::new(T + 4.0, 0.0)));
    }

    #[test]
    fn a_run_that_the_recording_ends_inside_has_no_connector_after_it() {
        let samples = [sample(4, 500_000), sample(5, 500_000)];
        let line = baseline_line(0..6, 4..6);

        let run = run_over_a_line(&samples, &line);

        assert_eq!(run.entry(), Some(PlotPoint::new(T + 3.0, 0.0)));
        assert_eq!(run.exit(), None);
    }

    /// Every sample of a track whose baseline is off-scale is a marker. Those
    /// markers join to each other alone.
    #[test]
    fn a_run_with_no_line_beside_it_has_no_connector_at_either_end() {
        let samples = [sample(0, 500_000), sample(1, 500_000)];

        let run = run_over_a_line(&samples, &[]);

        assert_eq!(run.entry(), None);
        assert_eq!(run.exit(), None);
    }

    #[test]
    fn no_connector_reaches_a_line_point_outside_the_visible_y_range() {
        let samples = [sample(2, 500_000)];
        let line = [
            PlotPoint::new(T + 1.0, -4_000.0),
            PlotPoint::new(T + 3.0, 0.0),
        ];

        let run = run_over_a_line(&samples, &line);

        assert_eq!(run.entry(), None);
        assert_eq!(run.exit(), Some(PlotPoint::new(T + 3.0, 0.0)));
    }

    /// The connectors reach the run's own samples beyond either boundary, each
    /// at the edge its markers sit at: the view starts and ends inside the
    /// run.
    #[test]
    fn a_run_wider_than_the_view_joins_its_own_samples_beyond_it() {
        let samples = [sample(0, 500_000), sample(1, 500_000), sample(2, 500_000)];
        let run = RunPlacement {
            samples: &samples,
            visible: 1..2,
            line: &[],
            y: VISIBLE_Y,
        };

        let edge = VISIBLE_Y.max - (VISIBLE_Y.max - VISIBLE_Y.min) * EDGE_MARKER_INSET;
        assert_eq!(run.entry(), Some(PlotPoint::new(T, edge)));
        assert_eq!(run.exit(), Some(PlotPoint::new(T + 2.0, edge)));
    }

    #[test]
    fn a_painted_run_connects_the_line_through_every_marker_of_the_run() {
        let markers = vec![
            (Placement::OffScaleAbove, PlotPoint::new(T + 2.0, 97.0)),
            (Placement::OffScaleAbove, PlotPoint::new(T + 3.0, 97.0)),
        ];

        let shapes = painted(vec![DrawnRun {
            markers,
            entry: Some(PlotPoint::new(T + 1.0, 0.0)),
            exit: Some(PlotPoint::new(T + 4.0, 0.0)),
        }]);

        let transform = paint_transform();
        let connectors = connectors(&shapes);
        let [entry, along, exit] = connectors.as_slice() else {
            panic!("expected three connectors, got {}", connectors.len());
        };
        assert_eq!(
            entry[0],
            transform.position_from_point(&PlotPoint::new(T + 1.0, 0.0)),
            "the first connector starts at the line point before the run"
        );
        assert_eq!(entry[1], along[0], "the entry ends at the first marker");
        assert_eq!(along[1], exit[0], "the exit starts at the last marker");
        assert_eq!(
            exit[1],
            transform.position_from_point(&PlotPoint::new(T + 4.0, 0.0)),
            "the last connector ends at the line point after the run"
        );
        assert!(
            entry[0].y > entry[1].y,
            "the connector runs up to the marker at the top edge, from {:?} to {:?}",
            entry[0],
            entry[1]
        );
    }

    #[test]
    fn a_marker_at_a_boundary_is_drawn_whole_inside_the_plot_area() {
        const EDGE_Y: f64 = 97.0;
        let transform = paint_transform();
        let frame = *transform.frame();

        let shapes = painted(vec![DrawnRun {
            markers: vec![
                (Placement::OffScaleAbove, PlotPoint::new(T, EDGE_Y)),
                (Placement::OffScaleAbove, PlotPoint::new(T + 10.0, EDGE_Y)),
            ],
            entry: None,
            exit: None,
        }]);

        let center_y = transform.position_from_point(&PlotPoint::new(T, EDGE_Y)).y;
        let glyph_around = |x: f32| {
            Rect::from_min_max(
                Pos2::new(x - MARKER_HALF_WIDTH, center_y - MARKER_HALF_WIDTH),
                Pos2::new(x + MARKER_HALF_WIDTH, center_y + MARKER_HALF_WIDTH),
            )
        };
        assert_eq!(
            glyph_bounds(&shapes),
            vec![
                glyph_around(frame.left() + MARKER_HALF_WIDTH),
                glyph_around(frame.right() - MARKER_HALF_WIDTH),
            ]
        );
    }

    #[rstest::rstest]
    #[case::below(-4_127_054.0, Placement::OffScaleBelow)]
    #[case::above(500.0, Placement::OffScaleAbove)]
    #[case::inside(-220.0, Placement::InView)]
    #[case::on_the_edge(-300.0, Placement::InView)]
    fn placement_follows_the_visible_range(#[case] value: f64, #[case] expected: Placement) {
        let y = VisibleYRange {
            min: -300.0,
            max: -100.0,
        };

        assert_eq!(Placement::resolve(value, y), expected);
    }

    #[test]
    fn markers_closer_than_a_glyph_width_draw_as_one() {
        let excursion = excursion();
        let placed: Vec<PlacedMarker<'_>> = (0..10)
            .map(|index| {
                PlacedMarker::new(
                    ExcursionSample {
                        index,
                        t: T + index as f64,
                        offset_ms: 0,
                    },
                    OffScaleCause::Excursion(&excursion),
                    VisibleYRange {
                        min: -1.0,
                        max: 1.0,
                    },
                )
            })
            .collect();

        // Every third marker clears the spacing of eight, at three points of
        // screen per second.
        let drawn = spaced_markers(&placed, |at| ((at.x - T) * 3.0) as f32);

        assert_eq!(drawn.len(), 4);
    }

    /// The hover reports the sample's real offset and the host stamp it implies,
    /// not the clamped edge value the marker is drawn at.
    #[test]
    fn the_hover_reports_the_true_offset_and_system_stamp() {
        let excursion = excursion();
        let sample = *excursion.peak();
        let hover = ClockOffsetHover::new(
            Some("ride.gtd"),
            sample,
            OffScaleCause::Excursion(&excursion),
        );
        assert_eq!(hover.gps_time, "12:00:00");
        assert_eq!(hover.sys_time, "13:08:47.054");
        assert_eq!(hover.offset, "\u{2212}1h8m47s");
        assert_eq!(
            hover.cause,
            FormattedCause::Excursion {
                baseline: "\u{2212}234ms".to_owned(),
                samples: 1,
                returned_to_baseline: true,
            }
        );
        assert_eq!(hover.track.as_deref(), Some("ride.gtd"));
    }

    /// The hover states the date of each clock: a host clock left at its
    /// power-on default stamps every fix decades from the receiver's epoch.
    #[test]
    fn the_hover_of_an_off_scale_baseline_dates_both_clocks() {
        const OFFSET_MS: i64 = 1_767_225_600_000;
        let sample = ExcursionSample {
            index: 0,
            t: T,
            offset_ms: OFFSET_MS,
        };

        let hover = ClockOffsetHover::new(None, sample, OffScaleCause::Baseline);

        assert_eq!(hover.gps_time, "2024-01-15 12:00:00 UTC");
        assert_eq!(hover.sys_time, "1968-01-15 12:00:00.000 UTC");
        assert_eq!(hover.cause, FormattedCause::Baseline);
    }
}
