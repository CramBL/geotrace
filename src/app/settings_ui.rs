use egui::{DragValue, Grid, Window};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::ARROWS_IN_LINE_HORIZONTAL as ICON_ARROWS_IN_LINE_HORIZONTAL;
use egui_phosphor::regular::CHECK as ICON_CHECK;
use egui_phosphor::regular::CHECK_CIRCLE as ICON_CHECK_CIRCLE;
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::GAUGE as ICON_GAUGE;
use egui_phosphor::regular::LINK_BREAK as ICON_LINK_BREAK;
use egui_phosphor::regular::MAP_PIN as ICON_MAP_PIN;
use egui_phosphor::regular::SCISSORS as ICON_SCISSORS;
use egui_phosphor::regular::SLIDERS_HORIZONTAL as ICON_SLIDERS_HORIZONTAL;
use egui_phosphor::regular::TEXT_AA as ICON_TEXT_AA;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use egui_phosphor::regular::WAVE_SINE as ICON_WAVE_SINE;
use egui_phosphor::regular::X_CIRCLE as ICON_X_CIRCLE;
use gt_map::MapLayer;
use gt_snap::wire::Costing;
use gt_track_builder::{GeneratedMarkerConfig, SegmentationConfig, TrackLayoutConfig};
use gt_types::AssociationConfig;
use strum::IntoEnumIterator;

use super::backfill_ui::{BackfillAction, BackfillReadiness};
use super::{App, day_failures, geomagnetic_index_ui, recording_name_template};

impl App {
    /// What a download control may do right now. An archive that could not be
    /// opened outranks offline mode: it is the permanent condition.
    fn backfill_readiness(&self, archive_available: bool) -> BackfillReadiness {
        if !archive_available {
            BackfillReadiness::WithoutArchive
        } else if self.offline {
            BackfillReadiness::Offline
        } else {
            BackfillReadiness::Ready
        }
    }

