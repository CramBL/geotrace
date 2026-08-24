//! The metric and channel line pass: one line per enabled metric and
//! channel component, plus the anomaly markers and the custom hover labels.

use chrono::DateTime;
use egui::{Color32, Tooltip};
use egui_plot::{Line, LineStyle, MarkerShape, PlotPoint, PlotPoints, Points};
use gt_analysis::satellite_utilization::UtilAnomaly;
use gt_types::MetricKind;
use gt_types::satellites::ConstellationSet;
use gt_ui_types::HighlightScope;
use rustc_hash::FxHashMap;
use strum::IntoEnumIterator;

use super::chips::{
    ChannelVisibility, HoveredChip, LoadedChannel, MetricKindUi, MetricVisibility, SectionGates,
    metric_is_shown,
};
use super::clock_excursion::ClockExcursionHover;
use super::flares::SolarFlareHover;
use super::geomagnetic::GeomagneticHover;
use super::jamming::JammingHover;
use super::levels::{LineViewport, TrackLevelCache};
use super::snap_error::{
    SnapErrorHover, SnapErrorPlotCache, SnapErrorStyle, add_snap_error_series,
};
use super::style::{
    channel_line_color, effective_component_color, file_line_style, metric_line_color,
};
use super::tec::TecHover;
use crate::series::{PlacedTrackSeries, TrackSeries};

/// The sub-slice of `items` - sorted ascending by `key` - whose key lies in
/// the visible `[x_min, x_max]` range. Marker overlays clip to this so they
/// draw and hover-test only what is on screen, not the whole track's markers
/// every frame.
pub(super) fn visible_by_x<T>(
    items: &[T],
    key: impl Fn(&T) -> f64,
    x_min: f64,
    x_max: f64,
) -> &[T] {
    let start = items.partition_point(|it| key(it) < x_min);
    let end = items.partition_point(|it| key(it) <= x_max);
    items.get(start..end).unwrap_or_default()
}

/// Format of the time a per-fix hover label reports, as the archived sources
/// write their own epochs.
pub(super) const HOVER_INSTANT_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// Pixel radius within which the pointer is considered to be hovering a
/// masked-satellite anomaly marker.
pub(super) const ANOMALY_HOVER_RADIUS_PX: f32 = 7.0;
/// Pixel radius within which a fix of a per-fix line is a hover target.
pub(super) const HOVER_RADIUS_PX: f32 = 12.0;

