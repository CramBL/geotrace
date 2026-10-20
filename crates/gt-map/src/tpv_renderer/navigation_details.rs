use egui::{CollapsingHeader, Grid, Label, Ui};
use gt_fmt::UTC_SUBSECOND_FORMAT;
use gt_types::satellites::Constellation;
use gt_types::{
    NavPoint, NavigationEvidenceSource, SatelliteQueryScope, SatelliteReportMetadata,
    SatelliteScope, SkyCounts,
};
use gt_ui_theme::{NavigationStatus, SatelliteCountBadge, SatelliteCountResponses};
use strum::IntoEnumIterator as _;

pub(super) struct NavigationDetails<'a>(pub &'a NavPoint);

impl NavigationDetails<'_> {
    pub(super) fn summary_ui(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            self.status_ui(ui);
            self.solution_fields_ui(ui);
        });
    }

    pub(super) fn status_ui(&self, ui: &mut Ui) {
        NavigationStatus {
            state: self.0.resolved_navigation_state().state,
            provenance: self.0.navigation.position_provenance,
        }
        .ui(ui);
    }

    pub(super) fn solution_fields_ui(&self, ui: &mut Ui) {
        if let Some(method) = self.0.navigation.method {
            ui.label(method.to_string());
        }
        if let Some(dimension) = self.0.navigation.dimension {
            ui.label(dimension.to_string());
        }
    }

    pub(super) fn dop_row_ui(&self, ui: &mut Ui) {
        if [
            self.0.navigation.hdop,
            self.0.navigation.vdop,
            self.0.navigation.pdop,
        ]
        .iter()
        .all(Option::is_none)
        {
            return;
        }
        ui.label("DOP");
        ui.horizontal(|ui| {
            for (label, value) in [
                ("H", self.0.navigation.hdop),
                ("V", self.0.navigation.vdop),
                ("P", self.0.navigation.pdop),
            ] {
                if let Some(value) = value {
                    ui.label(format!("{label} {value:.2}"))
                        .on_hover_text("Reported dimensionless dilution of precision");
                }
            }
        });
        ui.end_row();
    }

    pub(super) fn inspector_ui(&self, ui: &mut Ui) {
        Grid::new("navigation_evidence").show(ui, |ui| {
            ui.label("State");
            ui.label(self.0.resolved_navigation_state().state.to_string());
            ui.end_row();
            ui.label("State evidence");
            ui.label(Self::evidence_label(
                self.0.resolved_navigation_state().evidence,
            ));
            ui.end_row();
            ui.label("Position source");
            ui.label(
                self.0
                    .navigation
                    .position_provenance
                    .map_or_else(|| "Unreported".to_owned(), |value| value.to_string()),
            );
            ui.end_row();
            for (label, count) in [
                (
                    "Used evidence",
                    self.0.resolved_used_count(SatelliteQueryScope::Global),
                ),
                (
                    "Seen evidence",
                    self.0.resolved_seen_count(SatelliteQueryScope::Global),
                ),
            ] {
                ui.label(label);
                ui.label(Self::evidence_label(count.map(|value| value.evidence)));
                ui.end_row();
            }
            ui.label("Satellites");
            self.counts_ui(ui, SatelliteQueryScope::Global);
            ui.end_row();
            for constellation in Constellation::iter() {
                let scope = SatelliteQueryScope::Constellation(constellation);
                for (quantity, value) in [
                    ("used", self.0.resolved_used_count(scope)),
                    ("seen", self.0.resolved_seen_count(scope)),
                ] {
                    if let Some(value) = value {
                        ui.label(format!("{} {quantity}", constellation.display_name()));
                        ui.label(format!(
                            "{} · {}",
                            value.count,
                            Self::evidence_label(Some(value.evidence))
                        ));
                        ui.end_row();
                    }
                }
            }
            for (label, value) in [
                ("HDOP", self.0.navigation.hdop),
                ("VDOP", self.0.navigation.vdop),
                ("PDOP", self.0.navigation.pdop),
            ] {
                if let Some(value) = value {
                    ui.label(label);
                    ui.label(value.to_string());
                    ui.end_row();
                }
            }
            ui.label("Reported used");
            ui.label(
                self.0
                    .navigation
                    .used_count
                    .map_or_else(|| gt_fmt::EM_DASH.to_owned(), |value| value.to_string()),
            );
            ui.end_row();
            if let Some(time) = self.0.tpv.gps_time() {
                ui.label("Receiver");
                ui.add(
                    Label::new(format!("{} UTC", time.utc().format(UTC_SUBSECOND_FORMAT)))
                        .selectable(true),
                );
                ui.end_row();
            }
            if let Some(time) = self.0.tpv.sys_time() {
                ui.label("Host");
                ui.add(
                    Label::new(format!("{} UTC", time.utc().format(UTC_SUBSECOND_FORMAT)))
                        .selectable(true),
                );
                ui.end_row();
            }
        });
        self.reports_ui(ui);
    }

    fn evidence_label(evidence: Option<NavigationEvidenceSource>) -> String {
        match evidence {
            None => "Unreported".to_owned(),
            Some(NavigationEvidenceSource::ExplicitState) => "Reported state".to_owned(),
            Some(NavigationEvidenceSource::NavigationCount) => "Navigation used count".to_owned(),
            Some(NavigationEvidenceSource::UsageAggregate(index)) => {
                format!("Usage {} aggregate", index + 1)
            }
            Some(NavigationEvidenceSource::UsageList(index)) => {
                format!("Usage {} membership", index + 1)
            }
            Some(NavigationEvidenceSource::ObservationAggregate(index)) => {
                format!("Observation {} aggregate", index + 1)
            }
            Some(NavigationEvidenceSource::ObservationList(index)) => {
                format!("Observation {} identities", index + 1)
            }
        }
    }

    pub(super) fn counts_ui(
        &self,
        ui: &mut Ui,
        scope: SatelliteQueryScope,
    ) -> SatelliteCountResponses {
        SatelliteCountBadge(SkyCounts {
            used: self.0.resolved_used_count(scope).map(|value| value.count),
            seen: self.0.resolved_seen_count(scope).map(|value| value.count),
        })
        .ui(ui)
    }

    pub(super) fn reports_ui(&self, ui: &mut Ui) {
        for (index, report) in self.0.satellite_evidence.usage.iter().enumerate() {
            ui.push_id(("usage", index), |ui| {
                let details = ReportDetails(report.metadata);
                CollapsingHeader::new(format!(
                    "Usage {} · {} · {}",
                    index + 1,
                    report.metadata.normalized_time.utc().format("%H:%M:%S%.3f"),
                    details.scope_label()
                ))
                .default_open(true)
                .show(ui, |ui| {
                    details.ui(ui);
                    Grid::new("usage_rows").striped(true).show(ui, |ui| {
                        ui.weak("PRN");
                        ui.weak("Usage");
                        ui.end_row();
                        for satellite in &report.satellites {
                            ui.horizontal(|ui| {
                                super::constellation_swatch(ui, satellite.identity.constellation);
                                ui.add(
                                    Label::new(format!(
                                        "{}{:02}",
                                        satellite.identity.constellation.prn_prefix(),
                                        satellite.identity.prn
                                    ))
                                    .selectable(true),
                                );
                            });
                            ui.label(satellite.membership.to_string());
                            ui.end_row();
                        }
                    });
                });
            });
        }
        for (index, report) in self.0.satellite_evidence.observations.iter().enumerate() {
            ui.push_id(("observation", index), |ui| {
                let details = ReportDetails(report.metadata);
                CollapsingHeader::new(format!(
                    "Observation {} · {} · {}",
                    index + 1,
                    report.metadata.normalized_time.utc().format("%H:%M:%S%.3f"),
                    details.scope_label()
                ))
                .default_open(true)
                .show(ui, |ui| {
                    details.ui(ui);
                    Grid::new("observation_rows").striped(true).show(ui, |ui| {
                        for label in ["PRN", "Az", "El", "SNR"] {
                            ui.weak(label);
                        }
                        ui.end_row();
                        for satellite in &report.satellites {
                            ui.horizontal(|ui| {
                                super::constellation_swatch(ui, satellite.identity.constellation);
                                ui.add(
                                    Label::new(format!(
                                        "{}{:02}",
                                        satellite.identity.constellation.prn_prefix(),
                                        satellite.identity.prn
                                    ))
                                    .selectable(true),
                                );
                            });
                            ui.label(satellite.measurements.azimuth.map_or_else(
                                || gt_fmt::EM_DASH.to_owned(),
                                |value| format!("{value}°"),
                            ));
                            ui.label(satellite.measurements.elevation.map_or_else(
                                || gt_fmt::EM_DASH.to_owned(),
                                |value| format!("{value}°"),
                            ));
                            ui.label(satellite.measurements.snr.map_or_else(
                                || gt_fmt::EM_DASH.to_owned(),
                                |value| value.value().to_string(),
                            ));
                            ui.end_row();
                        }
                    });
                });
            });
        }
    }
}