    /// Render the Settings window.
    ///
    /// Returns `true` in the frame when the user clicks "Apply to loaded data",
    /// signalling that the caller should call `apply_resegmentation`.
    pub(super) fn show_settings_window(&mut self, ui: &egui::Ui) -> bool {
        if !self.settings_open {
            return false;
        }
        // The name-template preview reads a stored recording from the History
        // window's cached list when nothing is loaded.
        self.history_window
            .request_recording_list_if_missing(&self.history);
        let mut open = self.settings_open;
        let mut apply = false;
        Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(360.0)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(ICON_SLIDERS_HORIZONTAL);
                    ui.strong("Processing");
                });
                ui.separator();
                Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(format!(
                            "{} Track split gap",
                            ICON_SCISSORS
                        ))
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
                            "{} Log marker window",
                            ICON_ARROWS_IN_LINE_HORIZONTAL
                        ))
                        .on_hover_text(
                            "Maximum time between a log entry's timestamp and the nearest \
                             GPS fix for the entry to be placed on the map. For example, \
                             with a window of 60 s, a log line at 10:00:30 can associate \
                             with a fix from 10:00:00 - but not one from 09:59:00.",
                        );
                        let mut window_s = self.assoc_config.log_marker_window_s.clamp(1, 3600);
                        ui.horizontal(|ui| {
                            compound_duration_input(ui, &mut window_s, 1, 3600, false, false);
                        });
                        self.assoc_config.log_marker_window_s = window_s;
                        ui.end_row();
                    });

                self.show_generated_marker_settings(ui);

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let apply_label =
                        format!("{ICON_CHECK} Apply to loaded data");
                    if ui.button(apply_label).clicked() {
                        apply = true;
                    }
                    let reset_label = format!(
                        "{} Restore Defaults",
                        ICON_ARROW_COUNTER_CLOCKWISE
                    );
                    if ui.button(reset_label).clicked() {
                        let defaults = crate::settings::ProcessingSettings::default();
                        self.processing_config.track_layout.track_split_gap =
                            chrono::Duration::seconds(defaults.track_split_gap_seconds as i64);
                        self.assoc_config.log_marker_window_s = defaults.log_marker_window_s;
                        self.processing_config.generated_markers.detect_gnss_fix_lost =
                            defaults.detect_gnss_fix_lost;
                        self.processing_config.generated_markers.detect_gnss_fix_regained =
                            defaults.detect_gnss_fix_regained;
                        self.processing_config.generated_markers.detect_clock_discontinuities =
                            defaults.detect_clock_discontinuities;
                        self.processing_config.generated_markers.clock_discontinuity_sigmas =
                            defaults.clock_discontinuity_sigmas;
                        self.processing_config.generated_markers.detect_clock_offset_excursions =
                            defaults.detect_clock_offset_excursions;
                        self.processing_config.generated_markers.detect_slips = defaults.detect_slips;
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(ICON_GAUGE);
                    ui.strong("Analysis");
                });
                ui.separator();
                Grid::new("analysis_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        // Start from the analysis config the plot currently uses,
                        // then live-apply any edits below.
                        let mut analysis = self.shared.borrow().plot_state.analysis;

                        let mask_help =
                            "Satellites below this elevation are excluded from the 'in view' \
                             baseline of the utilization rate, and a satellite must sit above it \
                             to count toward the slip rate. This keeps the receiver from being \
                             penalised for ignoring low-elevation satellites (high atmospheric \
                             delay and multipath). \
                             For example, a 0° mask counts horizon satellites the receiver \
                             naturally rejects, which lowers the utilization rate and inflates the \
                             slip rate with routine horizon fades; a 30° mask considers only \
                             high, clean satellites.";
                        ui.label(format!("{ICON_FUNNEL} Elevation mask"))
                            .on_hover_text(mask_help);
                        ui.add(
                            DragValue::new(&mut analysis.elevation_mask_deg)
                                .range(0.0..=90.0)
                                .speed(1.0)
                                .fixed_decimals(0)
                                .suffix("°"),
                        )
                        .on_hover_text(mask_help);
                        ui.end_row();

                        let snr_help =
                            "An above-mask satellite whose SNR falls by more than this many dB-Hz \
                             between consecutive epochs is counted as a loss-of-lock (slip), a \
                             sign of a momentary signal break rather than ordinary variation. \
                             For example, a low threshold like 3 dB-Hz flags ordinary signal \
                             fluctuation as slips and inflates the rate; a high threshold like \
                             25 dB-Hz reports only near-total dropouts and may miss real slips.";
                        ui.label(format!("{ICON_WAVE_SINE} SNR drop threshold"))
                            .on_hover_text(snr_help);
                        ui.add(
                            DragValue::new(&mut analysis.snr_drop_db)
                                .range(1.0..=60.0)
                                .speed(0.5)
                                .fixed_decimals(0)
                                .suffix(" dB-Hz"),
                        )
                        .on_hover_text(snr_help);
                        ui.end_row();

                        let window_help =
                            "Trailing window over which the slip rate is averaged, in minutes. \
                             The plotted value is the slips counted in the window divided by its \
                             length, in slips per minute.";
                        ui.label(format!("{ICON_CLOCK} Slip window"))
                            .on_hover_text(window_help);
                        ui.add(
                            DragValue::new(&mut analysis.slip_window_min)
                                .range(1.0..=120.0)
                                .speed(1.0)
                                .fixed_decimals(0)
                                .suffix(" min"),
                        )
                        .on_hover_text(window_help);
                        ui.end_row();

                        let excursion_help =
                            "A sample whose GPS/system clock offset sits more than this far from \
                             the track's own baseline offset is treated as a clock offset \
                             excursion: it is kept off the clock offset line, so one sample \
                             carrying a whole recording gap cannot flatten the shared y-axis, and \
                             marked at the edge of the view instead. Hover a marker for its real \
                             offset. A large but steady offset sits at the baseline and is never \
                             an excursion, and an offset that steps and stays is a clock \
                             discontinuity, not an excursion.";
                        ui.label(format!("{ICON_WARNING} Clock offset excursion"))
                            .on_hover_text(excursion_help);
                        ui.add(
                            DragValue::new(&mut analysis.clock_excursion_threshold_s)
                                .range(gt_plot::CLOCK_EXCURSION_THRESHOLD_RANGE_S)
                                .speed(1.0)
                                .fixed_decimals(0)
                                .suffix(" s"),
                        )
                        .on_hover_text(excursion_help);
                        ui.end_row();

                        // Live-apply: re-derives only the analysis-dependent
                        // series, and keeps the loader in step for files loaded
                        // later. `set_analysis` is a no-op when unchanged.
                        self.loader.analysis_config = analysis;
                        // Slip markers share these detection params, but as
                        // load-time generated markers they only pick up the
                        // change on the next load or "Apply to loaded data".
                        self.processing_config.generated_markers.slip_elevation_mask_deg = analysis.elevation_mask_deg;
                        self.processing_config.generated_markers.slip_snr_drop_db = analysis.snr_drop_db;
                        self.processing_config.generated_markers.clock_excursion_threshold_s = analysis.clock_excursion_threshold_s;
                        {
                            let mut shared = self.shared.borrow_mut();
                            let s = &mut *shared;
                            s.plot_state.set_analysis(&s.loaded_files, analysis);
                        }

                        let mark_help =
                            "Place a red cross on the plot at each epoch where the receiver used a \
                             satellite below the elevation mask. Hover a marker to see which \
                             satellites and their elevation. Such satellites are excluded from the \
                             utilization rate, and the marker keeps that exclusion visible. Markers \
                             show with the 'Util all' plot.";
                        ui.label(format!(
                            "{} Mark masked-out used satellites",
                            ICON_WARNING
                        ))
                        .on_hover_text(mark_help);
                        let mut mark = self.shared.borrow().plot_state.mark_masked_fix;
                        if ui.checkbox(&mut mark, "").on_hover_text(mark_help).changed() {
                            self.shared.borrow_mut().plot_state.mark_masked_fix = mark;
                        }
                        ui.end_row();
                    });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(ICON_TEXT_AA);
                    ui.strong("Display");
                });
                ui.separator();
                Grid::new("display_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        let preview = self.name_template_preview_recording();
                        let mut template = self.shared.borrow().recording_name_template.clone();
                        if recording_name_template::recording_name_template_ui(
                            ui,
                            &mut template,
                            preview.as_ref(),
                        ) {
                            self.shared.borrow_mut().recording_name_template = template;
                        }
                        ui.end_row();
                    });

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
                        ui.label(format!("{} Server URL", egui_phosphor::regular::GLOBE_SIMPLE))
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
                                         manual trigger always jumps the queue. Enabling asks \
                                         for upload consent first when it has not been given \
                                         for the configured server.";
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
                                label: format!(
                                    "{} Search radius",
                                    egui_phosphor::regular::CIRCLE_DASHED
                                ),
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
                                label: format!(
                                    "{} Turn penalty",
                                    egui_phosphor::regular::ARROW_U_UP_LEFT
                                ),
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
                                label: format!(
                                    "{} GPS accuracy",
                                    egui_phosphor::regular::CROSSHAIR
                                ),
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

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(egui_phosphor::regular::AIRPLANE_TILT);
                    ui.strong("Aircraft interference");
                });
                ui.separator();
                egui::Grid::new("interference_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        let url_help = "Base URL of the host serving the daily interference \
                                        datasets. The default is gpsjam.org; point it at a \
                                        mirror or an offline copy to fetch from there instead. \
                                        Requests carry a date and nothing about your recordings.";
                        ui.label(format!(
                            "{} Base URL",
                            egui_phosphor::regular::GLOBE_SIMPLE
                        ))
                        .on_hover_text(url_help);
                        let mut base_url = self.interference_settings.base_url.clone();
                        if ui
                            .text_edit_singleline(&mut base_url)
                            .on_hover_text(url_help)
                            .changed()
                        {
                            self.jamming.set_base_url(&base_url);
                            self.interference_settings.base_url = base_url;
                        }
                        ui.end_row();
                    });
                ui.add_space(8.0);
                let readiness = self.backfill_readiness(self.jamming.archive_available());
                if let Some(action) =
                    self.interference_backfill_ui
                        .ui(ui, self.jamming.backfill_progress(), readiness)
                {
                    match action {
                        BackfillAction::Start { from, to } => {
                            let queued = self.jamming.backfill(from, to);
                            self.interference_backfill_ui.report_started(queued);
                        }
                        BackfillAction::Cancel => self.jamming.cancel_backfill(),
                    }
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(egui_phosphor::regular::MAGNET);
                    ui.strong("Geomagnetic indices");
                });
                ui.separator();
                egui::Grid::new("geomagnetic_index_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        let url_help = "Base URL of the host serving the Kp and Hp30 \
                                        geomagnetic indices. The default is GFZ Potsdam, which \
                                        publishes them. Point it at a mirror or an offline copy \
                                        to fetch from there instead. Requests carry a date range \
                                        and nothing about your recordings.";
                        ui.label(format!(
                            "{} Base URL",
                            egui_phosphor::regular::GLOBE_SIMPLE
                        ))
                        .on_hover_text(url_help);
                        let mut base_url = self.geomagnetic_index_settings.base_url.clone();
                        if ui
                            .text_edit_singleline(&mut base_url)
                            .on_hover_text(url_help)
                            .changed()
                        {
                            self.geomagnetic_indices.set_base_url(&base_url);
                            self.geomagnetic_index_settings.base_url = base_url;
                        }
                        ui.end_row();

                        geomagnetic_index_ui::show_fetch_rows(
                            ui,
                            self.geomagnetic_indices.fetch_status(),
                        );
                    });
                day_failures::show_failures(
                    ui,
                    "geomagnetic_index_failures",
                    self.geomagnetic_indices.failures(),
                );
                ui.add_space(8.0);
                let readiness =
                    self.backfill_readiness(self.geomagnetic_indices.archive_available());
                if let Some(action) = self.geomagnetic_index_backfill_ui.ui(
                    ui,
                    self.geomagnetic_indices.backfill_progress(),
                    readiness,
                ) {
                    match action {
                        BackfillAction::Start { from, to } => {
                            let queued = self.geomagnetic_indices.backfill(from, to);
                            self.geomagnetic_index_backfill_ui.report_started(queued);
                        }
                        BackfillAction::Cancel => self.geomagnetic_indices.cancel_backfill(),
                    }
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(egui_phosphor::regular::WAVES);
                    ui.strong("Ionospheric TEC");
                });
                ui.separator();
                egui::Grid::new("tec_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        let url_help = "Base URL of the host serving the global ionosphere maps. \
                                        The default is JPL, which publishes them. Point it at a \
                                        mirror or an offline copy to fetch from there instead. \
                                        Requests carry a date and nothing about your recordings.";
                        ui.label(format!(
                            "{} Base URL",
                            egui_phosphor::regular::GLOBE_SIMPLE
                        ))
                        .on_hover_text(url_help);
                        let mut base_url = self.tec_settings.base_url.clone();
                        if ui
                            .text_edit_singleline(&mut base_url)
                            .on_hover_text(url_help)
                            .changed()
                        {
                            self.tec_maps.set_base_url(&base_url);
                            self.tec_settings.base_url = base_url;
                        }
                        ui.end_row();
                    });
                day_failures::show_failures(ui, "tec_failures", self.tec_maps.failures());

                // Only meaningful in dist builds. Builds without the self-update
                // feature carry no update check to toggle.
                #[cfg(feature = "self-update")]
                {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.checkbox(
                        &mut self.update_check_on_startup,
                        "Check for updates on startup",
                    )
                    .on_hover_text(
                        "Check for a newer GeoTrace release on startup and prompt to install it. \
                         Always off in development builds and when GEOTRACE_OFFLINE is set.",
                    );
                }
            });

        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }

        self.settings_open = open;
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
            ui.strong("Generated markers");
        });
        ui.separator();
        Grid::new("generated_markers_grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                let fix_lost_help =
                    "Mark each epoch where the GNSS fix dropped (the receiver stopped resolving \
                     a position). For example, entering a tunnel typically drops the fix.";
                ui.label(format!("{ICON_X_CIRCLE} GNSS fix lost"))
                    .on_hover_text(fix_lost_help);
                ui.checkbox(&mut self.processing_config.generated_markers.detect_gnss_fix_lost, "")
                    .on_hover_text(fix_lost_help);
                ui.end_row();

                let fix_regained_help =
                    "Mark each epoch where the GNSS fix returned after being lost, annotated with \
                     how long it was gone.";
                ui.label(format!(
                    "{} GNSS fix regained",
                    ICON_CHECK_CIRCLE
                ))
                .on_hover_text(fix_regained_help);
                ui.checkbox(&mut self.processing_config.generated_markers.detect_gnss_fix_regained, "")
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
                ui.label(format!(
                    "{} Clock discontinuity",
                    ICON_WARNING
                ))
                .on_hover_text(clock_help);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.processing_config.generated_markers.detect_clock_discontinuities, "")
                        .on_hover_text(clock_help);
                    let detect_on = self.processing_config.generated_markers.detect_clock_discontinuities;
                    let sigmas = self.processing_config.generated_markers.clock_discontinuity_sigmas;
                    let floor_s = gt_track_builder::clock_discontinuity_floor_seconds(sigmas);
                    let sensitivity = ui.add_enabled(
                        detect_on,
                        DragValue::new(&mut self.processing_config.generated_markers.clock_discontinuity_sigmas)
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
                     The threshold comes from the Analysis section, so the markers and the \
                     plot's off-scale indicators agree.";
                ui.label(format!("{ICON_WARNING} Clock offset excursion"))
                    .on_hover_text(excursion_help);
                ui.checkbox(&mut self.processing_config.generated_markers.detect_clock_offset_excursions, "")
                    .on_hover_text(excursion_help);
                ui.end_row();

                let slip_help =
                    "Mark each loss-of-lock (cycle slip): an above-mask satellite that vanished, \
                     or whose SNR dropped sharply between epochs. Hover a marker for which \
                     satellite slipped and its before/after elevation, azimuth, and SNR. \
                     Detection uses the elevation mask and SNR-drop threshold from the Analysis \
                     section, so the markers and the slip-rate plot agree.";
                ui.label(format!("{ICON_LINK_BREAK} Satellite slip"))
                    .on_hover_text(slip_help);
                ui.checkbox(&mut self.processing_config.generated_markers.detect_slips, "")
                    .on_hover_text(slip_help);
                ui.end_row();
            });
    }

    /// Apply loaded settings on startup.
    pub(super) fn apply_startup_settings(&mut self, s: &crate::settings::Settings) {
        if !s.map.mapbox_token.is_empty() {
            self.map.set_mapbox_token(s.map.mapbox_token.clone());
        }
        self.map.set_layer(map_layer_from_setting(s.map.layer));
        // One value behind both the plot's off-scale indicators and the
        // excursion markers, clamped here so a hand-edited config cannot put
        // the threshold outside what the settings control can reach back.
        let excursion_threshold = s.analysis.clock_excursion_threshold_s.clamp(
            *gt_plot::CLOCK_EXCURSION_THRESHOLD_RANGE_S.start(),
            *gt_plot::CLOCK_EXCURSION_THRESHOLD_RANGE_S.end(),
        );
        self.processing_config = SegmentationConfig {
            track_layout: TrackLayoutConfig {
                track_split_gap: chrono::Duration::seconds(
                    s.processing.track_split_gap_seconds as i64,
                ),
            },
            generated_markers: GeneratedMarkerConfig {
                detect_gnss_fix_lost: s.processing.detect_gnss_fix_lost,
                detect_gnss_fix_regained: s.processing.detect_gnss_fix_regained,
                detect_clock_discontinuities: s.processing.detect_clock_discontinuities,
                clock_discontinuity_sigmas: s.processing.clock_discontinuity_sigmas,
                detect_clock_offset_excursions: s.processing.detect_clock_offset_excursions,
                detect_slips: s.processing.detect_slips,
                // Slip and excursion markers share the plot's detection params
                // so the markers and the plot always agree.
                clock_excursion_threshold_s: excursion_threshold,
                slip_elevation_mask_deg: s.analysis.elevation_mask_deg,
                slip_snr_drop_db: s.analysis.snr_drop_db,
            },
        };
        self.assoc_config = AssociationConfig {
            log_marker_window_s: s.processing.log_marker_window_s,
        };
        self.ctx.set_theme(theme_pref_from_setting(s.ui.theme));

        let analysis = gt_plot::AnalysisConfig {
            elevation_mask_deg: s.analysis.elevation_mask_deg,
            snr_drop_db: s.analysis.snr_drop_db,
            slip_window_min: s.analysis.slip_window_min,
            clock_excursion_threshold_s: excursion_threshold,
        };
        self.loader.analysis_config = analysis;
        self.sky_trails_window
            .set_trail_opacity_percent(s.map.sky_trail_opacity_percent);
        {
            let mut shared = self.shared.borrow_mut();
            shared.plot_state.sync_to_map = s.map.sync_to_map;
            shared.display_mask = s.map.display_mask;
            shared.sky_glyph_variant = s.map.sky_glyph_variant;
            shared.point_window_folds = s.map.point_window_folds;
            shared.recording_name_template = s.ui.recording_name_template.clone();
            shared.plot_state.show_grid = s.plot.show_grid;
            shared.plot_state.line_width = s.plot.line_width.clamp(
                *gt_plot::PLOT_LINE_WIDTH_RANGE.start(),
                *gt_plot::PLOT_LINE_WIDTH_RANGE.end(),
            );
            shared.plot_state.show_advanced_metrics = s.plot.show_advanced_metrics;
            shared.plot_state.show_channels = s.plot.show_channels;
            shared.plot_state.analysis = analysis;
            shared.plot_state.mark_masked_fix = s.analysis.mark_masked_fix;
            let vis = &mut shared.plot_state.metric_vis;
            for k in crate::settings::MetricKind::iter() {
                vis.set(
                    k,
                    s.plot
                        .metric
                        .get(&k)
                        .copied()
                        .unwrap_or_else(|| k.visible_by_default()),
                );
            }
            let channel_vis = &mut shared.plot_state.channel_vis;
            for (name, &visible) in &s.plot.channel {
                channel_vis.set(name, visible);
            }
            shared.plot_state.channel_component_colors = s
                .plot
                .channel_colors
                .iter()
                .map(|(name, entries)| (name.clone(), super::dense_component_colors(entries)))
                .collect();
        }

        self.tiles_tree
            .tiles
            .set_visible(self.plot_tile_id, s.plot.panel_visible);
        self.set_split_ratio(s.plot.split_ratio);

        self.storage_enabled = s.storage.enabled;
        self.auto_prune_enabled = s.storage.auto_prune_enabled;
        self.auto_prune_max_bytes = s.storage.auto_prune_max_bytes;
        self.auto_prune_confirm = s.storage.auto_prune_confirm;
        self.update_check_on_startup = s.update.check_on_startup;
        self.skipped_version = s.update.skipped_version.clone();
        self.query_window.set_history(s.query.history.clone());
        self.snap_settings = s.snap.clone();
        self.interference_settings = s.interference.clone();
        self.jamming.set_base_url(&s.interference.base_url);
        self.geomagnetic_index_settings = s.geomagnetic_indices.clone();
        self.geomagnetic_indices
            .set_base_url(&s.geomagnetic_indices.base_url);
        self.tec_settings = s.tec.clone();
        self.tec_maps.set_base_url(&s.tec.base_url);
        self.snap.set_server_url(&s.snap.server_url);
        self.sync_db_path();
    }

    /// Whether to run the startup update check: enabled in settings, a release
    /// build (avoids hitting GitHub during development), and not offline.
    #[cfg(feature = "self-update")]
    pub(super) fn should_check_for_updates(&self) -> bool {
        self.update_check_on_startup && !cfg!(debug_assertions) && !self.offline
    }

    pub(super) fn collect_settings_for_flush(&self) -> crate::settings::Settings {
        let s = self.shared.borrow();
        let vis = &s.plot_state.metric_vis;
        let metric = crate::settings::MetricKind::iter()
            .map(|k| (k, vis.field(k)))
            .collect();
        let theme = self
            .ctx
            .options(|o| theme_pref_to_setting(o.theme_preference));
        crate::settings::Settings {
            version: 1,
            plot: crate::settings::PlotSettings {
                show_grid: s.plot_state.show_grid,
                line_width: s.plot_state.line_width,
                panel_visible: self.tiles_tree.tiles.is_visible(self.plot_tile_id),
                split_ratio: self.get_split_ratio(),
                metric,
                channel: s.plot_state.channel_vis.entries().into_iter().collect(),
                show_advanced_metrics: s.plot_state.show_advanced_metrics,
                show_channels: s.plot_state.show_channels,
                channel_colors: s
                    .plot_state
                    .channel_component_colors
                    .iter()
                    .map(|(name, colors)| (name.clone(), super::sparse_component_colors(colors)))
                    .collect(),
            },
            map: crate::settings::MapSettings {
                layer: map_layer_to_setting(self.map.layer()),
                mapbox_token: self.map.mapbox_token().to_owned(),
                sync_to_map: s.plot_state.sync_to_map,
                display_mask: s.display_mask,
                sky_glyph_variant: s.sky_glyph_variant,
                point_window_folds: s.point_window_folds,
                sky_trail_opacity_percent: self.sky_trails_window.trail_opacity_percent(),
            },
            ui: crate::settings::UiSettings {
                theme,
                recording_name_template: s.recording_name_template.clone(),
            },
            processing: crate::settings::ProcessingSettings {
                track_split_gap_seconds: self
                    .processing_config
                    .track_layout
                    .track_split_gap
                    .to_std()
                    .map_or(300, |d| d.as_secs()),
                log_marker_window_s: self.assoc_config.log_marker_window_s,
                detect_gnss_fix_lost: self
                    .processing_config
                    .generated_markers
                    .detect_gnss_fix_lost,
                detect_gnss_fix_regained: self
                    .processing_config
                    .generated_markers
                    .detect_gnss_fix_regained,
                detect_clock_discontinuities: self
                    .processing_config
                    .generated_markers
                    .detect_clock_discontinuities,
                clock_discontinuity_sigmas: self
                    .processing_config
                    .generated_markers
                    .clock_discontinuity_sigmas,
                detect_clock_offset_excursions: self
                    .processing_config
                    .generated_markers
                    .detect_clock_offset_excursions,
                detect_slips: self.processing_config.generated_markers.detect_slips,
            },
            analysis: crate::settings::AnalysisSettings {
                elevation_mask_deg: s.plot_state.analysis.elevation_mask_deg,
                mark_masked_fix: s.plot_state.mark_masked_fix,
                snr_drop_db: s.plot_state.analysis.snr_drop_db,
                slip_window_min: s.plot_state.analysis.slip_window_min,
                clock_excursion_threshold_s: s.plot_state.analysis.clock_excursion_threshold_s,
            },
            storage: crate::settings::StorageSettings {
                enabled: self.storage_enabled,
                auto_prune_enabled: self.auto_prune_enabled,
                auto_prune_max_bytes: self.auto_prune_max_bytes,
                auto_prune_confirm: self.auto_prune_confirm,
            },
            update: crate::settings::UpdateSettings {
                check_on_startup: self.update_check_on_startup,
                skipped_version: self.skipped_version.clone(),
            },
            query: crate::settings::QuerySettings {
                history: self.query_window.history().to_vec(),
            },
            snap: self.snap_settings.clone(),
            interference: self.interference_settings.clone(),
            geomagnetic_indices: self.geomagnetic_index_settings.clone(),
            tec: self.tec_settings.clone(),
        }
    }

    pub(super) fn flush_settings(&self) {
        let Some(path) = self.config_path.as_ref() else {
            log::warn!("Config directory unavailable - settings not saved");
            return;
        };
        let current = self.collect_settings_for_flush();
        let text = match toml::to_string_pretty(&current) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Failed to serialize config: {e:#}");
                return;
            }
        };
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!("Failed to create config dir {dir:?}: {e:#}");
            return;
        }
        let header = "# GeoTrace configuration - generated automatically.\n\
                      # WARNING: do not commit this file to a public repository if mapbox_token is set.\n\n";
        let full_text = format!("{header}{text}");
        let tmp = path.with_extension("toml.tmp");
        if let Err(e) = std::fs::write(&tmp, full_text.as_bytes()) {
            log::warn!("Failed to write config to {tmp:?}: {e:#}");
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            log::warn!("Failed to rename config file: {e:#}");
        }
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