/// The item of `fixes` closest to `pointer` within `radius_px`, and its pixel
/// distance for [`NearestCandidate::offer`]. `at` places one item in plot
/// space.
pub(super) fn nearest_fix_under_pointer<'a, T>(
    plot_ui: &egui_plot::PlotUi<'_>,
    fixes: &'a [T],
    at: impl Fn(&T) -> PlotPoint,
    pointer: egui::Pos2,
    radius_px: f32,
) -> Option<(f32, &'a T)> {
    fixes
        .iter()
        .map(|fix| (plot_ui.screen_from_plot(at(fix)).distance(pointer), fix))
        .filter(|&(distance, _)| distance <= radius_px)
        .min_by(|(left, _), (right, _)| left.total_cmp(right))
}
/// On-plot radius of the anomaly cross marker.
pub(super) const ANOMALY_MARKER_RADIUS: f32 = 4.0;
/// Gap between the pointer and the custom hover label.
const HOVER_LABEL_TOOLTIP_GAP: f32 = 12.0;
/// Add all metric lines for one track to the plot using pre-computed level selections.
///
/// When `hovered_chip` is `Some(kind)`, that metric is highlighted (double stroke
/// width) and every other line is dimmed to 20 % brightness, matching the
/// standard egui-plot legend hover behaviour.
///
/// The `'a` lifetime ties both `plot_ui` and `series` together so that
/// [`egui_plot::PlotPoints::Borrowed`] can reference mipmap slices directly
/// without any per-frame allocation.
#[expect(
    clippy::too_many_arguments,
    reason = "per-line rendering needs the plot, series, level cache, visibility/hover state, track focus, and the advanced-section gate"
)]
pub(super) fn add_series_lines<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    placed: &'a PlacedTrackSeries,
    // The recording's plot label, `None` while a single track is visible and
    // nothing needs naming.
    track_label: Option<&str>,
    cache: &TrackLevelCache,
    metric_vis: &MetricVisibility,
    channel_vis: &ChannelVisibility,
    component_colors: &FxHashMap<String, Vec<Option<Color32>>>,
    present: ConstellationSet,
    channels: &[LoadedChannel],
    hovered_chip: Option<&HoveredChip>,
    hover_scope: Option<HighlightScope>,
    sections: SectionGates,
    line_width: f32,
    dark_mode: bool,
    // Snap error is resolved outside the recording and draws from its own
    // cache.
    snap_error: Option<&'a SnapErrorPlotCache>,
    // Where the pointer is, for the snap error line's own hit-testing.
    pointer: Option<egui::Pos2>,
    viewport: LineViewport,
    nearest: &mut NearestHoverLabel,
) {
    let prefix = track_label.map_or_else(String::new, |label| format!("{label}: "));
    let focused = placed.matches_hover_scope(hover_scope);
    let has_track_focus = hover_scope.is_some();

    let line_style = file_line_style(placed.fi);
    // The hover-dim treatment every line shares: full color plus highlight
    // while its own chip is hovered, dimmed while any other chip is.
    let stroke_with_hover_treatment = |base: Color32, is_hovered_chip: bool| {
        let (mut color, highlighted) = match hovered_chip {
            Some(_) if is_hovered_chip => (base, true),
            Some(_) => (base.gamma_multiply(0.2), false),
            None => (base, false),
        };
        if has_track_focus && !focused {
            color = color.gamma_multiply(0.2);
        }
        LineStroke {
            color,
            style: line_style,
            width: line_width,
            highlighted: highlighted || (has_track_focus && focused),
        }
    };

    for kind in MetricKind::iter() {
        // Skip metrics with no chip on screen - collapsed advanced section, or a
        // per-constellation metric whose constellation is absent from the data -
        // so a hidden chip never leaves a stray line on the plot.
        if !metric_is_shown(kind, present, sections.show_advanced) {
            continue;
        }
        if !metric_vis.field(kind) {
            continue;
        }
        let is_hovered = hovered_chip == Some(&HoveredChip::Metric(kind));
        let stroke =
            stroke_with_hover_treatment(metric_line_color(kind, placed.fi, dark_mode), is_hovered);
        // Snap error has no mipmap. It draws from the external per-run series
        // right after this loop.
        let (Some(mipmap), Some(level)) = (placed.series.mipmap_for(kind), cache.level_for(kind))
        else {
            continue;
        };
        add_line(
            plot_ui,
            mipmap.slice_at(level),
            format!("{prefix}{}", kind.label()),
            stroke,
        );
    }

    if metric_vis.field(MetricKind::SnapError)
        && let Some(cache) = snap_error
    {
        add_snap_error_series(
            plot_ui,
            &prefix,
            track_label,
            cache,
            viewport,
            pointer,
            nearest,
            SnapErrorStyle {
                stroke: stroke_with_hover_treatment(
                    metric_line_color(MetricKind::SnapError, placed.fi, dark_mode),
                    hovered_chip == Some(&HoveredChip::Metric(MetricKind::SnapError)),
                ),
                dark_mode,
            },
        );
    }

    // Channel lines, one per component, gated like the chips: the whole
    // section while collapsed, then the per-channel toggle.
    if !sections.show_channels {
        return;
    }
    for (channel, selections) in placed.series.channels.iter().zip(&cache.channels) {
        if !channel_vis.is_visible(&channel.name) {
            continue;
        }
        // The palette index comes from the cross-file union, so `accel` keeps
        // one hue no matter which file's track is drawing.
        let Some(color_index) = channels
            .iter()
            .find(|c| c.name == channel.name)
            .map(|c| c.color_index)
        else {
            continue;
        };
        let is_hovered =
            matches!(hovered_chip, Some(HoveredChip::Channel(name)) if *name == channel.name);
        let base = channel_line_color(color_index, placed.fi);
        let unit_suffix = channel
            .unit
            .as_deref()
            .map_or(String::new(), |u| format!(" ({u})"));
        for (index, (component, selection)) in channel.components.iter().zip(selections).enumerate()
        {
            // Rotate before the hover treatment, so dimming applies to the
            // component's own hue.
            let stroke = stroke_with_hover_treatment(
                effective_component_color(component_colors, &channel.name, base, index),
                is_hovered,
            );
            add_line(
                plot_ui,
                component.mipmap.slice_at(*selection),
                format!("{prefix}{}{unit_suffix}", component.label),
                stroke,
            );
        }
    }
}

/// One custom hover label, for the plot items whose message egui_plot's own
/// label cannot carry.
pub(super) enum PlotHoverLabel {
    Anomaly(AnomalyHover),
    SnapError(SnapErrorHover),
    Jamming(JammingHover),
    Geomagnetic(GeomagneticHover),
    Tec(TecHover),
    ClockExcursion(ClockExcursionHover),
    SolarFlare(SolarFlareHover),
}

