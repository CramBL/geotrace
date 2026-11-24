//! Settings applied when a recording is loaded or explicitly re-applied.

use std::ops::RangeInclusive;

use chrono::Duration;
use egui::emath::Numeric as _;
use egui::{DragValue, Grid};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::ARROWS_IN_LINE_HORIZONTAL as ICON_ARROWS_IN_LINE_HORIZONTAL;
use egui_phosphor::regular::CHAT_CIRCLE_TEXT as ICON_CHAT_CIRCLE_TEXT;
use egui_phosphor::regular::CHECK as ICON_CHECK;
use egui_phosphor::regular::CHECK_CIRCLE as ICON_CHECK_CIRCLE;
use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE as ICON_CLOCK_COUNTER_CLOCKWISE;
use egui_phosphor::regular::LINK_BREAK as ICON_LINK_BREAK;
use egui_phosphor::regular::MAP_PIN as ICON_MAP_PIN;
use egui_phosphor::regular::SCISSORS as ICON_SCISSORS;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use egui_phosphor::regular::X_CIRCLE as ICON_X_CIRCLE;
use gt_session_bridge::{
    ErrorCode, ProcessingSettingKey, SettingApply, SettingInfo, SettingInput, SettingKey,
    SettingUnit, SettingValue,
};
use gt_track_builder::{GeneratedMarkerConfig, SegmentationConfig};
use gt_types::{AssociationConfig, AssociationWindow};

use crate::app::App;
use crate::app::loader;
use crate::app::log_viewer::association_dialog::AUTOMATIC_POSITION_SOURCE_HOVER;
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::analysis::CLOCK_OFFSET_EXCURSION_LABEL;
use crate::app::settings_ui::label::SettingsLabel;
use crate::settings::{InitialPositionSourcePolicy, ProcessingSettings};

impl App {
    /// Returns `true` in the frame when the user clicks "Apply to loaded data".
    pub(super) fn show_processing_page(&mut self, ui: &mut egui::Ui) -> bool {
        SettingsPage::Processing.show_header(ui);
        Grid::new("settings_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                TRACK_SPLIT_GAP.show_row(ui, self);

                let debug_time_repair_help =
                    "Open debug… splits a recording when timestamps step backwards by at least \
                     this much, then loads each chronological run as a separate track. The \
                     Open… menu item starts a regular load with no time repair.";
                ui.label(format!(
                    "{ICON_CLOCK_COUNTER_CLOCKWISE} {DEBUG_BACKWARD_JUMP_THRESHOLD_LABEL}"
                ))
                .on_hover_text(debug_time_repair_help);
                let mut debug_time_repair_threshold_seconds =
                    u64::from(self.debug_time_repair_threshold_seconds);
                ui.horizontal(|ui| {
                    self::compound_duration_input(
                        ui,
                        &mut debug_time_repair_threshold_seconds,
                        u64::from(loader::DEBUG_TIME_REPAIR_MIN_THRESHOLD_SECONDS)
                            ..=u64::from(loader::DEBUG_TIME_REPAIR_MAX_THRESHOLD_SECONDS),
                        DurationDisplay::Days,
                    );
                })
                .response
                .on_hover_text(debug_time_repair_help);
                if let Ok(value) = u32::try_from(debug_time_repair_threshold_seconds) {
                    self.debug_time_repair_threshold_seconds = value;
                }
                ui.end_row();

                ui.label(format!(
                    "{ICON_ARROWS_IN_LINE_HORIZONTAL} {LOG_ASSOCIATION_WINDOW_LABEL}"
                ))
                .on_hover_text(
                    "Maximum time between a log entry's timestamp and the nearest \
                     fix of the recording the log is associated with for the entry \
                     to take a position from it. For example, with a window of \
                     60s, a log line at 10:00:30 can associate with a fix from \
                     10:00:00 - but not one from 09:59:00.",
                );
                let mut window_s = Duration::from(self.assoc_config.log_association_window).num_seconds().unsigned_abs();
                ui.horizontal(|ui| {
                    self::compound_duration_input(
                        ui,
                        &mut window_s,
                        0..=Duration::from(AssociationWindow::MAX).num_seconds().unsigned_abs(),
                        DurationDisplay::Minutes,
                    );
                });
                match AssociationWindow::try_from_seconds(window_s) {
                    Ok(window) => self.assoc_config.log_association_window = window,
                    Err(error) => log::warn!("Invalid log association window: {error}"),
                }
                ui.end_row();

                let mut automatically_choose_position_source = self.initial_position_source_policy.automatically_chooses_position_source();
                SettingsLabel {
                    icon: ICON_CHAT_CIRCLE_TEXT,
                    text: "Automatically choose an\nunambiguous log position source",
                }
                .ui(ui)
                .on_hover_text(AUTOMATIC_POSITION_SOURCE_HOVER);
                ui.checkbox(&mut automatically_choose_position_source, "")
                    .on_hover_text(AUTOMATIC_POSITION_SOURCE_HOVER);
                self.initial_position_source_policy = InitialPositionSourcePolicy::from_automatic_selection(automatically_choose_position_source);
                ui.end_row();
            });

