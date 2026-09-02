//! The clock offset excursion overlay: an off-scale indicator for each sample
//! whose GPS↔system offset left the track's baseline, plus its hover text.
//!
//! These samples are held out of the clock-offset line (see
//! [`crate::series::TrackSeries::clock_delta_ms`]) because a departure of
//! hours or days would set the auto-bounds of the y-axis every metric shares.
//! Each one is marked here at the edge it ran off, with its true offset on
//! hover, and drawn in place once the view is zoomed out far enough to hold
//! it.

use chrono::DateTime;
use egui::epaint::{Shape, Stroke};
use egui::{Color32, Pos2, Ui, Vec2};
use egui_plot::{PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotPoint, PlotTransform};
use gt_analysis::clock_offset::{ClockOffsetExcursion, ExcursionSample};
use gt_types::MetricKind;

use super::chips::MetricVisibility;
use super::lines::{
    ANOMALY_HOVER_RADIUS_PX, ANOMALY_MARKER_RADIUS, NearestHoverLabel, PlotHoverLabel, visible_by_x,
};
use crate::series::TrackSeries;

/// How far inside the plot's edge an off-scale marker sits, as a fraction of
/// the visible y range.  Keeps the whole glyph on screen.
const EDGE_INSET: f64 = 0.03;

/// Half-width of a marker glyph, in points.
const MARKER_HALF_WIDTH: f32 = ANOMALY_MARKER_RADIUS;

/// Width of the tail drawn behind an off-scale marker.
const TAIL_WIDTH: f32 = 1.0;

/// Length of that tail, in points.  Short on purpose: a full-height line would
/// read as a cursor, and the plot already has two of those.
const TAIL_LENGTH: f32 = 22.0;

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
    fn resolve(value: f64, y_min: f64, y_max: f64) -> Self {
        if value < y_min {
            Self::OffScaleBelow
        } else if value > y_max {
            Self::OffScaleAbove
        } else {
            Self::InView
        }
    }

    /// Y in plot coordinates for a sample worth `value`, given the visible
    /// range: the near edge when it runs off, the value itself when it fits.
    fn place(self, value: f64, y_min: f64, y_max: f64) -> f64 {
        let inset = (y_max - y_min) * EDGE_INSET;
        match self {
            Self::OffScaleBelow => y_min + inset,
            Self::OffScaleAbove => y_max - inset,
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

/// The excursion indicators of one track, as a plot item that contributes
/// nothing to the plot's auto-bounds.
///
/// An off-scale marker is positioned from the edge of the current view, so
/// letting it feed the bounds pushes that edge a little further out every
/// frame and the view creeps outward for as long as the plot is open.
/// `bounds()` returns [`PlotBounds::NOTHING`], as egui_plot's own `Span` does,
/// which breaks the loop at the source.
struct OffScaleMarkers {
    base: PlotItemBase,
    /// Placed markers, in ascending x.
    markers: Vec<(Placement, PlotPoint)>,
    color: Color32,
}

impl PlotItem for OffScaleMarkers {
    fn shapes(&self, _ui: &Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
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

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn color(&self) -> Color32 {
        self.color
    }

    /// No geometry: hovering is handled by [`ClockExcursionHover`], which
    /// reports the sample's real offset rather than the edge it is drawn at.
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }

    fn bounds(&self) -> PlotBounds {
        PlotBounds::NOTHING
    }

    fn base(&self) -> &PlotItemBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
}

/// The frame-level inputs the excursion overlay needs beyond the track itself:
/// the visible x range it clips to, the metric visibility it gates on, and the
/// theme.
#[derive(Clone, Copy)]
pub(super) struct ExcursionViewport<'v> {
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    pub(super) metric_vis: &'v MetricVisibility,
    pub(super) dark_mode: bool,
}

/// Draw the clock offset excursion indicators for one track and, when the
/// pointer is within [`ANOMALY_HOVER_RADIUS_PX`] of one, record the nearest in
/// `nearest` so the caller can show its tooltip.
///
/// The indicators annotate the clock-offset line, so they follow its chip:
/// with the metric hidden this draws nothing.
pub(super) fn add_clock_excursions(
    plot_ui: &mut egui_plot::PlotUi<'_>,
    series: &TrackSeries,
    track_label: Option<&str>,
    viewport: ExcursionViewport<'_>,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
) {
    if series.clock_excursions.is_empty() || !viewport.metric_vis.field(MetricKind::ClockDeltaMs) {
        return;
    }
    let bounds = plot_ui.plot_bounds();
    let [_, y_min] = bounds.min();
    let [_, y_max] = bounds.max();
    let color = gt_ui_theme::warning_amber(viewport.dark_mode);

    // Marker and hover must land on the same pixel for the tooltip to track
    // the glyph.
    let mut markers: Vec<(Placement, PlotPoint)> = Vec::new();
    let mut hovers: Vec<(PlotPoint, ClockExcursionHover)> = Vec::new();

    for excursion in &series.clock_excursions {
        let samples = visible_by_x(
            excursion.samples.as_slice(),
            |s| s.t,
            viewport.x_min,
            viewport.x_max,
        );
        for sample in samples {
            let value = sample.offset_ms as f64;
            let placement = Placement::resolve(value, y_min, y_max);
            let at = PlotPoint::new(sample.t, placement.place(value, y_min, y_max));
            markers.push((placement, at));
            hovers.push((
                at,
                ClockExcursionHover::new(track_label, excursion, *sample),
            ));
        }
    }
    if markers.is_empty() {
        return;
    }

    plot_ui.add(OffScaleMarkers {
        base: PlotItemBase::new("Clock offset excursion".to_owned()),
        markers,
        color,
    });

    let Some(ptr) = pointer else {
        return;
    };
    for (at, hover) in hovers {
        let dist = plot_ui.screen_from_plot(at).distance(ptr);
        if dist <= ANOMALY_HOVER_RADIUS_PX {
            nearest.offer(dist, || PlotHoverLabel::ClockExcursion(hover));
        }
    }
}

/// Pre-formatted tooltip contents for one clock offset excursion sample.
pub(super) struct ClockExcursionHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    gps_time: String,
    sys_time: String,
    offset: String,
    baseline: String,
    /// How many samples the excursion this sample belongs to spans.
    samples: usize,
}