impl PlotHoverLabel {
    fn show(&self, ui: &mut egui::Ui) {
        match self {
            Self::Anomaly(hover) => hover.show(ui),
            Self::SnapError(hover) => hover.show(ui),
            Self::Jamming(hover) => hover.show(ui),
            Self::Geomagnetic(hover) => hover.show(ui),
            Self::Tec(hover) => hover.show(ui),
            Self::ClockExcursion(hover) => hover.show(ui),
            Self::SolarFlare(hover) => hover.show(ui),
        }
    }
}

/// Keeps the closest of the candidates offered to it, by pixel distance from
/// the pointer.
pub(super) struct NearestCandidate<T> {
    closest: Option<(f32, T)>,
}

impl<T> Default for NearestCandidate<T> {
    fn default() -> Self {
        Self { closest: None }
    }
}

impl<T> NearestCandidate<T> {
    /// `candidate` is built only when `distance_px` is closer than the held
    /// candidate.
    pub(super) fn offer(&mut self, distance_px: f32, candidate: impl FnOnce() -> T) {
        if self
            .closest
            .as_ref()
            .is_none_or(|(closest, _)| distance_px < *closest)
        {
            self.closest = Some((distance_px, candidate()));
        }
    }

    pub(super) fn has_candidate(&self) -> bool {
        self.closest.is_some()
    }

    pub(super) fn take(self) -> Option<T> {
        self.closest.map(|(_, candidate)| candidate)
    }
}

/// One shared slot for the custom hover labels of every series and every
/// recording: they all anchor at the pointer, so only the closest one draws.
pub(super) type NearestHoverLabel = NearestCandidate<PlotHoverLabel>;

/// Surface the closest custom hover label as a tooltip anchored at the pointer.
/// This is the only label at the cursor: egui_plot's cursor label is suppressed
/// for the frame a candidate is offered.
pub(super) fn show_nearest_hover_label(
    ui: &egui::Ui,
    response: &egui::Response,
    nearest: NearestHoverLabel,
) {
    if !response.hovered() {
        return;
    }
    let Some(label) = nearest.take() else {
        return;
    };
    Tooltip::always_open(
        ui.ctx().clone(),
        response.layer_id,
        egui::Id::new("plot_hover_label"),
        egui::PopupAnchor::Pointer,
    )
    .gap(HOVER_LABEL_TOOLTIP_GAP)
    .show(|ui| label.show(ui));
}

impl PlacedTrackSeries {
    fn matches_hover_scope(&self, hover_scope: Option<HighlightScope>) -> bool {
        match hover_scope {
            Some(HighlightScope::File { file_index }) => file_index.as_usize() == self.fi,
            Some(HighlightScope::Track(track))
            | Some(HighlightScope::TrackCategory { track, .. }) => {
                track.fi.as_usize() == self.fi && track.index.as_usize() == self.series.ti
            }
            Some(HighlightScope::Point(_)) | None => true,
        }
    }
}

/// How one line is stroked: its color after the hover treatment, the file's
/// dash pattern, the user's line width, and whether it draws highlighted.
#[derive(Debug, Clone, Copy)]
pub(super) struct LineStroke {
    pub(super) color: Color32,
    pub(super) style: LineStyle,
    pub(super) width: f32,
    pub(super) highlighted: bool,
}

/// Submit one metric line to the plot, borrowing the point slice directly via
/// [`PlotPoints::Borrowed`] - no allocation.
///
/// Skips slices with fewer than two points: a single-point line produces no
/// visible geometry and would clutter the legend.
///
/// The shared lifetime `'a` ensures the borrowed slice lives at least as long
/// as the `PlotUi` that will consume it (required because `PlotUi<'a>` is
/// invariant over `'a`).
pub(super) fn add_line<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    data: &'a [egui_plot::PlotPoint],
    name: String,
    stroke: LineStroke,
) {
    if data.len() < 2 {
        return;
    }
    plot_ui.line(
        Line::new(name, PlotPoints::Borrowed(data))
            .color(stroke.color)
            .style(stroke.style)
            .width(stroke.width)
            .highlight(stroke.highlighted),
    );
}

/// Pre-formatted tooltip contents for one masked-satellite anomaly marker.
pub(super) struct AnomalyHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    time: String,
    /// One line per masked-out satellite, e.g. `GPS 07 - 12.3°`.
    sats: Vec<String>,
}

