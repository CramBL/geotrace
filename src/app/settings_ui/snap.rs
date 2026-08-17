//! Settings for the map-matching server tracks are snapped against.

use egui::DragValue;
use gt_snap::wire::Costing;
use strum::IntoEnumIterator;

use crate::app::App;

impl App {
    pub(super) fn show_snap_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui_phosphor::regular::PATH);
            ui.strong("Snap to road");
        });
        ui.separator();
        egui::Grid::new("snap_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let url_help = "Base URL of the Valhalla map-matching server tracks are \
                                snapped against. The default is the free public instance \
                                hosted by FOSSGIS e.V.; point it at your own server to \
                                avoid its fair-use rate limit. Recorded positions are \
                                only uploaded after a one-time acknowledgment per server.";
                ui.label(format!(
                    "{} Server URL",
                    egui_phosphor::regular::GLOBE_SIMPLE
                ))
                .on_hover_text(url_help);
                let mut server_url = self.snap_settings.server_url.clone();
                if ui
                    .text_edit_singleline(&mut server_url)
                    .on_hover_text(url_help)
                    .changed()
                {
                    self.snap.set_server_url(&server_url);
                    self.snap_settings.server_url = server_url;
                }
                ui.end_row();

                let costing_help = "Road network tracks are matched against when the \
                                    recording does not declare a travel mode - auto is \
                                    Valhalla's name for the motor-vehicle network. A \
                                    declared travel mode always wins over this setting.";
                ui.label(format!("{} Costing", egui_phosphor::regular::CAR))
                    .on_hover_text(costing_help);
                egui::ComboBox::from_id_salt("snap_costing")
                    .selected_text(self.snap_settings.costing.display_name())
                    .show_ui(ui, |ui| {
                        for costing in Costing::iter() {
                            ui.selectable_value(
                                &mut self.snap_settings.costing,
                                costing,
                                costing.display_name(),
                            );
                        }
                    })
                    .response
                    .on_hover_text(costing_help);
                ui.end_row();

                let auto_help = "Automatically snap loaded tracks that are shown on \
                                 the map. Hidden tracks wait until shown, and a \
                                 manual trigger always jumps the queue. Enabling \
                                 prompts for upload consent first when it has not \
                                 been given for the configured server.";
                ui.label(format!("{} Auto snap", egui_phosphor::regular::LIGHTNING))
                    .on_hover_text(auto_help);
                let mut auto = self.snap_settings.auto_snap == Some(true);
                if ui
                    .checkbox(&mut auto, "Snap to road automatically")
                    .on_hover_text(auto_help)
                    .changed()
                {
                    self.snap_settings.auto_snap = Some(auto);
                    self.snap_auto_sweep = auto;
                }
                ui.end_row();

                optional_snap_setting(
                    ui,
                    &mut self.snap_settings.search_radius_m,
                    OptionalSnapSetting {
                        label: format!("{} Search radius", egui_phosphor::regular::CIRCLE_DASHED),
                        help: "Meters around each recorded point searched for \
                               candidate road edges. Unset leaves the server \
                               default; raising it helps very noisy receivers \
                               reach the road at the cost of more match \
                               candidates. Changing it marks existing results \
                               stale - re-run to apply.",
                        range: gt_snap::request_plan::SEARCH_RADIUS_RANGE_M,
                        enable_seed: SEARCH_RADIUS_SEED_M,
                        suffix: " m",
                    },
                );
                ui.end_row();

                optional_snap_setting(
                    ui,
                    &mut self.snap_settings.turn_penalty_factor,
                    OptionalSnapSetting {
                        label: format!("{} Turn penalty", egui_phosphor::regular::ARROW_U_UP_LEFT),
                        help: "Cost multiplier penalizing route reversals. Unset \
                               leaves the server default; raising it (Valhalla \
                               suggests around 500) smooths matches that wander \
                               at intersections when debugging noisy receivers. \
                               Changing it marks existing results stale - re-run \
                               to apply.",
                        range: gt_snap::request_plan::TURN_PENALTY_FACTOR_RANGE,
                        enable_seed: TURN_PENALTY_SEED,
                        suffix: "",
                    },
                );
                ui.end_row();

                optional_snap_setting(
                    ui,
                    &mut self.snap_settings.gps_accuracy_override_m,
                    OptionalSnapSetting {
                        label: format!("{} GPS accuracy", egui_phosphor::regular::CROSSHAIR),
                        help: "Expected GNSS accuracy sent to the matcher, \
                               replacing the value derived from the recording's \
                               eph. Unset keeps the derivation - the usual best \
                               choice; set it when the receiver's claimed \
                               accuracy is known to be wrong. Changing it marks \
                               existing results stale - re-run to apply.",
                        range: gt_snap::request_plan::GPS_ACCURACY_OVERRIDE_RANGE_M,
                        enable_seed: GPS_ACCURACY_SEED_M,
                        suffix: " m",
                    },
                );
                ui.end_row();
            });
    }
}

/// Value an advanced snap option is seeded with when enabled. Search radius:
/// the tuned fixture's value, comfortably wider than typical GNSS noise.
const SEARCH_RADIUS_SEED_M: f64 = 25.0;
/// Turn penalty seed: Valhalla's own suggestion for smoothing wandering
/// matches (see the design doc's parameter inventory).
const TURN_PENALTY_SEED: f64 = 500.0;
/// GPS accuracy seed: a plausible mid-grade receiver's eph, inside the
/// derivation clamp and comfortably within the server-accepted range.
const GPS_ACCURACY_SEED_M: f64 = 10.0;

/// One optional advanced snap setting's presentation: label, hover help,
/// server-accepted bounds, the value enabling seeds, and the unit suffix.
struct OptionalSnapSetting {
    label: String,
    help: &'static str,
    range: std::ops::RangeInclusive<f64>,
    enable_seed: f64,
    suffix: &'static str,
}

/// A settings-grid row for an optional advanced snap option: a checkbox
/// arming it plus a drag value bounded to the server-accepted range, grayed
/// while unset (never hidden, per DESIGN.md). Unset means server default
/// (or derived, for the accuracy override).
fn optional_snap_setting(ui: &mut egui::Ui, value: &mut Option<f64>, setting: OptionalSnapSetting) {
    ui.label(&setting.label).on_hover_text(setting.help);
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui
            .checkbox(&mut enabled, "")
            .on_hover_text(setting.help)
            .changed()
        {
            *value = enabled.then_some(setting.enable_seed);
        }
        let mut shown = value.unwrap_or(setting.enable_seed);
        let drag = ui
            .add_enabled(
                value.is_some(),
                DragValue::new(&mut shown)
                    .range(setting.range)
                    .speed(1.0)
                    .fixed_decimals(0)
                    .suffix(setting.suffix),
            )
            .on_hover_text(setting.help)
            .on_disabled_hover_text("Unset - the server default (or derived value) applies");
        if drag.changed() && value.is_some() {
            *value = Some(shown);
        }
    });
}
