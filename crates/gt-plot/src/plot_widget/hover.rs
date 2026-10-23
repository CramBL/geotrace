use std::ops::{Range, RangeInclusive};

use egui::Tooltip;
use egui_plot::{Line, PlotItem as _, PlotPoint, PlotPoints};

use super::backward_time_step::BackwardTimeStepHover;
use super::clock_offset::ClockOffsetHover;
use super::flares::SolarFlareHover;
use super::geomagnetic::GeomagneticHover;
use super::jamming::JammingHover;
use super::metric_hover::{MetricHover, MetricHoverSelection};
use super::snap_error::SnapErrorHover;
use super::tec::TecHover;
use super::utilization::AnomalyHover;

pub(super) enum PlotHoverLabel {
    Anomaly(AnomalyHover),
    BackwardTimeStep(BackwardTimeStepHover),
    ClockOffset(ClockOffsetHover),
    Geomagnetic(GeomagneticHover),
    Jamming(JammingHover),
    Metric(MetricHover),
    SnapError(SnapErrorHover),
    SolarFlare(SolarFlareHover),
    Tec(TecHover),
}

impl PlotHoverLabel {
    pub(super) fn metric_selection(&self) -> Option<MetricHoverSelection> {
        match self {
            Self::Metric(hover) => hover.selection(),
            _ => None,
        }
    }

    fn show(&self, ui: &mut egui::Ui) {
        match self {
            Self::Metric(hover) => hover.show(ui),
            Self::Anomaly(hover) => hover.show(ui),
            Self::SnapError(hover) => hover.show(ui),
            Self::Jamming(hover) => hover.show(ui),
            Self::Geomagnetic(hover) => hover.show(ui),
            Self::Tec(hover) => hover.show(ui),
            Self::ClockOffset(hover) => hover.show(ui),
            Self::BackwardTimeStep(hover) => hover.show(ui),
            Self::SolarFlare(hover) => hover.show(ui),
        }
    }
}

pub(super) struct NearestCandidate<T> {
    closest: Option<(f32, T)>,
}

impl<T> NearestCandidate<T> {
    /// The closure runs for the first candidate and for strictly closer replacements.
    pub(super) fn offer(&mut self, distance_px: f32, candidate: impl FnOnce() -> T) {
        if self
            .closest
            .as_ref()
            .is_none_or(|(closest, _)| distance_px < *closest)
        {
            self.closest = Some((distance_px, candidate()));
        }
    }

    pub(super) fn take(self) -> Option<T> {
        self.closest.map(|(_, candidate)| candidate)
    }
}

impl<T> Default for NearestCandidate<T> {
    fn default() -> Self {
        Self { closest: None }
    }
}

/// The plot displays one custom tooltip across all tracks and overlay types.
#[derive(Default)]
pub(super) struct NearestHoverLabel {
    labels: NearestCandidate<PlotHoverLabel>,
    native_line_distance: Option<f32>,
}

impl NearestHoverLabel {
    pub(super) fn offer(&mut self, distance_px: f32, candidate: impl FnOnce() -> PlotHoverLabel) {
        self.labels.offer(distance_px, candidate);
    }

    pub(super) fn offer_native_line(
        &mut self,
        plot_ui: &egui_plot::PlotUi<'_>,
        points: &[PlotPoint],
        pointer: Option<egui::Pos2>,
    ) {
        if points.is_empty() {
            return;
        }
        let Some(pointer) = pointer else {
            return;
        };
        let line = Line::new("", PlotPoints::Borrowed(points));
        if let Some(closest) = line.find_closest(pointer, plot_ui.transform())
            && closest.dist_sq
                <= plot_ui
                    .ctx()
                    .global_style()
                    .interaction
                    .interact_radius
                    .powi(2)
        {
            let distance = closest.dist_sq.sqrt();
            self.native_line_distance = Some(
                self.native_line_distance
                    .map_or(distance, |previous| previous.min(distance)),
            );
        }
    }

    pub(super) fn selected(&self) -> Option<&PlotHoverLabel> {
        let (distance, label) = self.labels.closest.as_ref()?;
        if matches!(label, PlotHoverLabel::Metric(_))
            && self
                .native_line_distance
                .is_some_and(|native| native < *distance)
        {
            return None;
        }
        Some(label)
    }

    pub(super) fn has_candidate(&self) -> bool {
        self.selected().is_some()
    }

    fn take(self) -> Option<PlotHoverLabel> {
        self.selected()?;
        self.labels.take()
    }
}

/// Marker inputs must be sorted by `key`. Both range endpoints are included.
pub(super) fn visible_by_x<T>(
    items: &[T],
    key: impl Fn(&T) -> f64,
    range: RangeInclusive<f64>,
) -> &[T] {
    items
        .get(visible_x_range(items, key, range))
        .unwrap_or_default()
}

pub(super) fn visible_x_range<T>(
    items: &[T],
    key: impl Fn(&T) -> f64,
    range: RangeInclusive<f64>,
) -> Range<usize> {
    let start = items.partition_point(|it| key(it) < *range.start());
    let end = items.partition_point(|it| key(it) <= *range.end());
    start..end
}

/// The distance callback may measure one axis for markers that span the other.
pub(super) fn nearest_under_pointer<T>(
    items: &[T],
    distance_px: impl Fn(&T) -> f32,
    radius_px: f32,
) -> Option<(f32, &T)> {
    items
        .iter()
        .map(|item| (distance_px(item), item))
        .filter(|&(distance, _)| distance <= radius_px)
        .min_by(|(left, _), (right, _)| left.total_cmp(right))
}

pub(super) fn nearest_fix_under_pointer<'a, T>(
    plot_ui: &egui_plot::PlotUi<'_>,
    fixes: &'a [T],
    at: impl Fn(&T) -> PlotPoint,
    pointer: egui::Pos2,
    radius_px: f32,
) -> Option<(f32, &'a T)> {
    nearest_under_pointer(
        fixes,
        |fix| plot_ui.screen_from_plot(at(fix)).distance(pointer),
        radius_px,
    )
}
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

pub(super) const ANOMALY_HOVER_RADIUS_PX: f32 = 7.0;
pub(super) const HOVER_RADIUS_PX: f32 = 12.0;
const HOVER_LABEL_TOOLTIP_GAP: f32 = 12.0;

#[cfg(test)]
mod tests {
    use super::NearestCandidate;

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
        assert_eq!(
            super::visible_by_x(&xs, |&x| x, x_min..=x_max),
            expected.as_slice()
        );
    }
}
