use std::ops::RangeInclusive;

use egui::{DragValue, Grid};
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE as ICON_CLOCK_COUNTER_CLOCKWISE;
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use egui_phosphor::regular::WAVE_SINE as ICON_WAVE_SINE;
use gt_session_bridge::{
    AnalysisSettingKey, SettingApply, SettingInfo, SettingKey, SettingUnit, SettingValue,
};

use crate::app::App;
use crate::app::settings_ui::SettingsPage;
use crate::settings::AnalysisSettings;

impl App {
    pub(super) fn show_analysis_page(&mut self, ui: &mut egui::Ui) {
        SettingsPage::Analysis.show_header(ui);
        let mut values = AnalysisSettings::from(&self.shared.borrow().plot_state);
        let mut changed = false;
        Grid::new("analysis_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                for control in ANALYSIS_CONTROLS {
                    let label = ui
                        .label(format!("{} {}", control.icon, control.label))
                        .on_hover_text(control.help);
                    let (input, edited) = control.input.show(ui, &mut values);
                    let input = input.on_hover_text(control.help);
                    self.reveal_setting_row(
                        ui,
                        SettingKey::Analysis(control.key),
                        label.rect.union(input.rect),
                    );
                    changed |= edited;
                    ui.end_row();
                }
            });
        if changed {
            self.apply_analysis_controls(values);
        }
    }

    fn apply_analysis_controls(&mut self, values: AnalysisSettings) {
        let analysis = values.plot_config();
        self.loader.analysis_config = analysis;
        let markers = &mut self.processing_config.generated_markers;
        markers.slip_elevation_mask_deg = analysis.elevation_mask_deg;
        markers.slip_snr_drop_db = analysis.snr_drop_db;
        markers.clock_excursion_threshold_s = analysis.clock_excursion_threshold_s;
        let mut shared = self.shared.borrow_mut();
        let state = &mut *shared;
        state.plot_state.set_analysis(&state.loaded_files, analysis);
        state.plot_state.mark_masked_fix = values.mark_masked_fix;
        state.plot_state.mark_backward_time_steps = values.mark_backward_time_steps;
    }
}

pub(super) struct AnalysisControl {
    key: AnalysisSettingKey,
    icon: &'static str,
    pub(super) label: &'static str,
    help: &'static str,
    apply: SettingApply,
    input: AnalysisInput,
}

impl AnalysisControl {
    pub(super) const fn for_key(key: AnalysisSettingKey) -> Self {
        match key {
            AnalysisSettingKey::ClockExcursionThreshold => Self {
                key,
                icon: ICON_WARNING,
                label: CLOCK_OFFSET_EXCURSION_LABEL,
                help: "Clock offset excursions deviate from the track's baseline GPS/system clock \
                       offset by more than this threshold. The plot marks excursions at the \
                       edge of the view and excludes them from the clock offset line. The line \
                       includes sustained shifts in the baseline.",
                apply: SettingApply::LiveWithDeferredMarkers,
                input: AnalysisInput::Number {
                    field: |values| &mut values.clock_excursion_threshold_s,
                    range: gt_plot::CLOCK_EXCURSION_THRESHOLD_RANGE_S,
                    unit: SettingUnit::Seconds,
                    speed: 1.0,
                },
            },
            AnalysisSettingKey::ElevationMask => Self {
                key,
                icon: ICON_FUNNEL,
                label: "Elevation mask",
                help: "The utilization and slip rates use satellites at or above this elevation. \
                       A higher mask excludes more satellites near the horizon, where atmospheric \
                       delay and multipath are more common.",
                apply: SettingApply::LiveWithDeferredMarkers,
                input: AnalysisInput::Number {
                    field: |values| &mut values.elevation_mask_deg,
                    range: 0.0..=90.0,
                    unit: SettingUnit::Degrees,
                    speed: 1.0,
                },
            },
            AnalysisSettingKey::MarkBackwardTimeSteps => Self {
                key,
                icon: ICON_CLOCK_COUNTER_CLOCKWISE,
                label: "Mark backward time steps",
                help: "Mark backward steps in channel sample time at the bottom of the plot. \
                       Hover a mark for the affected channels and both timestamps. \
                       Marks appear with the channel lines. The plot preserves the file's \
                       sample order.",
                apply: SettingApply::Live,
                input: AnalysisInput::Toggle(|values| &mut values.mark_backward_time_steps),
            },
            AnalysisSettingKey::MarkMaskedFix => Self {
                key,
                icon: ICON_WARNING,
                label: "Mark masked-out used satellites",
                help: "Show a red cross on the Util all plot when a fix uses a satellite below \
                       the elevation mask. Hover the cross for the satellites and their elevations.",
                apply: SettingApply::Live,
                input: AnalysisInput::Toggle(|values| &mut values.mark_masked_fix),
            },
            AnalysisSettingKey::SlipWindow => Self {
                key,
                icon: ICON_CLOCK,
                label: "Slip window",
                help: "The plot divides the number of slips in this trailing window by its \
                       duration, in slips per minute.",
                apply: SettingApply::Live,
                input: AnalysisInput::Number {
                    field: |values| &mut values.slip_window_min,
                    range: 1.0..=120.0,
                    unit: SettingUnit::Minutes,
                    speed: 1.0,
                },
            },
            AnalysisSettingKey::SnrDropThreshold => Self {
                key,
                icon: ICON_WAVE_SINE,
                label: "SNR drop threshold",
                help: "A drop greater than this threshold between consecutive epochs counts as \
                       a slip when the receiver still tracks the satellite at or above the \
                       elevation mask.",
                apply: SettingApply::LiveWithDeferredMarkers,
                input: AnalysisInput::Number {
                    field: |values| &mut values.snr_drop_db,
                    range: 1.0..=60.0,
                    unit: SettingUnit::DecibelHertz,
                    speed: 0.5,
                },
            },
        }
    }