struct ReportDetails(SatelliteReportMetadata);

impl ReportDetails {
    fn scope_label(&self) -> String {
        match self.0.scope {
            SatelliteScope::Global => "Global".to_owned(),
            SatelliteScope::Constellations(set) => Constellation::iter()
                .filter(|constellation| set.contains(*constellation))
                .map(Constellation::display_name)
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    pub(super) fn ui(&self, ui: &mut Ui) {
        Grid::new("report_metadata").show(ui, |ui| {
            ui.label("Coverage");
            ui.label(self.0.coverage.to_string());
            ui.end_row();
            ui.label("Reported count");
            ui.label(
                self.0
                    .reported_count
                    .map_or_else(|| gt_fmt::EM_DASH.to_owned(), |count| count.to_string()),
            );
            ui.end_row();
            for (label, time) in [
                ("Track", Some(self.0.normalized_time.utc())),
                ("Receiver", self.0.gps_time.map(|time| time.utc())),
                ("Host", self.0.sys_time.map(|time| time.utc())),
            ] {
                if let Some(time) = time {
                    ui.label(label);
                    ui.add(
                        Label::new(format!("{} UTC", time.format(UTC_SUBSECOND_FORMAT)))
                            .selectable(true),
                    );
                    ui.end_row();
                }
            }
            ui.label("Input order");
            ui.label(self.0.input_order.to_string());
            ui.end_row();
        });
    }
}
