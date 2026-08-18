//! Loading the persisted settings into the app and writing them back out.

use gt_track_builder::{GeneratedMarkerConfig, SegmentationConfig, TrackLayoutConfig};
use gt_types::AssociationConfig;
use strum::IntoEnumIterator;

use crate::app::App;

impl App {
    /// Apply loaded settings on startup.
    pub(in crate::app) fn apply_startup_settings(&mut self, s: &crate::settings::Settings) {
        if !s.map.mapbox_token.is_empty() {
            self.map.set_mapbox_token(s.map.mapbox_token.clone());
        }
        self.map
            .set_layer(super::map_layer_from_setting(s.map.layer));
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
            log_association_window_s: s.processing.log_association_window_s,
        };
        self.ctx
            .set_theme(super::theme_pref_from_setting(s.ui.theme));

        let analysis = gt_plot::AnalysisConfig {
            elevation_mask_deg: s.analysis.elevation_mask_deg,
            snr_drop_db: s.analysis.snr_drop_db,
            slip_window_min: s.analysis.slip_window_min,
            clock_excursion_threshold_s: excursion_threshold,
        };
        self.loader.analysis_config = analysis;
        self.sky_trails_window
            .set_trail_opacity_percent(s.map.sky_trail_opacity_percent);
        self.map
            .set_tec_heatmap_opacity_percent(s.map.tec_heatmap_opacity_percent);
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
                .map(|(name, entries)| (name.clone(), crate::app::dense_component_colors(entries)))
                .collect();
        }

        self.tiles_tree
            .tiles
            .set_visible(self.plot_tile_id, s.plot.panel_visible);
        self.set_split_ratio(s.plot.split_ratio);

        self.storage_settings = s.storage;
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
        self.tec_maps.set_mirrors(&s.tec.mirrors);
        self.snap.set_server_url(&s.snap.server_url);
        self.sync_db_path();
    }

    pub(in crate::app) fn collect_settings_for_flush(&self) -> crate::settings::Settings {
        let s = self.shared.borrow();
        let vis = &s.plot_state.metric_vis;
        let metric = crate::settings::MetricKind::iter()
            .map(|k| (k, vis.field(k)))
            .collect();
        let theme = self
            .ctx
            .options(|o| super::theme_pref_to_setting(o.theme_preference));
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
                    .map(|(name, colors)| {
                        (name.clone(), crate::app::sparse_component_colors(colors))
                    })
                    .collect(),
            },
            map: crate::settings::MapSettings {
                layer: super::map_layer_to_setting(self.map.layer()),
                mapbox_token: self.map.mapbox_token().to_owned(),
                sync_to_map: s.plot_state.sync_to_map,
                display_mask: s.display_mask,
                sky_glyph_variant: s.sky_glyph_variant,
                point_window_folds: s.point_window_folds,
                sky_trail_opacity_percent: self.sky_trails_window.trail_opacity_percent(),
                tec_heatmap_opacity_percent: self.map.tec_heatmap_opacity_percent(),
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
                log_association_window_s: self.assoc_config.log_association_window_s,
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
            storage: self.storage_settings,
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

    pub(in crate::app) fn flush_settings(&self) {
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