pub(super) fn map_layer_to_setting(layer: MapLayer) -> crate::settings::MapLayerSetting {
    match layer {
        MapLayer::OpenStreetMap => crate::settings::MapLayerSetting::Osm,
        MapLayer::Satellite => crate::settings::MapLayerSetting::Satellite,
    }
}

fn map_layer_from_setting(s: crate::settings::MapLayerSetting) -> MapLayer {
    match s {
        crate::settings::MapLayerSetting::Osm => MapLayer::OpenStreetMap,
        crate::settings::MapLayerSetting::Satellite => MapLayer::Satellite,
    }
}

pub(super) fn theme_pref_to_setting(p: egui::ThemePreference) -> crate::settings::ThemeSetting {
    match p {
        egui::ThemePreference::System => crate::settings::ThemeSetting::System,
        egui::ThemePreference::Light => crate::settings::ThemeSetting::Light,
        egui::ThemePreference::Dark => crate::settings::ThemeSetting::Dark,
    }
}

fn theme_pref_from_setting(s: crate::settings::ThemeSetting) -> egui::ThemePreference {
    match s {
        crate::settings::ThemeSetting::System => egui::ThemePreference::System,
        crate::settings::ThemeSetting::Light => egui::ThemePreference::Light,
        crate::settings::ThemeSetting::Dark => egui::ThemePreference::Dark,
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
