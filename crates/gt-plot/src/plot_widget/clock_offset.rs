//! The off-scale clock offset overlay: an indicator for each sample the plot
//! holds off its clock offset line, plus its hover text.
//!
//! A sample is held off the line (see
//! [`crate::series::TrackSeries::clock_delta_ms`]) when its offset departed
//! from the track's baseline and returned, or when the track's baseline itself
//! lies outside [`gt_analysis::clock_offset::MAX_PLOTTED_OFFSET_S`]. Either
//! offset would set the auto-bounds of the y-axis every metric shares. Each
//! held-back sample is marked here at the edge it ran off, with its true offset
//! on hover, and drawn in place once the view is zoomed out far enough to hold
//! it.

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
use super::overlay::{EDGE_MARKER_INSET, OverlayItem, OverlayPainter, TAIL_LENGTH};
use crate::series::TrackSeries;

/// Half-width of a marker glyph, in points.
const MARKER_HALF_WIDTH: f32 = ANOMALY_MARKER_RADIUS;

/// Width of the tail drawn behind an off-scale marker.
const TAIL_WIDTH: f32 = 1.0;

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

    /// Screen-space offset from the marker to the far end of its tail, or
    /// `None` for a marker drawn at its real value, which needs none.
    fn tail(self) -> Option<Vec2> {
        match self {
            Self::OffScaleBelow => Some(Vec2::new(0.0, -TAIL_LENGTH)),
            Self::OffScaleAbove => Some(Vec2::new(0.0, TAIL_LENGTH)),
            Self::InView => None,
        }
    }
}

/// What put one sample off the shared y-axis.
#[derive(Clone, Copy)]
enum OffScaleCause<'a> {
    /// The track's baseline lies outside
    /// [`gt_analysis::clock_offset::MAX_PLOTTED_OFFSET_S`].
    Baseline,
    /// The offset departed from the track's baseline and returned to it.
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
        let value = sample.offset_ms as f64;
        let placement = Placement::resolve(value, y);
        Self {
            placement,
            at: PlotPoint::new(sample.t, placement.place(value, y)),
            sample,
            cause,
        }
    }
}

/// The off-scale indicators of one track.
struct OffScaleMarkers {
    /// Placed markers, in ascending x.
    markers: Vec<(Placement, PlotPoint)>,
    color: Color32,
}

impl OverlayPainter for OffScaleMarkers {
    fn legend_color(&self) -> Color32 {
        self.color
    }

    fn paint(&self, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        for &(placement, at) in &self.markers {
            let center = transform.position_from_point(&at);
            if let Some(tail) = placement.tail() {
                shapes.push(Shape::line_segment(
                    [center, center + tail],
                    Stroke::new(TAIL_WIDTH, self.color),
                ));
            }
            shapes.push(Shape::convex_polygon(
                placement.glyph(center),
                self.color,
                Stroke::NONE,
            ));
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
    let visible = |samples| {
        lines::visible_by_x(
            samples,
            |s: &ExcursionSample| s.t,
            viewport.x_min,
            viewport.x_max,
        )
    };
    let (title, placed): (&str, Vec<PlacedMarker<'_>>) = match &series.clock_offset_placement {
        ClockOffsetPlacement::BaselineOffScale(off_scale) => (
            OFF_SCALE_BASELINE_TITLE,
            visible(off_scale.samples.as_slice())
                .iter()
                .map(|&sample| PlacedMarker::new(sample, OffScaleCause::Baseline, y))
                .collect(),
        ),
        ClockOffsetPlacement::BaselineOnScale(excursions) => (
            EXCURSION_TITLE,
            excursions
                .iter()
                .flat_map(|excursion| {
                    visible(excursion.samples.as_slice())
                        .iter()
                        .map(move |&sample| {
                            PlacedMarker::new(sample, OffScaleCause::Excursion(excursion), y)
                        })
                })
                .collect(),
        ),
    };
    if placed.is_empty() {
        return;
    }

    plot_ui.add(OverlayItem::new(
        title,
        OffScaleMarkers {
            markers: spaced_markers(&placed, |at| plot_ui.screen_from_plot(at).x),
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
    /// The offset departed from the track's baseline and returned to it.
    Excursion {
        /// The track's baseline offset.
        baseline: String,
        /// How many samples the excursion this sample belongs to spans.
        samples: usize,
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
        let FormattedCause::Excursion { baseline, samples } = &self.cause else {
            ui.separator();
            ui.label(
                "Every fix of this recording is marked at the edge. Zoom the y-axis out to \
                 draw the offset in place.",
            );
            return;
        };
        ui.label(format!("Track baseline {baseline}"));
        ui.separator();
        ui.label(format!(
            "The offset left the track's baseline for {samples} {} and returned.",
            gt_fmt::pluralize(*samples, "sample", "samples"),
        ));
    }
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
    use vec1::vec1;

    use super::*;

    /// 2024-01-15 12:00:00 UTC, the x of the excursion sample below.
    const T: f64 = 1_705_320_000.0;

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
        }
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