        self.show_generated_marker_settings(ui);
        self.show_processing_apply_and_restore_row(ui)
    }

    fn show_processing_apply_and_restore_row(&mut self, ui: &mut egui::Ui) -> bool {
        let mut apply = false;
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let apply_label = format!("{ICON_CHECK} {APPLY_TO_LOADED_DATA_LABEL}");
            if ui.button(apply_label).clicked() {
                apply = true;
            }
            let reset_label = format!("{ICON_ARROW_COUNTER_CLOCKWISE} {RESTORE_DEFAULTS_LABEL}");
            if ui.button(reset_label).clicked() {
                let defaults = ProcessingSettings::default();
                self.processing_config.track_layout.track_split_gap =
                    Duration::seconds(defaults.track_split_gap_seconds as i64);
                self.debug_time_repair_threshold_seconds =
                    defaults.debug_time_repair_backward_jump_threshold_seconds;
                self.assoc_config.log_association_window =
                    AssociationConfig::default().log_association_window;
                self.initial_position_source_policy = defaults.initial_position_source_policy;
                self.processing_config
                    .generated_markers
                    .detect_gnss_fix_lost = defaults.detect_gnss_fix_lost;
                self.processing_config
                    .generated_markers
                    .detect_gnss_fix_regained = defaults.detect_gnss_fix_regained;
                self.processing_config
                    .generated_markers
                    .detect_clock_discontinuities = defaults.detect_clock_discontinuities;
                self.processing_config
                    .generated_markers
                    .clock_discontinuity_sigmas = defaults.clock_discontinuity_sigmas;
                self.processing_config
                    .generated_markers
                    .detect_clock_offset_excursions = defaults.detect_clock_offset_excursions;
                self.processing_config.generated_markers.detect_slips = defaults.detect_slips;
            }
        });
        apply
    }

    /// Render the "Generated markers" settings section: a per-kind on/off toggle
    /// for each automatically-detected marker.  These are produced at
    /// load/segmentation time, so they share the same "Apply to loaded data"
    /// action as the rest of the processing settings.
    fn show_generated_marker_settings(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(ICON_MAP_PIN);
            ui.strong(GENERATED_MARKERS_LABEL);
        });
        ui.separator();
        Grid::new("generated_markers_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                for control in GENERATED_MARKER_CONTROLS {
                    control.show_row(ui, self);
                }
            });
    }
}

#[derive(Clone, Copy)]
pub(super) struct ProcessingControl {
    key: ProcessingSettingKey,
    icon: &'static str,
    pub(super) label: &'static str,
    help: &'static str,
    input: ProcessingInput,
}

impl ProcessingControl {
    fn track_split_gap_range() -> RangeInclusive<f64> {
        *TRACK_SPLIT_GAP_RANGE.start() as f64..=*TRACK_SPLIT_GAP_RANGE.end() as f64
    }

    fn write_clock_sensitivity(config: &mut SegmentationConfig, value: f64) -> bool {
        std::mem::replace(
            &mut config.generated_markers.clock_discontinuity_sigmas,
            value,
        )
        .to_bits()
            != value.to_bits()
    }

    fn write_track_split_gap(config: &mut SegmentationConfig, seconds: u64) -> bool {
        let gap = Duration::seconds(seconds as i64);
        std::mem::replace(&mut config.track_layout.track_split_gap, gap) != gap
    }

