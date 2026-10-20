use egui::{Grid, RichText, Ui};
use gt_fmt::UTC_SUBSECOND_FORMAT;
use gt_types::satellites::Constellation;
use gt_types::{
    NavPoint, NavigationEvidenceSource, SatelliteQueryScope, SatelliteReportMetadata,
    SatelliteScope, SkyCounts,
};
use strum::IntoEnumIterator as _;

pub(super) struct NavigationDetails<'a>(pub &'a NavPoint);

impl NavigationDetails<'_> {
    pub(super) fn rows_ui(&self, ui: &mut Ui) {
        let state = self.0.resolved_navigation_state();
        ui.label("State");
        ui.label(state.state.to_string());
        ui.end_row();
        ui.label("State evidence");
        ui.label(Self::evidence_label(state.evidence))
            .on_hover_ui(|ui| {
                if let Some(metadata) = state
                    .evidence
                    .and_then(|source| self.report_metadata(source))
                {
                    ReportDetails(metadata).ui(ui);
                }
            });
        ui.end_row();
        if let Some(method) = self.0.navigation.method {
            ui.label("Reported method");
            ui.label(method.to_string());
            ui.end_row();
        }
        if let Some(dimension) = self.0.navigation.dimension {
            ui.label("Reported dimension");
            ui.label(dimension.to_string());
            ui.end_row();
        }
        if let Some(provenance) = self.0.navigation.position_provenance {
            ui.label("Position source");
            ui.label(provenance.to_string()).on_hover_text(
                "Source provided means the producer supplied the coordinates. SDK synthesized means the SDK generated the coordinates.",
            );
            ui.end_row();
        }
        for (label, value) in [
            ("HDOP", self.0.navigation.hdop),
            ("VDOP", self.0.navigation.vdop),
            ("PDOP", self.0.navigation.pdop),
        ] {
            if let Some(value) = value {
                ui.label(label);
                ui.label(value.to_string())
                    .on_hover_text("Reported dimensionless dilution of precision");
                ui.end_row();
            }
        }
    }

    fn evidence_label(evidence: Option<NavigationEvidenceSource>) -> String {
        match evidence {
            None => "No state observation".to_owned(),
            Some(NavigationEvidenceSource::ExplicitState) => "Reported state".to_owned(),
            Some(NavigationEvidenceSource::NavigationCount) => "Navigation used count".to_owned(),
            Some(NavigationEvidenceSource::UsageAggregate(index)) => {
                format!("Usage report {} aggregate", index + 1)
            }
            Some(NavigationEvidenceSource::UsageList(index)) => {
                format!("Usage report {} membership", index + 1)
            }
            Some(NavigationEvidenceSource::ObservationAggregate(index)) => {
                format!("Observation report {} aggregate", index + 1)
            }
            Some(NavigationEvidenceSource::ObservationList(index)) => {
                format!("Observation report {} identities", index + 1)
            }
        }
    }

    fn report_metadata(&self, source: NavigationEvidenceSource) -> Option<SatelliteReportMetadata> {
        match source {
            NavigationEvidenceSource::UsageAggregate(index)
            | NavigationEvidenceSource::UsageList(index) => self
                .0
                .satellite_evidence
                .usage
                .get(index)
                .map(|report| report.metadata),
            NavigationEvidenceSource::ObservationAggregate(index)
            | NavigationEvidenceSource::ObservationList(index) => self
                .0
                .satellite_evidence
                .observations
                .get(index)
                .map(|report| report.metadata),
            NavigationEvidenceSource::ExplicitState | NavigationEvidenceSource::NavigationCount => {
                None
            }
        }
    }

    pub(super) fn count_row_ui(
        &self,
        ui: &mut Ui,
        scope: SatelliteQueryScope,
        label: &str,
    ) -> egui::Response {
        let used = self.0.resolved_used_count(scope);
        let seen = self.0.resolved_seen_count(scope);
        let counts = SkyCounts {
            used: used.map(|value| value.count),
            seen: seen.map(|value| value.count),
        };
        ui.label(label);
        let dark_mode = ui.visuals().dark_mode;
        let color = counts.used.map_or_else(
            || {
                counts.seen.map_or_else(
                    || ui.visuals().weak_text_color(),
                    |count| super::seen_count_color(count, dark_mode),
                )
            },
            |count| super::fix_count_color(count, dark_mode),
        );
        let response = ui
            .colored_label(color, gt_fmt::SatelliteCounts(counts).to_string())
            .on_hover_ui(|ui| {
                for (quantity, count) in [("Used", used), ("Seen", seen)] {
                    ui.label(format!(
                        "{quantity} evidence {}",
                        count.map_or_else(
                            || "Unreported".to_owned(),
                            |value| Self::evidence_label(Some(value.evidence))
                        )
                    ));
                    if let Some(metadata) =
                        count.and_then(|value| self.report_metadata(value.evidence))
                    {
                        ReportDetails(metadata).ui(ui);
                    }
                }
            });
        ui.end_row();
        response
    }

    pub(super) fn reports_ui(&self, ui: &mut Ui) {
        if self.0.satellite_evidence.usage.is_empty()
            && self.0.satellite_evidence.observations.is_empty()
        {
            return;
        }
        ui.collapsing("Recorded satellite reports", |ui| {
            for (index, report) in self.0.satellite_evidence.usage.iter().enumerate() {
                ui.push_id(("usage", index), |ui| {
                    ui.label(RichText::new(format!("Usage report {}", index + 1)).strong());
                    ReportDetails(report.metadata).ui(ui);
                    Grid::new("usage_rows").show(ui, |ui| {
                        for satellite in &report.satellites {
                            ui.label(format!(
                                "{} {}",
                                satellite.identity.constellation.display_name(),
                                satellite.identity.prn
                            ));
                            ui.label(satellite.membership.to_string());
                            ui.end_row();
                        }
                    });
                });
            }
            for (index, report) in self.0.satellite_evidence.observations.iter().enumerate() {
                ui.push_id(("observation", index), |ui| {
                    ui.label(RichText::new(format!("Observation report {}", index + 1)).strong());
                    ReportDetails(report.metadata).ui(ui);
                    Grid::new("observation_rows").show(ui, |ui| {
                        for satellite in &report.satellites {
                            ui.label(format!(
                                "{} {}",
                                satellite.identity.constellation.display_name(),
                                satellite.identity.prn
                            ));
                            ui.label(format!(
                                "Az {} · El {} · SNR {}",
                                satellite.measurements.azimuth.map_or_else(
                                    || gt_fmt::EM_DASH.to_owned(),
                                    |value| format!("{value}°")
                                ),
                                satellite.measurements.elevation.map_or_else(
                                    || gt_fmt::EM_DASH.to_owned(),
                                    |value| format!("{value}°")
                                ),
                                satellite.measurements.snr.map_or_else(
                                    || gt_fmt::EM_DASH.to_owned(),
                                    |value| format!("{} dB-Hz", value.value())
                                )
                            ));
                            ui.end_row();
                        }
                    });
                });
            }
        });
    }
}

struct ReportDetails(SatelliteReportMetadata);

impl ReportDetails {
    fn ui(&self, ui: &mut Ui) {
        let scope = match self.0.scope {
            SatelliteScope::Global => "Global".to_owned(),
            SatelliteScope::Constellations(set) => Constellation::iter()
                .filter(|constellation| set.contains(*constellation))
                .map(Constellation::display_name)
                .collect::<Vec<_>>()
                .join(", "),
        };
        ui.label(format!("{scope} · {}", self.0.coverage));
        ui.label(format!(
            "Reported count {}",
            self.0
                .reported_count
                .map_or_else(|| gt_fmt::EM_DASH.to_owned(), |count| count.to_string())
        ));
        ui.label(format!(
            "Track time {} UTC",
            self.0.normalized_time.utc().format(UTC_SUBSECOND_FORMAT)
        ));
        if let Some(time) = self.0.gps_time {
            ui.label(format!(
                "Receiver time {} UTC",
                time.utc().format(UTC_SUBSECOND_FORMAT)
            ));
        }
        if let Some(time) = self.0.sys_time {
            ui.label(format!(
                "Host time {} UTC",
                time.utc().format(UTC_SUBSECOND_FORMAT)
            ));
        }
    }
}
