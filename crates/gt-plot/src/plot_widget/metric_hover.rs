use chrono::{DateTime, Utc};
use egui_plot::{Line, PlotItem as _, PlotPoint, PlotPoints};
use gt_types::{FixRef, LoadedTrack, MetricKind, PointIdx, TrackRef};

use super::chips::MetricKindUi;
use super::hover::HOVER_RADIUS_PX;
use crate::metric_series::{MetricSampleSource, MetricSeries};

pub(super) struct MetricHoverContext<'a> {
    pub(super) track: &'a LoadedTrack,
    pub(super) track_ref: TrackRef,
    pub(super) track_label: Option<&'a str>,
    pub(super) kind: MetricKind,
}

pub(super) struct MetricHoverCandidate {
    pub(super) distance_px: f32,
    point: PlotPoint,
    source: MetricSampleSource,
}

#[derive(Clone, Copy)]
pub(super) struct MetricHoverSelection {
    pub(super) fix: FixRef,
    pub(super) time: DateTime<Utc>,
}

pub(super) struct MetricHover {
    fix: FixRef,
    track: Option<String>,
    kind: MetricKind,
    point: PlotPoint,
    time: Option<DateTime<Utc>>,
}

impl MetricHover {
    pub(super) fn from_sample(
        context: &MetricHoverContext<'_>,
        sample: &MetricHoverCandidate,
    ) -> Self {
        Self {
            fix: FixRef::new(context.track_ref, PointIdx::new(sample.source.point_index)),
            time: sample
                .source
                .normalized_time(context.track)
                .map(|time| time.utc()),
            track: context.track_label.map(ToOwned::to_owned),
            kind: context.kind,
            point: sample.point,
        }
    }

    pub(super) fn selection(&self) -> Option<MetricHoverSelection> {
        Some(MetricHoverSelection {
            fix: self.fix,
            time: self.time?,
        })
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(self.kind.label());
        if let Some(track) = &self.track {
            ui.label(track);
        }
        if let Some(time) = self.time {
            ui.label(time.format("%H:%M:%S%.f").to_string());
        }
        let value = match self.kind {
            MetricKind::SatsFix
            | MetricKind::SatsSeen
            | MetricKind::GpsFix
            | MetricKind::GpsSeen
            | MetricKind::GlonassFix
            | MetricKind::GlonassSeen
            | MetricKind::GalileoFix
            | MetricKind::GalileoSeen
            | MetricKind::BeidouFix
            | MetricKind::BeidouSeen
            | MetricKind::NavicFix
            | MetricKind::NavicSeen
            | MetricKind::QzssFix
            | MetricKind::QzssSeen => format!("{:.0}", self.point.y),
            _ => format!("{:.2}", self.point.y),
        };
        ui.label(value);
    }
}

impl MetricSeries {
    pub(super) fn nearest_sample(
        &self,
        plot_ui: &egui_plot::PlotUi<'_>,
        run_index: usize,
        points: &[PlotPoint],
        pointer: egui::Pos2,
    ) -> Option<MetricHoverCandidate> {
        let line = Line::new("", PlotPoints::Borrowed(points));
        let closest = line
            .find_closest(pointer, plot_ui.transform())
            .filter(|closest| closest.dist_sq <= HOVER_RADIUS_PX.powi(2))?;
        let point = *points.get(closest.index)?;
        Some(MetricHoverCandidate {
            distance_px: closest.dist_sq.sqrt(),
            point,
            source: self.sample_source(run_index, point.x)?,
        })
    }
}