    pub(super) const fn for_key(key: ProcessingSettingKey) -> Self {
        match key {
            ProcessingSettingKey::ClockDiscontinuitySigmas => Self {
                key,
                icon: ICON_WARNING,
                label: "Jump sensitivity",
                help: "The clock discontinuity test compares each jump with the track's normal \
                       clock variation in robust standard deviations. Lower values flag smaller \
                       jumps. Enable Clock discontinuity to adjust the sensitivity.",
                input: ProcessingInput::ClockSensitivity,
            },
            ProcessingSettingKey::DetectClockDiscontinuities => Self {
                key,
                icon: ICON_WARNING,
                label: "Clock discontinuity",
                help: "Mark abrupt jumps in the GPS/system clock offset. For example, a device \
                       can report an old GPS timestamp with the current system timestamp after \
                       resuming from suspend.",
                input: ProcessingInput::MarkerToggle(|markers| {
                    &mut markers.detect_clock_discontinuities
                }),
            },
            ProcessingSettingKey::DetectClockOffsetExcursions => Self {
                key,
                icon: ICON_WARNING,
                label: CLOCK_OFFSET_EXCURSION_LABEL,
                help: "Mark isolated departures from the track's baseline GPS/system clock \
                       offset. For example, a receiver can report its last GPS timestamp from \
                       before a recording gap on the first fix after the gap. The Analysis page \
                       sets the threshold for both markers and the plot's off-scale indicators.",
                input: ProcessingInput::MarkerToggle(|markers| {
                    &mut markers.detect_clock_offset_excursions
                }),
            },
            ProcessingSettingKey::DetectGnssFixLost => Self {
                key,
                icon: ICON_X_CIRCLE,
                label: "GNSS fix lost",
                help: "Place a marker at the last successful GNSS fix before each loss",
                input: ProcessingInput::MarkerToggle(|markers| &mut markers.detect_gnss_fix_lost),
            },
            ProcessingSettingKey::DetectGnssFixRegained => Self {
                key,
                icon: ICON_CHECK_CIRCLE,
                label: "GNSS fix regained",
                help: "Mark the first GNSS fix after a loss. The marker shows the elapsed time \
                       since the previous fix.",
                input: ProcessingInput::MarkerToggle(|markers| {
                    &mut markers.detect_gnss_fix_regained
                }),
            },
            ProcessingSettingKey::DetectSlips => Self {
                key,
                icon: ICON_LINK_BREAK,
                label: "Satellite slip",
                help: "Mark each loss of lock when a satellite at or above the elevation mask \
                       disappears or its SNR drops sharply between epochs. A marker's tooltip \
                       shows the satellite and its elevation, azimuth and SNR before and after \
                       the slip. The Analysis page sets the elevation mask and SNR threshold for \
                       both markers and the slip-rate plot.",
                input: ProcessingInput::MarkerToggle(|markers| &mut markers.detect_slips),
            },
            ProcessingSettingKey::TrackSplitGap => Self {
                key,
                icon: ICON_SCISSORS,
                label: "Track split gap",
                help: "Start a new track when the timestamps of consecutive fixes differ by at \
                       least this duration in either direction. With a five-minute gap, fixes \
                       at 10:00 and 10:06 start separate tracks in either order. The split gap \
                       has a resolution of one second.",
                input: ProcessingInput::TrackSplitGap,
            },
        }
    }

    pub(super) fn catalog() -> impl Iterator<Item = Self> {
        std::iter::once(TRACK_SPLIT_GAP).chain(
            GENERATED_MARKER_CONTROLS
                .iter()
                .flat_map(|row| std::iter::once(*row).chain(row.sensitivity())),
        )
    }

    pub(super) fn set(
        &self,
        config: &mut SegmentationConfig,
        input: SettingInput,
    ) -> Result<bool, ErrorCode> {
        match (self.input, input) {
            (ProcessingInput::ClockSensitivity, SettingInput::Number(value)) => {
                if !self.enabled(config) {
                    return Err(ErrorCode::InactiveSetting);
                }
                if !CLOCK_SENSITIVITY_RANGE.contains(&value) {
                    return Err(ErrorCode::InvalidSettingValue);
                }
                Ok(Self::write_clock_sensitivity(config, value))
            }
            (ProcessingInput::MarkerToggle(field), SettingInput::Boolean(value)) => {
                Ok(std::mem::replace(field(&mut config.generated_markers), value) != value)
            }
            (ProcessingInput::TrackSplitGap, SettingInput::Number(value)) => {
                if !Self::track_split_gap_range().contains(&value) || value.fract() != 0.0 {
                    return Err(ErrorCode::InvalidSettingValue);
                }
                Ok(Self::write_track_split_gap(config, u64::from_f64(value)))
            }
            _ => Err(ErrorCode::InvalidSettingType),
        }
    }