impl ClockExcursionHover {
    fn new(
        track_label: Option<&str>,
        excursion: &ClockOffsetExcursion,
        sample: ExcursionSample,
    ) -> Self {
        let gps_ms = (sample.t * 1000.0) as i64;
        Self {
            track: track_label.map(ToOwned::to_owned),
            gps_time: format_ms(gps_ms, "%H:%M:%S"),
            // The offset is GPS−system, so the host stamp is the GPS epoch
            // less the offset.
            sys_time: format_ms(gps_ms.saturating_sub(sample.offset_ms), "%H:%M:%S%.3f"),
            offset: gt_fmt::format_signed_delta(sample.offset_ms),
            baseline: gt_fmt::format_signed_delta(excursion.baseline_ms),
            samples: excursion.samples.len(),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong("Clock offset excursion");
        if let Some(track) = &self.track {
            ui.label(track);
        }
        ui.label(format!("GPS epoch {}", self.gps_time));
        ui.label(format!("System timestamp {}", self.sys_time));
        ui.separator();
        ui.label(format!("Offset {}", self.offset));
        ui.label(format!("Track baseline {}", self.baseline));
        ui.separator();
        ui.label(format!(
            "The offset left the track's baseline for {} {} and returned.",
            self.samples,
            gt_fmt::pluralize(self.samples, "sample", "samples"),
        ));
    }
}

/// Format a Unix-millisecond timestamp, or an empty string when it is out of
/// range for a date.
fn format_ms(ms: i64, fmt: &str) -> String {
    DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format(fmt).to_string())
        .unwrap_or_default()
}

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

    /// The overlay must stay out of the plot's auto-bounds.  An off-scale
    /// marker is placed from the edge of the current view, so contributing that
    /// position back to the bounds pushes the edge further out every frame and
    /// the view creeps outward for as long as the plot is open.
    #[test]
    fn the_markers_contribute_no_bounds() {
        let item = OffScaleMarkers {
            base: PlotItemBase::new("Clock offset excursion".to_owned()),
            markers: vec![(Placement::OffScaleBelow, PlotPoint::new(T, -4_127_054.0))],
            color: egui::Color32::WHITE,
        };
        assert_eq!(item.bounds(), PlotBounds::NOTHING);
    }

    #[rstest::rstest]
    #[case::below(-4_127_054.0, Placement::OffScaleBelow)]
    #[case::above(500.0, Placement::OffScaleAbove)]
    #[case::inside(-220.0, Placement::InView)]
    #[case::on_the_edge(-300.0, Placement::InView)]
    fn placement_follows_the_visible_range(#[case] value: f64, #[case] expected: Placement) {
        assert_eq!(Placement::resolve(value, -300.0, -100.0), expected);
    }

    /// The hover reports the sample's real offset and the host stamp it implies,
    /// not the clamped edge value the marker is drawn at.
    #[test]
    fn the_hover_reports_the_true_offset_and_system_stamp() {
        let excursion = excursion();
        let sample = *excursion.peak();
        let hover = ClockExcursionHover::new(Some("ride.gtd"), &excursion, sample);
        assert_eq!(hover.gps_time, "12:00:00");
        assert_eq!(hover.sys_time, "13:08:47.054");
        assert_eq!(hover.offset, "\u{2212}1h8m47s");
        assert_eq!(hover.baseline, "\u{2212}234ms");
        assert_eq!(hover.samples, 1);
        assert_eq!(hover.track.as_deref(), Some("ride.gtd"));
    }
}