impl AnomalyHover {
    fn new(track_label: Option<&str>, anomaly: &UtilAnomaly) -> Self {
        let time = DateTime::from_timestamp(anomaly.t as i64, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default();
        let sats = anomaly
            .masked
            .iter()
            .map(|m| {
                format!(
                    "{} {:02} - {:.1}°",
                    m.constellation.display_name(),
                    m.prn,
                    m.elevation
                )
            })
            .collect();
        Self {
            track: track_label.map(ToOwned::to_owned),
            time,
            sats,
        }
    }

    fn show(&self, ui: &mut egui::Ui) {
        ui.strong("Used satellites below the elevation mask");
        if let Some(track) = &self.track {
            ui.label(track);
        }
        ui.label(format!("at {}", self.time));
        ui.separator();
        for line in &self.sats {
            ui.label(line);
        }
    }
}

/// Draw the masked-satellite anomaly markers for one track and, when the pointer
/// is within [`ANOMALY_HOVER_RADIUS_PX`] of a marker, record the nearest one in
/// `nearest` so the caller can show its tooltip.
///
/// Each marker sits on the all-constellations utilization line at the epoch
/// where the receiver used a satellite below the elevation mask.
pub(super) fn add_util_anomalies<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    series: &'a TrackSeries,
    track_label: Option<&str>,
    x_range: std::ops::RangeInclusive<f64>,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
    dark_mode: bool,
) {
    let visible = visible_by_x(
        &series.util_anomalies,
        |a| a.t,
        *x_range.start(),
        *x_range.end(),
    );
    if visible.is_empty() {
        return;
    }

    let points: Vec<PlotPoint> = visible
        .iter()
        .map(|a| PlotPoint::new(a.t, a.value))
        .collect();
    plot_ui.points(
        Points::new("Masked-out used satellites", PlotPoints::Owned(points))
            .shape(MarkerShape::Cross)
            .color(gt_ui_theme::error_indicator(dark_mode))
            .radius(ANOMALY_MARKER_RADIUS)
            // Hover is handled with a custom tooltip, so suppress egui_plot's own.
            .allow_hover(false),
    );

    let Some(pointer) = pointer else {
        return;
    };
    if let Some((distance, anomaly)) = nearest_fix_under_pointer(
        plot_ui,
        visible,
        |anomaly| PlotPoint::new(anomaly.t, anomaly.value),
        pointer,
        ANOMALY_HOVER_RADIUS_PX,
    ) {
        nearest.offer(distance, || {
            PlotHoverLabel::Anomaly(AnomalyHover::new(track_label, anomaly))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{NearestCandidate, visible_by_x};

    /// Every series of every recording offers into one slot, so the closest
    /// candidate is the only one left to draw - the label anchors at the
    /// pointer, where a second one would land on top of it.
    #[test]
    fn only_the_closest_candidate_survives() {
        let mut nearest = NearestCandidate::default();
        nearest.offer(9.0, || "first recording's interference fix");
        nearest.offer(3.0, || "second recording's interference fix");
        nearest.offer(7.0, || "unsnapped point");
        assert_eq!(nearest.take(), Some("second recording's interference fix"));
    }

    #[test]
    fn nothing_offered_leaves_no_label() {
        assert_eq!(NearestCandidate::<&str>::default().take(), None);
    }

    #[test]
    fn a_losing_candidate_is_never_built() {
        let builds = std::cell::Cell::new(0_u32);
        let mut nearest = NearestCandidate::default();
        nearest.offer(3.0, || {
            builds.set(builds.get() + 1);
            "closest"
        });
        nearest.offer(8.0, || {
            builds.set(builds.get() + 1);
            "further away"
        });
        assert_eq!(builds.get(), 1);
        assert_eq!(nearest.take(), Some("closest"));
    }

    /// The viewport clip keeps exactly the markers whose x lies in the closed
    /// `[x_min, x_max]` range, and yields an empty slice when the range sits
    /// entirely outside the data (never a panic).
    #[rstest::rstest]
    #[case::interior_inclusive(1.0, 3.0, vec![1.0, 2.0, 3.0])]
    #[case::fractional_window(0.5, 2.5, vec![1.0, 2.0])]
    #[case::whole_range(-10.0, 10.0, vec![0.0, 1.0, 2.0, 3.0, 4.0])]
    #[case::entirely_right(5.0, 9.0, vec![])]
    #[case::entirely_left(-9.0, -5.0, vec![])]
    fn visible_by_x_clips_to_the_closed_range(
        #[case] x_min: f64,
        #[case] x_max: f64,
        #[case] expected: Vec<f64>,
    ) {
        let xs = [0.0_f64, 1.0, 2.0, 3.0, 4.0];
        assert_eq!(visible_by_x(&xs, |&x| x, x_min, x_max), expected.as_slice());
    }
}