    pub(super) fn describe(self, mut config: SegmentationConfig) -> SettingInfo {
        let value = match self.input {
            ProcessingInput::ClockSensitivity => {
                let value = config.generated_markers.clock_discontinuity_sigmas;
                SettingValue::Number {
                    value: value.is_finite().then_some(value),
                    minimum: *CLOCK_SENSITIVITY_RANGE.start(),
                    maximum: *CLOCK_SENSITIVITY_RANGE.end(),
                    unit: SettingUnit::RobustStandardDeviations,
                }
            }
            ProcessingInput::MarkerToggle(field) => SettingValue::Boolean {
                value: *field(&mut config.generated_markers),
            },
            ProcessingInput::TrackSplitGap => {
                let range = Self::track_split_gap_range();
                SettingValue::Number {
                    value: Some(config.track_layout.track_split_gap.as_seconds_f64()),
                    minimum: *range.start(),
                    maximum: *range.end(),
                    unit: SettingUnit::Seconds,
                }
            }
        };
        SettingInfo {
            key: SettingKey::Processing(self.key),
            page: SettingsPage::Processing.rail_label().to_owned(),
            label: self.label.to_owned(),
            help: self.help.to_owned(),
            apply: SettingApply::DeferredProcessing,
            value,
        }
    }

    fn show_row(&self, ui: &mut egui::Ui, app: &mut App) {
        let label = ui
            .label(format!("{} {}", self.icon, self.label))
            .on_hover_text(self.help);
        let input = if let Some(sensitivity) = self.sensitivity() {
            let row = ui.horizontal(|ui| {
                self.show_input(ui, &mut app.processing_config);
                sensitivity.show_input(ui, &mut app.processing_config)
            });
            app.reveal_setting_row(ui, SettingKey::Processing(sensitivity.key), row.inner.rect);
            row.response
        } else {
            self.show_input(ui, &mut app.processing_config)
        };
        let row = label.rect.union(input.rect);
        let target = if row.width() <= ui.clip_rect().width() {
            row
        } else {
            input.rect
        };
        app.reveal_setting_row(ui, SettingKey::Processing(self.key), target);
        ui.end_row();
    }

    fn show_input(&self, ui: &mut egui::Ui, config: &mut SegmentationConfig) -> egui::Response {
        match self.input {
            ProcessingInput::ClockSensitivity => {
                let enabled = self.enabled(config);
                let example_sigmas = config.generated_markers.clock_discontinuity_sigmas;
                let floor_s = gt_track_builder::clock_discontinuity_floor_seconds(example_sigmas);
                let response = ui.add_enabled(
                    enabled,
                    DragValue::from_get_set(|new| {
                        if enabled && let Some(new) = new.filter(|new| new.is_finite()) {
                            Self::write_clock_sensitivity(
                                config,
                                new.clamp(
                                    *CLOCK_SENSITIVITY_RANGE.start(),
                                    *CLOCK_SENSITIVITY_RANGE.end(),
                                ),
                            );
                        }
                        config.generated_markers.clock_discontinuity_sigmas
                    })
                    .range(CLOCK_SENSITIVITY_RANGE)
                    .clamp_existing_to_range(false)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix("σ"),
                );
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_label(self.label));
                if enabled {
                    response.on_hover_text(format!(
                        "{}\n\nOn a steady recording, {example_sigmas:.1}σ flags jumps larger \
                         than about {floor_s:.1}s. The threshold rises with the track's variation.",
                        self.help,
                    ))
                } else {
                    response.on_disabled_hover_text(self.help)
                }
            }
            ProcessingInput::MarkerToggle(field) => ui
                .checkbox(field(&mut config.generated_markers), "")
                .on_hover_text(self.help),
            ProcessingInput::TrackSplitGap => {
                let gap = config.track_layout.track_split_gap;
                ui.horizontal(|ui| {
                    let mut gap_secs = gap.to_std().map_or_else(
                        |_| ProcessingSettings::default().track_split_gap_seconds,
                        |duration| duration.as_secs(),
                    );
                    let initial_secs = gap_secs;
                    self::compound_duration_input(
                        ui,
                        &mut gap_secs,
                        TRACK_SPLIT_GAP_RANGE,
                        DurationDisplay::Days,
                    );
                    if gap_secs != initial_secs {
                        Self::write_track_split_gap(config, gap_secs);
                    }
                })
                .response
            }
        }
    }

    fn sensitivity(&self) -> Option<Self> {
        (self.key == ProcessingSettingKey::DetectClockDiscontinuities).then_some(Self::for_key(
            ProcessingSettingKey::ClockDiscontinuitySigmas,
        ))
    }

    fn enabled(&self, config: &SegmentationConfig) -> bool {
        !matches!(self.input, ProcessingInput::ClockSensitivity)
            || config.generated_markers.detect_clock_discontinuities
    }
}

