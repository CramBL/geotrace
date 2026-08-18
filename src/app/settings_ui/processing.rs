//! Settings applied when a recording is loaded or explicitly re-applied.

use egui::{DragValue, Grid};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::ARROWS_IN_LINE_HORIZONTAL as ICON_ARROWS_IN_LINE_HORIZONTAL;
use egui_phosphor::regular::CHECK as ICON_CHECK;
use egui_phosphor::regular::CHECK_CIRCLE as ICON_CHECK_CIRCLE;
use egui_phosphor::regular::LINK_BREAK as ICON_LINK_BREAK;
use egui_phosphor::regular::MAP_PIN as ICON_MAP_PIN;
use egui_phosphor::regular::SCISSORS as ICON_SCISSORS;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use egui_phosphor::regular::X_CIRCLE as ICON_X_CIRCLE;

use crate::app::App;
use crate::app::settings_ui::SettingsPage;
use crate::app::settings_ui::analysis::CLOCK_OFFSET_EXCURSION_LABEL;

const TRACK_SPLIT_GAP_LABEL: &str = "Track split gap";
const LOG_ASSOCIATION_WINDOW_LABEL: &str = "Log association window";
const GENERATED_MARKERS_LABEL: &str = "Generated markers";
const GNSS_FIX_LOST_LABEL: &str = "GNSS fix lost";
const GNSS_FIX_REGAINED_LABEL: &str = "GNSS fix regained";
const CLOCK_DISCONTINUITY_LABEL: &str = "Clock discontinuity";
const SATELLITE_SLIP_LABEL: &str = "Satellite slip";
const APPLY_TO_LOADED_DATA_LABEL: &str = "Apply to loaded data";
const RESTORE_DEFAULTS_LABEL: &str = "Restore defaults";

pub(super) const SEARCHABLE_LABELS: &[&str] = &[
    TRACK_SPLIT_GAP_LABEL,
    LOG_ASSOCIATION_WINDOW_LABEL,
    GENERATED_MARKERS_LABEL,
    GNSS_FIX_LOST_LABEL,
    GNSS_FIX_REGAINED_LABEL,
    CLOCK_DISCONTINUITY_LABEL,
    CLOCK_OFFSET_EXCURSION_LABEL,
    SATELLITE_SLIP_LABEL,
    APPLY_TO_LOADED_DATA_LABEL,
    RESTORE_DEFAULTS_LABEL,
];

