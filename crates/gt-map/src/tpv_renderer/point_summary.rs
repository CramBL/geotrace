use gt_types::{NavPoint, SatelliteQueryScope, SkyCounts};
use gt_ui_theme::{NavigationStatus, SatelliteCountBadge, SatelliteCountResponses};

pub(super) struct PointSummary<'a>(pub &'a NavPoint);

impl PointSummary<'_> {
    pub(super) fn summary_ui(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.status_ui(ui);
            self.solution_fields_ui(ui);
        });
    }

    pub(super) fn status_ui(&self, ui: &mut egui::Ui) {
        NavigationStatus {
            state: self.0.resolved_navigation_state().state,
            provenance: self.0.navigation.position_provenance,
        }
        .ui(ui);
    }

    pub(super) fn solution_fields_ui(&self, ui: &mut egui::Ui) {
        if let Some(method) = self.0.navigation.method {
            ui.label(method.to_string());
        }
        if let Some(dimension) = self.0.navigation.dimension {
            ui.label(dimension.to_string());
        }
    }

    pub(super) fn dop_row_ui(&self, ui: &mut egui::Ui) {
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

    pub(super) fn counts_ui(
        &self,
        ui: &mut egui::Ui,
        scope: SatelliteQueryScope,
    ) -> SatelliteCountResponses {
        SatelliteCountBadge(SkyCounts {
            used: self.0.resolved_used_count(scope).map(|value| value.count),
            seen: self.0.resolved_seen_count(scope).map(|value| value.count),
        })
        .ui(ui)
    }
}