#[derive(Clone, Copy)]
enum ProcessingInput {
    ClockSensitivity,
    MarkerToggle(fn(&mut GeneratedMarkerConfig) -> &mut bool),
    TrackSplitGap,
}

#[derive(Clone, Copy)]
enum DurationDisplay {
    Days,
    Minutes,
}

fn compound_duration_input(
    ui: &mut egui::Ui,
    value_secs: &mut u64,
    range: RangeInclusive<u64>,
    display: DurationDisplay,
) {
    let mut remaining = *value_secs;
    let show_days = matches!(display, DurationDisplay::Days);
    let (mut days, mut hours) = if show_days {
        let days = remaining / SECONDS_PER_DAY;
        remaining %= SECONDS_PER_DAY;
        let hours = remaining / SECONDS_PER_HOUR;
        remaining %= SECONDS_PER_HOUR;
        (days, hours)
    } else {
        (0, 0)
    };
    let mut minutes = remaining / SECONDS_PER_MINUTE;
    let mut seconds = remaining % SECONDS_PER_MINUTE;
    let mut add_component = |value: &mut u64, maximum: u64, suffix| {
        let initial = *value;
        ui.add(
            DragValue::from_get_set(|new| {
                if let Some(new) = new.filter(|new| new.is_finite()) {
                    *value = u64::from_f64(new).min(maximum);
                }
                *value as f64
            })
            .range(0..=maximum)
            .clamp_existing_to_range(false)
            .speed(0.25)
            .max_decimals(0)
            .suffix(suffix),
        );
        *value != initial
    };

    let mut changed = false;
    if show_days {
        changed |= add_component(&mut days, range.end() / SECONDS_PER_DAY, "d");
        changed |= add_component(&mut hours, 23, "h");
    }
    let max_minutes = if show_days {
        59
    } else {
        range.end() / SECONDS_PER_MINUTE
    };
    changed |= add_component(&mut minutes, max_minutes, "m");
    changed |= add_component(&mut seconds, 59, "s");

    if changed {
        let total = days * SECONDS_PER_DAY
            + hours * SECONDS_PER_HOUR
            + minutes * SECONDS_PER_MINUTE
            + seconds;
        *value_secs = total.clamp(*range.start(), *range.end());
    }
}

const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;
const TRACK_SPLIT_GAP_RANGE: RangeInclusive<u64> = 30..=7 * SECONDS_PER_DAY;
const CLOCK_SENSITIVITY_RANGE: RangeInclusive<f64> = 1.0..=20.0;
const TRACK_SPLIT_GAP: ProcessingControl =
    ProcessingControl::for_key(ProcessingSettingKey::TrackSplitGap);
const GENERATED_MARKER_CONTROLS: &[ProcessingControl] = &[
    ProcessingControl::for_key(ProcessingSettingKey::DetectGnssFixLost),
    ProcessingControl::for_key(ProcessingSettingKey::DetectGnssFixRegained),
    ProcessingControl::for_key(ProcessingSettingKey::DetectClockDiscontinuities),
    ProcessingControl::for_key(ProcessingSettingKey::DetectClockOffsetExcursions),
    ProcessingControl::for_key(ProcessingSettingKey::DetectSlips),
];

const DEBUG_BACKWARD_JUMP_THRESHOLD_LABEL: &str = "Debug backward jump threshold";
const LOG_ASSOCIATION_WINDOW_LABEL: &str = "Log association window";
pub(in crate::app) const AUTOMATIC_LOG_POSITION_SOURCE_LABEL: &str =
    "Automatically choose an unambiguous log position source";
const GENERATED_MARKERS_LABEL: &str = "Generated markers";
const APPLY_TO_LOADED_DATA_LABEL: &str = "Apply to loaded data";
const RESTORE_DEFAULTS_LABEL: &str = "Restore defaults";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    DEBUG_BACKWARD_JUMP_THRESHOLD_LABEL,
    LOG_ASSOCIATION_WINDOW_LABEL,
    AUTOMATIC_LOG_POSITION_SOURCE_LABEL,
    GENERATED_MARKERS_LABEL,
    APPLY_TO_LOADED_DATA_LABEL,
    RESTORE_DEFAULTS_LABEL,
];