    pub(super) fn describe(&self, mut values: AnalysisSettings) -> SettingInfo {
        let value = match &self.input {
            AnalysisInput::Number {
                field, range, unit, ..
            } => {
                let number = *field(&mut values);
                SettingValue::Number {
                    value: number.is_finite().then_some(f64::from(number)),
                    minimum: f64::from(*range.start()),
                    maximum: f64::from(*range.end()),
                    unit: *unit,
                }
            }
            AnalysisInput::Toggle(field) => SettingValue::Boolean {
                value: *field(&mut values),
            },
        };
        SettingInfo {
            key: SettingKey::Analysis(self.key),
            page: SettingsPage::Analysis.rail_label().to_owned(),
            label: self.label.to_owned(),
            help: self.help.to_owned(),
            apply: self.apply,
            value,
        }
    }
}

enum AnalysisInput {
    Number {
        field: fn(&mut AnalysisSettings) -> &mut f32,
        range: RangeInclusive<f32>,
        unit: SettingUnit,
        speed: f64,
    },
    Toggle(fn(&mut AnalysisSettings) -> &mut bool),
}

impl AnalysisInput {
    fn show(&self, ui: &mut egui::Ui, values: &mut AnalysisSettings) -> (egui::Response, bool) {
        match self {
            Self::Number {
                field,
                range,
                unit,
                speed,
            } => {
                let value = field(values);
                let initial = value.to_bits();
                let suffix = match unit {
                    SettingUnit::DecibelHertz => " dB-Hz",
                    SettingUnit::Degrees => "°",
                    SettingUnit::Minutes => "min",
                    SettingUnit::Seconds => "s",
                    SettingUnit::RobustStandardDeviations => "σ",
                };
                let response = ui.add(
                    DragValue::from_get_set(|new| {
                        if let Some(new) = new.filter(|new| new.is_finite()) {
                            *value = (new as f32).clamp(*range.start(), *range.end());
                        }
                        f64::from(*value)
                    })
                    .range(range.clone())
                    .clamp_existing_to_range(false)
                    .speed(*speed)
                    .fixed_decimals(0)
                    .suffix(suffix),
                );
                (response, value.to_bits() != initial)
            }
            Self::Toggle(field) => {
                let response = ui.checkbox(field(values), "");
                let changed = response.changed();
                (response, changed)
            }
        }
    }
}

pub(super) const CLOCK_OFFSET_EXCURSION_LABEL: &str = "Clock offset excursion";

pub(super) const ANALYSIS_CONTROLS: &[AnalysisControl] = &[
    AnalysisControl::for_key(AnalysisSettingKey::ElevationMask),
    AnalysisControl::for_key(AnalysisSettingKey::SnrDropThreshold),
    AnalysisControl::for_key(AnalysisSettingKey::SlipWindow),
    AnalysisControl::for_key(AnalysisSettingKey::ClockExcursionThreshold),
    AnalysisControl::for_key(AnalysisSettingKey::MarkMaskedFix),
    AnalysisControl::for_key(AnalysisSettingKey::MarkBackwardTimeSteps),
];