impl App {
    /// Returns `true` in the frame when the user clicks "Apply to loaded data".
    pub(super) fn show_processing_page(&mut self, ui: &mut egui::Ui) -> bool {
        SettingsPage::Processing.show_header(ui);
        Grid::new("settings_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(format!("{ICON_SCISSORS} {TRACK_SPLIT_GAP_LABEL}"))
                    .on_hover_text(
                        "Consecutive GPS points separated by more than this gap \
                         start a new track. For example, with a gap of 5 min, two \
                         fixes at 10:00 and 10:06 would be split into separate tracks.",
                    );
                let mut gap_secs = self
                    .processing_config
                    .track_layout
                    .track_split_gap
                    .to_std()
                    .map_or(300, |d| d.as_secs());
                ui.horizontal(|ui| {
                    compound_duration_input(ui, &mut gap_secs, 30, 7 * 86400, true, true);
                });
                self.processing_config.track_layout.track_split_gap =
                    chrono::Duration::seconds(gap_secs as i64);
                ui.end_row();

                ui.label(format!(
                    "{ICON_ARROWS_IN_LINE_HORIZONTAL} {LOG_ASSOCIATION_WINDOW_LABEL}"
                ))
                .on_hover_text(
                    "Maximum time between a log entry's timestamp and the nearest \
                     fix of the recording the log is associated with for the entry \
                     to take a position from it. For example, with a window of \
                     60 s, a log line at 10:00:30 can associate with a fix from \
                     10:00:00 - but not one from 09:59:00.",
                );
                let mut window_s = self.assoc_config.log_association_window_s.clamp(1, 3600);
                ui.horizontal(|ui| {
                    compound_duration_input(ui, &mut window_s, 1, 3600, false, false);
                });
                self.assoc_config.log_association_window_s = window_s;
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
                let defaults = crate::settings::ProcessingSettings::default();
                self.processing_config.track_layout.track_split_gap =
                    chrono::Duration::seconds(defaults.track_split_gap_seconds as i64);
                self.assoc_config.log_association_window_s = defaults.log_association_window_s;
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
                let fix_lost_help =
                    "Mark each epoch where the GNSS fix dropped (the receiver stopped resolving \
                     a position). For example, entering a tunnel typically drops the fix.";
                ui.label(format!("{ICON_X_CIRCLE} {GNSS_FIX_LOST_LABEL}"))
                    .on_hover_text(fix_lost_help);
                ui.checkbox(
                    &mut self.processing_config.generated_markers.detect_gnss_fix_lost,
                    "",
                )
                .on_hover_text(fix_lost_help);
                ui.end_row();

                let fix_regained_help =
                    "Mark each epoch where the GNSS fix returned after being lost, annotated with \
                     how long it was gone.";
                ui.label(format!("{ICON_CHECK_CIRCLE} {GNSS_FIX_REGAINED_LABEL}"))
                    .on_hover_text(fix_regained_help);
                ui.checkbox(
                    &mut self
                        .processing_config
                        .generated_markers
                        .detect_gnss_fix_regained,
                    "",
                )
                .on_hover_text(fix_regained_help);
                ui.end_row();

                // Clock discontinuity: the toggle and its jump sensitivity share
                // one row. The sensitivity stays visible but grays out when
                // detection is off rather than hiding (DESIGN.md: keep the layout
                // stable and the feature discoverable).
                let clock_help =
                    "Flag abrupt jumps in the GPS/system clock offset - e.g. a device resuming \
                     from suspend, where a stale GPS timestamp meets a fresh system timestamp. \
                     Surfaced for inspection; the underlying data is never altered.";
                ui.label(format!("{ICON_WARNING} {CLOCK_DISCONTINUITY_LABEL}"))
                    .on_hover_text(clock_help);
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut self
                            .processing_config
                            .generated_markers
                            .detect_clock_discontinuities,
                        "",
                    )
                    .on_hover_text(clock_help);
                    let detect_on = self
                        .processing_config
                        .generated_markers
                        .detect_clock_discontinuities;
                    let sigmas = self
                        .processing_config
                        .generated_markers
                        .clock_discontinuity_sigmas;
                    let floor_s = gt_track_builder::clock_discontinuity_floor_seconds(sigmas);
                    let sensitivity = ui.add_enabled(
                        detect_on,
                        DragValue::new(
                            &mut self
                                .processing_config
                                .generated_markers
                                .clock_discontinuity_sigmas,
                        )
                        .range(1.0..=20.0)
                        .speed(0.1)
                        .fixed_decimals(1)
                        .suffix(" σ"),
                    );
                    if detect_on {
                        sensitivity.on_hover_text(format!(
                            "Jump sensitivity: how far a jump must stand out from the track's \
                             normal clock variation to be flagged, in robust standard deviations. \
                             Lower flags more (smaller) jumps; higher flags only the most extreme.\
                             \n\nFor example, on a steady recording {sigmas:.1} σ flags jumps \
                             larger than about {floor_s:.1} s; on a noisier one the bar rises \
                             with the track's own variation.",
                        ));
                    } else {
                        sensitivity.on_hover_text(
                            "Enable clock discontinuity to adjust the jump sensitivity",
                        );
                    }
                });
                ui.end_row();

                let excursion_help =
                    "Mark each clock offset excursion: a sample or two whose GPS/system clock \
                     offset left the track's baseline and returned - typically a receiver \
                     reporting its pre-gap GPS epoch for the first fix after a recording gap. \
                     The threshold comes from the Analysis page, so the markers and the \
                     plot's off-scale indicators agree.";
                ui.label(format!("{ICON_WARNING} {CLOCK_OFFSET_EXCURSION_LABEL}"))
                    .on_hover_text(excursion_help);
                ui.checkbox(
                    &mut self
                        .processing_config
                        .generated_markers
                        .detect_clock_offset_excursions,
                    "",
                )
                .on_hover_text(excursion_help);
                ui.end_row();

                let slip_help =
                    "Mark each loss-of-lock (cycle slip): an above-mask satellite that vanished, \
                     or whose SNR dropped sharply between epochs. Hover a marker for which \
                     satellite slipped and its before/after elevation, azimuth, and SNR. \
                     Detection uses the elevation mask and SNR-drop threshold from the Analysis \
                     page, so the markers and the slip-rate plot agree.";
                ui.label(format!("{ICON_LINK_BREAK} {SATELLITE_SLIP_LABEL}"))
                    .on_hover_text(slip_help);
                ui.checkbox(
                    &mut self.processing_config.generated_markers.detect_slips,
                    "",
                )
                .on_hover_text(slip_help);
                ui.end_row();
            });
    }
}

/// Renders compound duration fields (e.g. `[0d] [9h] [30m] [0s]`).
///
/// Each component is independent. The total is clamped to `[min_secs, max_secs]`.
fn compound_duration_input(
    ui: &mut egui::Ui,
    value_secs: &mut u64,
    min_secs: u64,
    max_secs: u64,
    show_days: bool,
    show_hours: bool,
) {
    let mut remaining = *value_secs;
    let mut d = if show_days {
        let v = remaining / 86400;
        remaining %= 86400;
        v
    } else {
        0
    };
    let mut h = if show_hours {
        let v = remaining / 3600;
        remaining %= 3600;
        v
    } else {
        0
    };
    let mut m = remaining / 60;
    let mut s = remaining % 60;

    let max_d = max_secs / 86400;
    let max_m = if show_hours { 59 } else { max_secs / 60 };

    let mut changed = false;
    if show_days {
        changed |= ui
            .add(DragValue::new(&mut d).range(0..=max_d).suffix("d"))
            .changed();
    }
    if show_hours {
        changed |= ui
            .add(DragValue::new(&mut h).range(0..=23).suffix("h"))
            .changed();
    }
    changed |= ui
        .add(DragValue::new(&mut m).range(0..=max_m).suffix("m"))
        .changed();
    changed |= ui
        .add(DragValue::new(&mut s).range(0..=59).suffix("s"))
        .changed();

    if changed {
        let total = d * 86400 + h * 3600 + m * 60 + s;
        *value_secs = total.clamp(min_secs, max_secs);
    }
}
