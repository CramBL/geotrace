use std::ptr;

use egui::RichText;
use gt_sky::{SkyHighlight, SkyPlot, SkyPlotSize};
use gt_types::{
    LoadedTrack, NavPoint, NearbySkyEvidence, PointIdx, SKY_REPORT_MAX_AGE_SECS, SkyGeometry,
};

/// The sky column of the hover badge.
pub(crate) enum SkySection<'a> {
    /// The track has satellite reports, but none within the age window of
    /// this point.
    NoReportNearby,
    /// The point's own satellite report, or one borrowed from a nearby point.
    Report(NearbySkyEvidence<'a>),
    /// A track without reports uses the space for ordinary metrics.
    TrackWithoutReports,
}

impl<'a> SkySection<'a> {
    pub(crate) fn resolve(track: &'a LoadedTrack, point_index: PointIdx) -> Self {
        if track.metadata.satellite_report_count == 0 {
            return Self::TrackWithoutReports;
        }
        match track
            .nearest_sky_evidence(point_index, SkyGeometry::Position)
            .or_else(|| track.nearest_sky_evidence(point_index, SkyGeometry::Identities))
        {
            Some(report) => Self::Report(report),
            None => Self::NoReportNearby,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct SkyHost<'a> {
    pub(super) point: &'a NavPoint,
    pub(super) identities_in_table: bool,
}

/// The sky plot with its report-age line, or the no-report placeholder. The
/// full size is interactive (per-mark tooltips). The compact size lives
/// inside the hover badge, which is itself a tooltip.
pub(super) fn sky_section_ui(
    ui: &mut egui::Ui,
    sky: &SkySection<'_>,
    size: SkyPlotSize,
    highlight: Option<SkyHighlight>,
    host: SkyHost<'_>,
) {
    match sky {
        SkySection::TrackWithoutReports => {}
        SkySection::NoReportNearby => {
            ui.label(
                RichText::new(format!(
                    "No satellite report within {SKY_REPORT_MAX_AGE_SECS}s"
                ))
                .weak()
                .small(),
            );
        }
        SkySection::Report(report) => {
            let plot = SkyPlot::from_evidence(report.evidence, size).with_highlight(highlight);
            let same_point = ptr::eq(report.evidence.point, host.point);
            let plot = if same_point {
                plot.counts_in_host()
            } else {
                plot
            };
            let same_identity_table = same_point
                && host.identities_in_table
                && host
                    .point
                    .sky_evidence(SkyGeometry::Identities)
                    .is_some_and(|evidence| {
                        evidence.observation.map(|(index, _)| index)
                            == report.evidence.observation.map(|(index, _)| index)
                    });
            let plot = if same_identity_table {
                plot.identities_in_host()
            } else {
                plot
            };
            let plot = match size {
                SkyPlotSize::Full => plot.interactive(),
                SkyPlotSize::Compact => plot,
            };
            plot.ui(ui);
            if !same_point {
                ui.label(RichText::new(report_age_label(report.age)).weak().small())
                    .on_hover_text(report_age_description(report.age));
            }
        }
    }
}

pub(super) fn report_age_label(age: chrono::Duration) -> String {
    format!("Sky {:+.1}s", -age.as_seconds_f64())
}

pub(super) fn report_age_description(age: chrono::Duration) -> String {
    if age.is_zero() {
        return "Report at the same time".to_owned();
    }
    let seconds = age.abs().as_seconds_f64();
    let side = if age > chrono::Duration::zero() {
        "earlier"
    } else {
        "later"
    };
    format!("Report {seconds:.1}s {side}")
}
