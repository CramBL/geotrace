use chrono::{DateTime, TimeDelta, Utc};
use egui_plot::{Line, PlotItem as _, PlotPoint, PlotPoints};
use gt_types::{
    BootClockBackwardStep, FixRef, LoadedTrack, MetricKind, PointIdx, SystemBootComparison,
    TrackRef,
};

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
    system_boot: Option<SystemBootMetricHover>,
}

struct SystemBootMetricHover {
    comparison: SystemBootComparison,
    backward_step: Option<BootClockBackwardStep>,
    repair_threshold: Option<TimeDelta>,
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
            system_boot: (context.kind == MetricKind::SystemBootDeltaMs)
                .then(|| {
                    let tpv = context.track.points.get(sample.source.point_index)?.tpv;
                    Some(SystemBootMetricHover {
                        comparison: tpv.system_boot_comparison()?,
                        backward_step: tpv.boot_clock_backward_step(),
                        repair_threshold: tpv.debug_time_repair_threshold(),
                    })
                })
                .flatten(),
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
        if let Some(system_boot) = &self.system_boot {
            system_boot.show_ui(ui);
            return;
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

impl SystemBootMetricHover {
    fn show_ui(&self, ui: &mut egui::Ui) {
        ui.label(format!(
            "{:+.3}ms ({:+}us)",
            self.comparison.delta.as_millis_f64(),
            self.comparison.delta.as_micros(),
        ));
        let reference = self.comparison.reference;
        ui.label(format!(
            "Reference system {}",
            reference
                .system_time
                .utc()
                .format("%Y-%m-%d %H:%M:%S%.6f UTC"),
        ));
        ui.label(format!(
            "Reference boot {}us",
            reference.boot_time.as_micros()
        ));
        ui.label("Positive means system time advanced farther than boot time");
        if let Some(step) = self.backward_step {
            ui.label(format!("Boot-clock backward step {step}"));
        }
        if let Some(threshold) = self.repair_threshold {
            ui.label(format!("Computed from system timestamps loaded with debug time repair (threshold {}s). Differences between shifted spans include synthetic shifts.", threshold.as_seconds_f64()));
        }
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
