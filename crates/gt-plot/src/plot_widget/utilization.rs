use std::ops::RangeInclusive;

use chrono::DateTime;
use egui_plot::{MarkerShape, PlotPoint, PlotPoints, Points};
use gt_analysis::satellite_utilization::UtilAnomaly;

use super::hover::{self, ANOMALY_HOVER_RADIUS_PX, NearestHoverLabel, PlotHoverLabel};
use super::metric_paint::ANOMALY_MARKER_RADIUS;
use crate::series::TrackSeries;

pub(super) struct AnomalyHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    time: String,
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

    pub(super) fn show(&self, ui: &mut egui::Ui) {
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

/// Markers use the all-constellation ratio at each report epoch.
pub(super) fn add_util_anomalies<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    series: &'a TrackSeries,
    track_label: Option<&str>,
    x_range: RangeInclusive<f64>,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
    dark_mode: bool,
) {
    let visible = hover::visible_by_x(&series.util_anomalies, |a| a.t, x_range);
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
            // Suppress native hover for the shared custom tooltip.
            .allow_hover(false),
    );

    let Some(pointer) = pointer else {
        return;
    };
    if let Some((distance, anomaly)) = hover::nearest_fix_under_pointer(
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
