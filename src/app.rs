mod auto_prune;
mod config_manager;
mod history;
mod loader;
mod modals;

use std::{cell::RefCell, env, path::PathBuf, rc::Rc, str, sync::Arc};

use config_manager::{AppSnapshot, ConfigManager};
use egui_tiles::{
    Container, Linear, LinearDir, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse,
};
use gt_filter::GlobalFilter;
use gt_map::{MapContextAction, MapLayer, NavMap};
use gt_plot::PlotState;
use gt_side_panel::{FilterPanelState, PanelContext, TreeState, show_side_panel};
use gt_track_builder::SegmentationConfig;
use gt_types::{AssociationConfig, DataCategory, FileIdx, LoadWarning, LoadedFile, NavPoint};
use gt_ui_types::{HighlightScope, MapHighlight, TrackDataVisibility};
use loader::{CompletedLoad, FinishedJob, LoadOutcome, LoaderManager};

use modals::{
    show_delete_confirmation, show_load_warnings_dialog, show_mapbox_token_dialog,
    show_orphaned_event_markers_popup, show_unassociated_popup,
};

/// Pane variants for the central area tiles tree.
enum MainPane {
    Map,
    Plot,
}

struct SharedAppState {
    loaded_files: Vec<LoadedFile>,
    tree: TreeState,
    highlight: MapHighlight,
    filter: GlobalFilter,
    filter_state: FilterPanelState,
    plot_state: PlotState,
    map_center_request: Option<(f64, f64)>,
    /// Requested screen position for the next sticky info popup, set by panel
    /// item clicks and consumed by `NavMap::draw` as the popup's default position.
    popup_pos_request: Option<egui::Pos2>,
    /// When `true`, `NavMap::draw` zooms the map to fit all currently visible data.
    zoom_to_visible_request: bool,
    /// Filename and warnings for the currently open data quality warnings dialog, if any.
    warnings_popup: Option<(String, Vec<LoadWarning>)>,
}

pub struct App {
    map: NavMap,
    shared: Rc<RefCell<SharedAppState>>,
    load_error: Option<String>,
    unassociated_log_lines: Option<Vec<(chrono::DateTime<chrono::Utc>, String)>>,
    orphaned_event_markers: Option<Vec<(chrono::DateTime<chrono::Utc>, String)>>,
    mapbox_token_input: String,

    /// Egui context - cloned into background threads for `request_repaint`.
    ctx: egui::Context,
    /// Manages background load threads and the file-picker dialog thread.
    loader: LoaderManager,

    /// Tiles tree for the central area - map (top) and plot (bottom).
    tiles_tree: Tree<MainPane>,
    /// TileId of the map pane - used to read/write the split ratio.
    map_tile_id: TileId,
    /// TileId of the plot pane - toggled visible/invisible via the menu button.
    plot_tile_id: TileId,

    /// Detects settings changes and drives debounced write-through to disk.
    config: ConfigManager,
    /// Path to config file - if None, settings are not loaded from or saved to disk.
    config_path: Option<PathBuf>,

    /// Whether the Settings window is currently open.
    settings_open: bool,
    /// Active segmentation config - applied to all new file loads and re-segmentation.
    processing_config: SegmentationConfig,
    /// Active association config - applied to all new log loads.
    assoc_config: AssociationConfig,

    /// History database - `None` if the database could not be opened at startup.
    db: Option<gt_history::Database>,

    /// When `false`, GTD files are not stored in the history database on load.
    storage_enabled: bool,
    /// When `true`, the oldest recordings are pruned after each import if the
    /// total stored size exceeds `auto_prune_max_bytes`.
    auto_prune_enabled: bool,
    /// Maximum total stored size (bytes) before auto-pruning triggers.
    auto_prune_max_bytes: u64,
    /// When `true`, show a confirmation dialog before auto-pruning.
    auto_prune_confirm: bool,
    /// Recordings selected for auto-pruning, waiting for the user to confirm.
    pending_auto_prune: Option<Vec<gt_history::DatabaseRef>>,

    /// History window state.
    history_window: history::HistoryWindow,

    /// Toast notification queue - rendered every frame over the top of all content.
    toasts: egui_notify::Toasts,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_files(cc, &[])
    }

    pub fn new_with_files(cc: &eframe::CreationContext<'_>, paths: &[PathBuf]) -> Self {
        let default_path = crate::settings::settings_path();
        Self::new_with_config(cc, paths, default_path)
    }

    pub fn new_with_config(
        cc: &eframe::CreationContext<'_>,
        paths: &[PathBuf],
        config_path: Option<PathBuf>,
    ) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        // Register the SVG image loaders (must come before register_marker_icons).
        egui_extras::install_image_loaders(&cc.egui_ctx);
        // Pre-register the compiled-in SVG marker icons so the texture cache
        // can serve them without any per-frame heap allocation.
        gt_map::register_marker_icons(&cc.egui_ctx);

        let mut loaded_settings = config_path
            .as_ref()
            .map(|p| crate::settings::load_settings_from(p))
            .unwrap_or_default();

        // One-time migration: pick up mapbox_token and map_layer from the old
        // eframe storage when config.toml doesn't exist yet.
        let path_exists = config_path.as_ref().is_some_and(|p| p.exists());
        if !path_exists {
            if let Some(token) = cc.storage.and_then(|s| s.get_string("mapbox_token"))
                && loaded_settings.map.mapbox_token.is_empty()
            {
                loaded_settings.map.mapbox_token = token;
            }
            if cc
                .storage
                .and_then(|s| s.get_string("map_layer"))
                .as_deref()
                == Some("satellite")
            {
                loaded_settings.map.layer = crate::settings::MapLayerSetting::Satellite;
            }
        }

        // Environment variables override the config file for the token.
        if let Ok(token) = env::var("MAPBOX_TOKEN").or_else(|_| env::var("MAPBOX_ACCESS_TOKEN")) {
            loaded_settings.map.mapbox_token = token;
        }

        let map = NavMap::new(cc.egui_ctx.clone());
        let loader = LoaderManager::new(cc.egui_ctx.clone());

        // Build the central-area tiles tree: map on top, plot on bottom.
        // The split ratio and panel visibility are applied from settings below.
        let mut tiles: Tiles<MainPane> = Tiles::default();
        let map_tile_id = tiles.insert_pane(MainPane::Map);
        let plot_tile_id = tiles.insert_pane(MainPane::Plot);
        let root_tile_id = tiles.insert_new(Tile::Container(Container::Linear(
            Linear::new_binary(LinearDir::Vertical, [map_tile_id, plot_tile_id], 0.6),
        )));
        let tiles_tree = Tree::new("main_tiles", root_tile_id, tiles);

        // Tests must not touch the production database.  `loader.db_path` starts
        // `None` and is populated by `sync_db_path` (called from
        // `apply_startup_settings`) only in non-test builds.
        #[cfg(not(test))]
        let db = match gt_history::default_path().and_then(|p| {
            use gt_history::HistoryDatabase;
            gt_history::Database::open_or_create(&p)
        }) {
            Ok(db) => Some(db),
            Err(e) => {
                log::error!("Failed to open history database: {e}");
                None
            }
        };
        #[cfg(test)]
        let db: Option<gt_history::Database> = None;

        let mut app = Self {
            map,
            shared: Rc::new(RefCell::new(SharedAppState {
                loaded_files: Vec::new(),
                tree: TreeState::new(),
                highlight: MapHighlight::default(),
                filter: GlobalFilter::default(),
                filter_state: FilterPanelState::default(),
                plot_state: PlotState::default(),
                map_center_request: None,
                popup_pos_request: None,
                zoom_to_visible_request: false,
                warnings_popup: None,
            })),
            load_error: None,
            unassociated_log_lines: None,
            orphaned_event_markers: None,
            mapbox_token_input: String::new(),
            ctx: cc.egui_ctx.clone(),
            loader,
            tiles_tree,
            map_tile_id,
            plot_tile_id,
            config: ConfigManager::new(AppSnapshot::default()),
            config_path,
            settings_open: false,
            processing_config: SegmentationConfig::default(),
            assoc_config: AssociationConfig::default(),
            db,
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 10 * 1024 * 1024 * 1024,
            auto_prune_confirm: true,
            pending_auto_prune: None,
            history_window: history::HistoryWindow::new(),
            toasts: egui_notify::Toasts::default(),
        };

        app.apply_startup_settings(&loaded_settings);
        let initial_snapshot = app.collect_snapshot();
        app.config = ConfigManager::new(initial_snapshot);

        for path in paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "gtd" {
                app.loader
                    .spawn_gtd_path(path.clone(), app.processing_config);
            } else {
                let nav_points = app.snapshot_nav_points();
                app.loader
                    .spawn_log_path(path.clone(), nav_points, app.assoc_config);
            }
        }

        app
    }

    /// Collect a snapshot of all currently loaded GPS points - used by log-file
    /// loaders so they can associate log timestamps with the existing GPS track.
    fn snapshot_nav_points(&self) -> Vec<NavPoint> {
        let s = self.shared.borrow();
        s.loaded_files
            .iter()
            .flat_map(|f| f.tracks.iter())
            .flat_map(|t| t.points.iter())
            .cloned()
            .collect()
    }

    /// Dispatch a file path to the appropriate loader based on its extension.
    fn spawn_load_path(&mut self, path: PathBuf) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "gtd" {
            self.loader.spawn_gtd_path(path, self.processing_config);
        } else {
            let nav_points = self.snapshot_nav_points();
            self.loader
                .spawn_log_path(path, nav_points, self.assoc_config);
        }
    }

    /// Returns `true` when the plot tile is currently visible.
    fn plot_is_visible(&self) -> bool {
        self.tiles_tree.tiles.is_visible(self.plot_tile_id)
    }

    /// Returns the current map/plot split ratio (fraction for the map tile, 0.0–1.0).
    fn get_split_ratio(&self) -> f32 {
        let Some(root_id) = self.tiles_tree.root else {
            return 0.6;
        };
        let Some(Tile::Container(Container::Linear(linear))) = self.tiles_tree.tiles.get(root_id)
        else {
            return 0.6;
        };
        let map_share = linear
            .shares
            .iter()
            .find(|(id, _)| *id == &self.map_tile_id)
            .map_or(0.6, |(_, s)| *s);
        let total: f32 = linear.shares.iter().map(|(_, s)| s).sum();
        if total > 0.0 { map_share / total } else { 0.6 }
    }

    fn set_split_ratio(&mut self, ratio: f32) {
        let Some(root_id) = self.tiles_tree.root else {
            return;
        };
        let Some(Tile::Container(Container::Linear(linear))) =
            self.tiles_tree.tiles.get_mut(root_id)
        else {
            return;
        };
        linear.shares.set_share(self.map_tile_id, ratio);
        linear.shares.set_share(self.plot_tile_id, 1.0 - ratio);
    }

    /// Render the Settings window.
    ///
    /// Returns `true` in the frame when the user clicks "Apply to loaded data",
    /// signalling that the caller should call `apply_resegmentation`.
    fn show_settings_window(&mut self, ui: &egui::Ui) -> bool {
        if !self.settings_open {
            return false;
        }
        let mut open = self.settings_open;
        let mut apply = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(360.0)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui_phosphor::regular::SLIDERS_HORIZONTAL);
                    ui.strong("Processing");
                });
                ui.separator();
                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(format!(
                            "{} Track split gap",
                            egui_phosphor::regular::SCISSORS
                        ))
                        .on_hover_text(
                            "Consecutive GPS points separated by more than this gap \
                             start a new track. For example, with a gap of 5 min, two \
                             fixes at 10:00 and 10:06 would be split into separate tracks.",
                        );
                        let mut gap_secs = self
                            .processing_config
                            .track_split_gap
                            .to_std()
                            .map_or(300, |d| d.as_secs());
                        ui.horizontal(|ui| {
                            compound_duration_input(ui, &mut gap_secs, 30, 7 * 86400, true, true);
                        });
                        self.processing_config.track_split_gap =
                            chrono::Duration::seconds(gap_secs as i64);
                        ui.end_row();

                        ui.label(format!(
                            "{} Log marker window",
                            egui_phosphor::regular::ARROWS_IN_LINE_HORIZONTAL
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

                        let clock_discontinuities_help =
                            "Flag abrupt jumps in the GPS/system clock offset - e.g. a device \
                             resuming from suspend, where a stale GPS timestamp meets a fresh \
                             system timestamp - as markers. These are surfaced for inspection; \
                             the underlying data is never altered.";
                        ui.label(format!(
                            "{} Clock discontinuities",
                            egui_phosphor::regular::WARNING
                        ))
                        .on_hover_text(clock_discontinuities_help);
                        // Detection toggle and its jump sensitivity share one row.
                        // The sensitivity stays visible but grays out when
                        // detection is off rather than hiding (DESIGN.md: keep
                        // the layout stable and the feature discoverable).
                        ui.horizontal(|ui| {
                            ui.checkbox(
                                &mut self.processing_config.detect_clock_discontinuities,
                                "",
                            )
                            .on_hover_text(clock_discontinuities_help);
                            let detect_on = self.processing_config.detect_clock_discontinuities;
                            let sigmas = self.processing_config.clock_discontinuity_sigmas;
                            let floor_s =
                                gt_track_builder::clock_discontinuity_floor_seconds(sigmas);
                            let sensitivity = ui.add_enabled(
                                detect_on,
                                egui::DragValue::new(
                                    &mut self.processing_config.clock_discontinuity_sigmas,
                                )
                                .range(1.0..=20.0)
                                .speed(0.1)
                                .fixed_decimals(1)
                                .suffix(" σ"),
                            );
                            if detect_on {
                                sensitivity.on_hover_text(format!(
                                    "Jump sensitivity: how far a jump must stand out from the \
                                     track's normal clock variation to be flagged, in robust \
                                     standard deviations. Lower flags more (smaller) jumps; \
                                     higher flags only the most extreme.\n\n\
                                     For example, on a steady recording {sigmas:.1} σ flags \
                                     jumps larger than about {floor_s:.1} s; on a noisier one \
                                     the bar rises with the track's own variation.",
                                ));
                            } else {
                                sensitivity.on_hover_text(
                                    "Enable clock discontinuities to adjust the jump sensitivity",
                                );
                            }
                        });
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let apply_label =
                        format!("{} Apply to loaded data", egui_phosphor::regular::CHECK);
                    if ui.button(apply_label).clicked() {
                        apply = true;
                    }
                    let reset_label = format!(
                        "{} Restore Defaults",
                        egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE
                    );
                    if ui.button(reset_label).clicked() {
                        let defaults = crate::settings::ProcessingSettings::default();
                        self.processing_config.track_split_gap =
                            chrono::Duration::seconds(defaults.track_split_gap_seconds as i64);
                        self.assoc_config.log_marker_window_s = defaults.log_marker_window_s;
                        self.processing_config.detect_clock_discontinuities =
                            defaults.detect_clock_discontinuities;
                        self.processing_config.clock_discontinuity_sigmas =
                            defaults.clock_discontinuity_sigmas;
                    }
                });
            });

        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }

        self.settings_open = open;
        apply
    }

    /// Apply loaded settings on startup.
    fn apply_startup_settings(&mut self, s: &crate::settings::Settings) {
        if !s.map.mapbox_token.is_empty() {
            self.map.set_mapbox_token(s.map.mapbox_token.clone());
        }
        self.map.set_layer(map_layer_from_setting(s.map.layer));
        self.processing_config = SegmentationConfig {
            track_split_gap: chrono::Duration::seconds(s.processing.track_split_gap_seconds as i64),
            detect_clock_discontinuities: s.processing.detect_clock_discontinuities,
            clock_discontinuity_sigmas: s.processing.clock_discontinuity_sigmas,
        };
        self.assoc_config = AssociationConfig {
            log_marker_window_s: s.processing.log_marker_window_s,
        };
        self.ctx.set_theme(theme_pref_from_setting(s.ui.theme));

        {
            let mut shared = self.shared.borrow_mut();
            shared.plot_state.sync_to_map = s.map.sync_to_map;
            shared.plot_state.show_grid = s.plot.show_grid;
            let vis = &mut shared.plot_state.metric_vis;
            let get_metric = |k| s.plot.metric.get(&k).copied().unwrap_or(true);
            vis.sats_seen = get_metric(crate::settings::MetricKind::SatsSeen);
            vis.sats_fix = get_metric(crate::settings::MetricKind::SatsFix);
            vis.gps_seen = get_metric(crate::settings::MetricKind::GpsSeen);
            vis.gps_fix = get_metric(crate::settings::MetricKind::GpsFix);
            vis.glonass_seen = get_metric(crate::settings::MetricKind::GlonassSeen);
            vis.glonass_fix = get_metric(crate::settings::MetricKind::GlonassFix);
            vis.galileo_seen = get_metric(crate::settings::MetricKind::GalileoSeen);
            vis.galileo_fix = get_metric(crate::settings::MetricKind::GalileoFix);
            vis.beidou_seen = get_metric(crate::settings::MetricKind::BeidouSeen);
            vis.beidou_fix = get_metric(crate::settings::MetricKind::BeidouFix);
            vis.velocity = get_metric(crate::settings::MetricKind::Velocity);
            vis.eph = get_metric(crate::settings::MetricKind::Eph);
            vis.heading_deg = get_metric(crate::settings::MetricKind::HeadingDeg);
            vis.clock_delta_ms = get_metric(crate::settings::MetricKind::ClockDeltaMs);
        }

        self.tiles_tree
            .tiles
            .set_visible(self.plot_tile_id, s.plot.panel_visible);
        self.set_split_ratio(s.plot.split_ratio);

        self.storage_enabled = s.storage.enabled;
        self.auto_prune_enabled = s.storage.auto_prune_enabled;
        self.auto_prune_max_bytes = s.storage.auto_prune_max_bytes;
        self.auto_prune_confirm = s.storage.auto_prune_confirm;
        self.sync_db_path();
    }

    /// Snapshot of all settings-relevant state for change detection.
    fn collect_snapshot(&self) -> AppSnapshot {
        let s = self.shared.borrow();
        let theme = self
            .ctx
            .options(|o| theme_pref_to_setting(o.theme_preference));
        let vis = &s.plot_state.metric_vis;
        AppSnapshot {
            show_grid: s.plot_state.show_grid,
            panel_visible: self.tiles_tree.tiles.is_visible(self.plot_tile_id),
            split_ratio: self.get_split_ratio().into(),
            metric_sats_seen: vis.sats_seen,
            metric_sats_fix: vis.sats_fix,
            metric_gps_seen: vis.gps_seen,
            metric_gps_fix: vis.gps_fix,
            metric_glonass_seen: vis.glonass_seen,
            metric_glonass_fix: vis.glonass_fix,
            metric_galileo_seen: vis.galileo_seen,
            metric_galileo_fix: vis.galileo_fix,
            metric_beidou_seen: vis.beidou_seen,
            metric_beidou_fix: vis.beidou_fix,
            metric_velocity: vis.velocity,
            metric_eph: vis.eph,
            metric_heading_deg: vis.heading_deg,
            metric_clock_delta_ms: vis.clock_delta_ms,
            layer: map_layer_to_setting(self.map.layer()),
            mapbox_token: self.map.mapbox_token().to_owned(),
            sync_to_map: s.plot_state.sync_to_map,
            theme,
            track_split_gap_seconds: self
                .processing_config
                .track_split_gap
                .to_std()
                .map_or(300, |d| d.as_secs()),
            log_marker_window_s: self.assoc_config.log_marker_window_s,
            detect_clock_discontinuities: self.processing_config.detect_clock_discontinuities,
            clock_discontinuity_sigmas: self.processing_config.clock_discontinuity_sigmas.into(),
            storage_enabled: self.storage_enabled,
            auto_prune_enabled: self.auto_prune_enabled,
            auto_prune_max_bytes: self.auto_prune_max_bytes,
            auto_prune_confirm: self.auto_prune_confirm,
        }
    }

    fn collect_settings_for_flush(&self) -> crate::settings::Settings {
        use crate::settings::MetricKind as K;
        let s = self.shared.borrow();
        let vis = &s.plot_state.metric_vis;
        let metric = std::collections::HashMap::from([
            (K::SatsSeen, vis.sats_seen),
            (K::SatsFix, vis.sats_fix),
            (K::GpsSeen, vis.gps_seen),
            (K::GpsFix, vis.gps_fix),
            (K::GlonassSeen, vis.glonass_seen),
            (K::GlonassFix, vis.glonass_fix),
            (K::GalileoSeen, vis.galileo_seen),
            (K::GalileoFix, vis.galileo_fix),
            (K::BeidouSeen, vis.beidou_seen),
            (K::BeidouFix, vis.beidou_fix),
            (K::Velocity, vis.velocity),
            (K::Eph, vis.eph),
            (K::HeadingDeg, vis.heading_deg),
            (K::ClockDeltaMs, vis.clock_delta_ms),
        ]);
        let theme = self
            .ctx
            .options(|o| theme_pref_to_setting(o.theme_preference));
        crate::settings::Settings {
            version: 1,
            plot: crate::settings::PlotSettings {
                show_grid: s.plot_state.show_grid,
                panel_visible: self.tiles_tree.tiles.is_visible(self.plot_tile_id),
                split_ratio: self.get_split_ratio(),
                metric,
            },
            map: crate::settings::MapSettings {
                layer: map_layer_to_setting(self.map.layer()),
                mapbox_token: self.map.mapbox_token().to_owned(),
                sync_to_map: s.plot_state.sync_to_map,
            },
            ui: crate::settings::UiSettings { theme },
            processing: crate::settings::ProcessingSettings {
                track_split_gap_seconds: self
                    .processing_config
                    .track_split_gap
                    .to_std()
                    .map_or(300, |d| d.as_secs()),
                log_marker_window_s: self.assoc_config.log_marker_window_s,
                detect_clock_discontinuities: self.processing_config.detect_clock_discontinuities,
                clock_discontinuity_sigmas: self.processing_config.clock_discontinuity_sigmas,
            },
            storage: crate::settings::StorageSettings {
                enabled: self.storage_enabled,
                auto_prune_enabled: self.auto_prune_enabled,
                auto_prune_max_bytes: self.auto_prune_max_bytes,
                auto_prune_confirm: self.auto_prune_confirm,
            },
        }
    }

    fn flush_settings(&self) {
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

    /// Re-segment all loaded GPS files using the current `processing_config`.
    ///
    /// Log-only files (those with no nav points) are left unchanged since they
    /// don't have track structure to re-segment.
    fn apply_resegmentation(&mut self) {
        let config = self.processing_config;
        {
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            for file in &mut s.loaded_files {
                let has_nav_points = file.tracks.iter().any(|t| !t.points.is_empty());
                if !has_nav_points {
                    continue;
                }
                let all_points: Vec<gt_types::NavPoint> = file
                    .tracks
                    .iter()
                    .flat_map(|t| t.points.iter())
                    .cloned()
                    .collect();
                let all_custom_markers: Vec<gt_types::CustomMarker> = file
                    .tracks
                    .iter()
                    .flat_map(|t| t.custom_markers.iter())
                    .cloned()
                    .collect();
                let all_event_markers: Vec<gt_types::EventMarker> = file
                    .tracks
                    .iter()
                    .flat_map(|t| t.event_markers.iter())
                    .chain(file.orphaned_event_markers.iter())
                    .cloned()
                    .collect();
                let event_marker_styles: Vec<gt_types::EventMarkerStyle> =
                    file.event_marker_styles.values().cloned().collect();
                let filename = file.metadata.filename.clone();
                let identity = file.identity.clone();
                let source = file.source.clone();
                *file = gt_track_builder::build_loaded_file(
                    filename,
                    identity,
                    &all_points,
                    &all_custom_markers,
                    all_event_markers,
                    event_marker_styles,
                    &config,
                    source,
                    file.load_warnings.clone(),
                );
            }
            s.plot_state.rebuild_all(&s.loaded_files);
            s.tree.reset_for_files(&s.loaded_files);
        }
        let s = self.shared.borrow();
        self.map.rebuild_spatial_index(&s.loaded_files);
    }

    /// Process a completed background load: integrate the result into shared state.
    fn handle_completed_load(&mut self, completed: CompletedLoad) {
        match completed.outcome {
            Ok(LoadOutcome::GtdFile {
                mut file,
                series,
                db_ref,
            }) => {
                let was_stored = db_ref.is_some();
                file.db_ref = db_ref;
                let orphans: Vec<(chrono::DateTime<chrono::Utc>, String)> = file
                    .orphaned_event_markers
                    .iter()
                    .map(|m| (m.time, m.variant_path.clone()))
                    .collect();
                let mut s = self.shared.borrow_mut();
                let fi = s.loaded_files.len();
                s.loaded_files.push(file);
                let files = std::mem::take(&mut s.loaded_files);
                s.tree.sync_from_loaded_files(&files);
                s.loaded_files = files;
                s.plot_state.integrate_file(fi, series);
                drop(s);
                if !orphans.is_empty() {
                    self.orphaned_event_markers = Some(orphans);
                }
                self.load_error = None;
                self.loader.finishing_jobs.push(FinishedJob {
                    filename: completed.filename,
                    elapsed_secs: completed.elapsed_secs,
                    completed_at: std::time::Instant::now(),
                });
                if was_stored {
                    self.check_auto_prune();
                }
            }
            Ok(LoadOutcome::LogFile {
                loaded,
                series,
                unassociated,
            }) => {
                if let (Some(loaded), Some(series)) = (loaded, series) {
                    let mut s = self.shared.borrow_mut();
                    let fi = s.loaded_files.len();
                    s.loaded_files.push(loaded);
                    let files = std::mem::take(&mut s.loaded_files);
                    s.tree.sync_from_loaded_files(&files);
                    s.loaded_files = files;
                    s.plot_state.integrate_file(fi, series);
                }
                if !unassociated.is_empty() {
                    self.unassociated_log_lines = Some(unassociated);
                }
                self.load_error = None;
                self.loader.finishing_jobs.push(FinishedJob {
                    filename: completed.filename,
                    elapsed_secs: completed.elapsed_secs,
                    completed_at: std::time::Instant::now(),
                });
            }
            Err(e) => {
                log::error!("Background load failed: {e}");
                self.load_error = Some(e);
            }
        }
    }

    fn sync_db_path(&mut self) {
        use gt_history::HistoryDatabase;
        self.loader.db_path = if self.storage_enabled {
            self.db.as_ref().map(|db| db.path().to_owned())
        } else {
            None
        };
    }

    /// Check whether auto-pruning is needed and either enqueue a confirmation
    /// or prune silently.  Called after each successful GTD insert.
    fn check_auto_prune(&mut self) {
        if !self.auto_prune_enabled {
            return;
        }
        let Some(db) = self.db.as_mut() else { return };
        match auto_prune::run(db, self.auto_prune_max_bytes, self.auto_prune_confirm) {
            Ok(auto_prune::AutoPruneOutcome::NotNeeded) => {}
            Ok(auto_prune::AutoPruneOutcome::PrunedSilently(n)) => {
                self.history_window.invalidate();
                self.toasts
                    .info(format!(
                        "Auto-pruned {n} {}",
                        if n == 1 { "recording" } else { "recordings" }
                    ))
                    .duration(Some(std::time::Duration::from_secs(4)));
            }
            Ok(auto_prune::AutoPruneOutcome::NeedsConfirmation(candidates)) => {
                self.pending_auto_prune = Some(candidates);
            }
            Err(e) => log::error!("Auto-prune failed: {e}"),
        }
    }

    fn handle_history_action(&mut self, action: history::HistoryAction, ctx: &egui::Context) {
        use gt_history::HistoryDatabase;
        let Some(db) = self.db.as_mut() else {
            return;
        };

        match action {
            history::HistoryAction::Open(db_ref) => match db.load_bytes(&db_ref) {
                Ok(bytes) => {
                    let filename = format!("{}/{}", db_ref.identity, db_ref.group_name);
                    self.loader
                        .spawn_gtd_bytes(bytes.into(), filename, self.processing_config);
                    ctx.request_repaint();
                }
                Err(e) => {
                    log::error!("Failed to load recording from history: {e}");
                    self.toasts.error(format!("Could not open recording: {e}"));
                }
            },
            history::HistoryAction::Delete(db_ref) => {
                let label = format!("{}/{}", db_ref.identity, db_ref.group_name);
                match db.delete(&db_ref) {
                    Ok(()) => {
                        log::info!("Deleted recording '{label}' from history");
                        self.history_window.invalidate();
                    }
                    Err(e) => {
                        log::error!("Failed to delete recording '{label}': {e}");
                        self.toasts
                            .error(format!("Could not delete recording: {e}"));
                    }
                }
            }
            history::HistoryAction::Prune(refs) => {
                let count = refs.len();
                match db.delete_batch(&refs) {
                    Ok(()) => {
                        self.toasts.info(format!(
                            "Pruned {count} {} from history",
                            if count == 1 {
                                "recording"
                            } else {
                                "recordings"
                            }
                        ));
                        self.history_window.invalidate();
                    }
                    Err(e) => {
                        log::error!("Batch prune failed: {e}");
                        self.toasts.error(format!("Prune failed: {e}"));
                    }
                }
            }
        }
    }
}

/// Behavior implementation that renders each pane of the central tiles tree.
struct MainBehavior<'a> {
    map: &'a mut NavMap,
    state: &'a mut SharedAppState,
    plot_hover_scope: Option<HighlightScope>,
    map_hover_time: Option<chrono::DateTime<chrono::Utc>>,
    toggle_plot_request: bool,
}

impl egui_tiles::Behavior<MainPane> for MainBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut MainPane) -> UiResponse {
        match pane {
            MainPane::Map => {
                let s = &mut *self.state;
                let center_req = s.map_center_request.take();
                let popup_pos = s.popup_pos_request.take();
                let zoom_to_visible = std::mem::replace(&mut s.zoom_to_visible_request, false);
                if let Some(action) = self.map.draw(
                    ui,
                    &s.loaded_files,
                    s.tree.visibility(),
                    &mut s.highlight,
                    &s.filter,
                    s.tree.event_marker_visibility(),
                    center_req,
                    zoom_to_visible,
                    popup_pos,
                ) {
                    match action {
                        MapContextAction::ShowOnlyTrack(track) => {
                            s.tree.show_only_track(track);
                        }
                        MapContextAction::ShowOnlyFile(fi) => {
                            s.tree.show_only_file(fi);
                        }
                    }
                }
            }
            MainPane::Plot => {
                egui::Panel::top("plot_header").show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(egui_phosphor::regular::CARET_DOWN)
                                .on_hover_text("Hide plot")
                                .clicked()
                            {
                                self.toggle_plot_request = true;
                            }
                        });
                    });
                });
                let s = &mut *self.state;
                let map_sync_x_range = if s.plot_state.sync_to_map {
                    self.map.viewport_geo_bounds().and_then(|b| {
                        tpv_time_range_in_bounds(&s.loaded_files, s.tree.visibility(), b)
                    })
                } else {
                    None
                };
                gt_plot::show_track_plot(
                    ui,
                    &s.loaded_files,
                    s.tree.visibility(),
                    &s.filter,
                    self.plot_hover_scope,
                    self.map_hover_time,
                    map_sync_x_range,
                    &mut s.plot_state,
                );
            }
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &MainPane) -> egui::WidgetText {
        match pane {
            MainPane::Map => "Map".into(),
            MainPane::Plot => "Plot".into(),
        }
    }

    fn simplification_options(&self) -> SimplificationOptions {
        // Do not auto-prune single-child or empty containers - this keeps the
        // root Linear alive when the plot is hidden so the plot tile can be
        // re-added to children without rebuilding the whole tree.
        SimplificationOptions {
            prune_single_child_containers: false,
            prune_empty_containers: false,
            ..Default::default()
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.flush_settings();
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "egui immediate-mode UI rendering is inherently sequential; splitting artificially hurts readability"
    )]
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Drain background load results first so newly loaded data is
        // visible in the same frame that it arrives.
        let completed_loads: Vec<CompletedLoad> = self.loader.drain();
        for completed in completed_loads {
            self.handle_completed_load(completed);
        }

        // Consume a pending file-picker result and dispatch the chosen path.
        if let Some(path) = self.loader.drain_file_dialog() {
            self.spawn_load_path(path);
        }

        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = &file.path {
                self.spawn_load_path(path.clone());
            } else if let Some(bytes) = file.bytes.clone() {
                self.handle_dropped_bytes(bytes, &file.name);
            }
        }

        {
            let mut s = self.shared.borrow_mut();
            let delete_pressed = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete));
            if delete_pressed && !s.tree.selection.is_empty() && s.tree.delete_confirm.is_none() {
                let items = s.tree.selection.iter().cloned().collect();
                s.tree.delete_confirm = Some(gt_side_panel::DeleteConfirmState { items });
            }
        }

        egui::Panel::top("top_panel").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                // Left zone - file actions
                if ui.button("Open…").clicked() {
                    self.loader.open_file_dialog();
                }

                // Right zone - utility windows and preferences, trailing-aligned
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::widgets::global_theme_preference_buttons(ui);

                    ui.separator();

                    if ui
                        .selectable_label(self.settings_open, egui_phosphor::regular::GEAR)
                        .on_hover_text("Settings")
                        .clicked()
                    {
                        self.settings_open = !self.settings_open;
                    }

                    if ui
                        .selectable_label(
                            self.history_window.open,
                            egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE,
                        )
                        .on_hover_text("Browse and re-open previously recorded sessions")
                        .clicked()
                    {
                        self.history_window.open = !self.history_window.open;
                        self.history_window.invalidate();
                    }
                });
            });
        });

        {
            // Forward the previous frame's plot-legend hover so NavMap::draw's
            // track layers highlight the matching file on the map this frame.
            // NavMap overwrites `highlight.hover` with its own pointer-hover at
            // the end of draw(), which is fine: the plot re-derives its line
            // highlight from `legend_hover_file` directly, so this write only
            // needs to survive until the map has rendered.
            let mut s = self.shared.borrow_mut();
            if let Some(fi) = s.plot_state.legend_hover_file {
                s.highlight.hover = Some(HighlightScope::File {
                    file_index: FileIdx::new(fi),
                });
            }
        }

        let detached = self.shared.borrow().tree.detached;
        if !detached {
            egui::Panel::left("track_data_panel")
                .min_size(240.0)
                .show_inside(ui, |ui| {
                    let mut refmut = self.shared.borrow_mut();
                    let s = &mut *refmut;
                    show_side_panel(
                        ui,
                        &mut PanelContext {
                            files: &s.loaded_files,
                            tree: &mut s.tree,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            map_center_request: &mut s.map_center_request,
                            popup_pos_request: &mut s.popup_pos_request,
                            zoom_to_visible_request: &mut s.zoom_to_visible_request,
                            warnings_request: &mut s.warnings_popup,
                        },
                    );
                });
        } else {
            // Render the panel as a floating egui Window inside the same OS window
            // as the map. A separate OS viewport caused Wayland compositors to
            // suspend event delivery when the child was minimised or occluded,
            // freezing both windows. The floating-window approach is fully
            // platform-independent.
            let mut is_open = !ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            egui::Window::new("Track data")
                .id(egui::Id::new("detached_panel"))
                .open(&mut is_open)
                .default_pos(egui::pos2(10.0, 30.0))
                .default_width(320.0)
                .min_width(240.0)
                .resizable(true)
                .show(ui.ctx(), |ui| {
                    let mut refmut = self.shared.borrow_mut();
                    let s = &mut *refmut;
                    show_side_panel(
                        ui,
                        &mut PanelContext {
                            files: &s.loaded_files,
                            tree: &mut s.tree,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            map_center_request: &mut s.map_center_request,
                            popup_pos_request: &mut s.popup_pos_request,
                            zoom_to_visible_request: &mut s.zoom_to_visible_request,
                            warnings_request: &mut s.warnings_popup,
                        },
                    );
                });
            if !is_open {
                self.shared.borrow_mut().tree.detached = false;
            }
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let panel_rect = ui.max_rect();
            let mut s = self.shared.borrow_mut();
            let plot_hover_scope = match s.highlight.hover {
                Some(HighlightScope::File { .. })
                | Some(HighlightScope::Track(_))
                | Some(HighlightScope::TrackCategory { .. }) => s.highlight.hover,
                Some(HighlightScope::Point(_)) | None => None,
            };
            let map_hover_time = extract_map_hover_time(&s.loaded_files, &s.highlight);

            // Render the tiles tree (map on top, optional plot on bottom).
            // Borrow tiles_tree and map explicitly so the borrow checker can see
            // they are disjoint from s (which comes from self.shared).
            let toggle_plot_request;
            {
                let map = &mut self.map;
                let tiles_tree = &mut self.tiles_tree;
                let mut behavior = MainBehavior {
                    map,
                    state: &mut s,
                    plot_hover_scope,
                    map_hover_time,
                    toggle_plot_request: false,
                };
                tiles_tree.ui(&mut behavior, ui);
                toggle_plot_request = behavior.toggle_plot_request;
            }
            if toggle_plot_request {
                self.tiles_tree.tiles.toggle_visibility(self.plot_tile_id);
            }

            // Forward plot hover → map highlight (must happen after the tree renders
            // so that show_track_plot has already written the current hovered_time).
            // The pre-computed `plot_hover_point` lets TpvRenderer look up the
            // closest point in O(1) instead of re-scanning all track points.
            let plot_visible = self.plot_is_visible();
            if plot_visible {
                if let Some(t) = s.plot_state.hovered_time {
                    let closest = gt_plot::find_closest_tpv(
                        &s.loaded_files,
                        s.tree.visibility(),
                        &s.filter,
                        t,
                    );
                    s.highlight.plot_hover_time = closest.map(|_| t);
                    s.highlight.plot_hover_point = closest;
                } else {
                    s.highlight.plot_hover_time = None;
                    s.highlight.plot_hover_point = None;
                }
            } else {
                s.plot_state.hovered_time = None;
                s.highlight.plot_hover_time = None;
                s.highlight.plot_hover_point = None;

                let btn_size = egui::vec2(28.0, 22.0);
                let btn_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.min.x + 8.0, panel_rect.max.y - btn_size.y - 8.0),
                    btn_size,
                );
                if ui
                    .put(
                        btn_rect,
                        egui::Button::new(egui_phosphor::regular::CHART_LINE_UP).small(),
                    )
                    .on_hover_text("Show plot")
                    .clicked()
                {
                    self.tiles_tree.tiles.toggle_visibility(self.plot_tile_id);
                }
            }
        });

        let apply_resegment = self.show_settings_window(ui);
        if apply_resegment {
            self.apply_resegmentation();
        }

        if self.map.layer() == MapLayer::Satellite && !self.map.has_mapbox_token() {
            show_mapbox_token_dialog(ui, &mut self.map, &mut self.mapbox_token_input);
        }

        // Loading progress overlay - floats in the bottom-right corner.
        // Shows in-flight jobs with a live elapsed timer, and recently completed
        // jobs that fade out over ~3 seconds so the user can see how long it took.
        let any_finishing = !self.loader.finishing_jobs.is_empty();
        self.loader.expire_finished();

        if !self.loader.loading_jobs.is_empty() || any_finishing {
            // Keep repainting while jobs are active or fading.
            ui.ctx().request_repaint();

            egui::Window::new("##loading_progress")
                .title_bar(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(260.0);

                    for job in &self.loader.loading_jobs {
                        let elapsed = job.started_at.elapsed().as_secs_f32();
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(egui::RichText::new(&job.filename).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{elapsed:.1}s"))
                                            .small()
                                            .weak(),
                                    );
                                },
                            );
                        });
                        ui.add(
                            egui::ProgressBar::new(job.progress)
                                .animate(true)
                                .desired_width(240.0)
                                .text(job.stage),
                        );
                        ui.add_space(2.0);
                    }

                    for job in &self.loader.finishing_jobs {
                        let since = job.completed_at.elapsed().as_secs_f32();
                        // Fully opaque for the first 2 s, then fade to transparent by 3 s.
                        #[expect(
                            clippy::cast_sign_loss,
                            reason = "fade_frac is clamped to [0, 1] before multiplying by 255"
                        )]
                        let alpha = if since < 2.0 {
                            255_u8
                        } else {
                            let fade = 1.0 - ((since - 2.0) / 1.0).min(1.0);
                            (fade * 255.0) as u8
                        };
                        let color = egui::Color32::from_rgba_unmultiplied(140, 210, 140, alpha);
                        let weak_color =
                            egui::Color32::from_rgba_unmultiplied(120, 170, 120, alpha);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(egui_phosphor::regular::CHECK)
                                    .color(color)
                                    .small(),
                            );
                            ui.label(egui::RichText::new(&job.filename).color(color).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{:.1}s", job.elapsed_secs))
                                            .color(weak_color)
                                            .small(),
                                    );
                                },
                            );
                        });
                        ui.add_space(2.0);
                    }
                });
        }

        let mut dismiss = false;
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            egui::warn_if_debug_build(ui);
            if let Some(error) = &self.load_error {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        gt_ui_theme::ERROR_INDICATOR,
                        format!("{} {error}", egui_phosphor::regular::WARNING),
                    );
                    dismiss = ui.small_button(egui_phosphor::regular::X).clicked();
                });
            }
        });
        if dismiss {
            self.load_error = None;
        }

        let delete_happened = {
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            let deleted = show_delete_confirmation(ui, &mut s.tree, &mut s.loaded_files);
            if deleted {
                s.plot_state.rebuild_all(&s.loaded_files);
            }
            deleted
        };
        if delete_happened {
            let s = self.shared.borrow();
            self.map.rebuild_spatial_index(&s.loaded_files);
        }

        show_unassociated_popup(ui, &mut self.unassociated_log_lines);
        show_orphaned_event_markers_popup(ui, &mut self.orphaned_event_markers);
        show_load_warnings_dialog(ui, &mut self.shared.borrow_mut().warnings_popup);

        let prev_storage = self.storage_enabled;
        let loaded_metas: Vec<gt_history::RecordingMeta> = {
            let s = self.shared.borrow();
            s.loaded_files
                .iter()
                .filter_map(|f| f.recording_meta)
                .collect()
        };
        if let Some(action) = self.history_window.show(
            ui.ctx(),
            self.db.as_ref(),
            &loaded_metas,
            &mut self.storage_enabled,
            &mut self.auto_prune_enabled,
            &mut self.auto_prune_max_bytes,
            &mut self.auto_prune_confirm,
        ) {
            self.handle_history_action(action, ui.ctx());
        }
        if self.storage_enabled != prev_storage {
            self.sync_db_path();
        }

        // Auto-prune confirmation dialog.
        if let Some(refs) = &self.pending_auto_prune {
            let max_gb = self.auto_prune_max_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let n = refs.len();
            let mut do_prune = false;
            let mut cancel = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            egui::Window::new("Auto-prune")
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    let rec_label = if n == 1 { "recording" } else { "recordings" };
                    ui.label(format!(
                        "{n} {rec_label} will be deleted to keep storage under {max_gb:.1} GB"
                    ));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for r in refs {
                                ui.label(format!("{}/{}", r.identity, r.group_name));
                            }
                        });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                egui::RichText::new("Delete these recordings")
                                    .color(gt_ui_theme::WARNING_AMBER),
                            )
                            .on_hover_text(
                                "This cannot be undone. The original source files are unaffected.",
                            )
                            .clicked()
                        {
                            do_prune = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if do_prune {
                let candidates = self.pending_auto_prune.take().unwrap_or_default();
                if let Some(db) = self.db.as_mut() {
                    use gt_history::HistoryDatabase;
                    match db.delete_batch(&candidates) {
                        Ok(()) => {
                            self.history_window.invalidate();
                            self.toasts
                                .info(format!(
                                    "Auto-pruned {} {}",
                                    candidates.len(),
                                    if candidates.len() == 1 {
                                        "recording"
                                    } else {
                                        "recordings"
                                    }
                                ))
                                .duration(Some(std::time::Duration::from_secs(4)));
                        }
                        Err(e) => log::error!("Auto-prune failed: {e}"),
                    }
                }
            } else if cancel {
                self.pending_auto_prune = None;
            }
        }

        self.toasts.show(ui.ctx());

        // Detect settings changes and trigger a debounced write-through.
        let snapshot = self.collect_snapshot();
        self.config.sync(snapshot);
        if self.config.take_flush() {
            self.flush_settings();
        }
    }
}

fn handle_dropped_bytes_dispatch(
    loader: &mut LoaderManager,
    load_error: &mut Option<String>,
    nav_points: Vec<NavPoint>,
    bytes: Arc<[u8]>,
    name: &str,
    config: SegmentationConfig,
    assoc_config: AssociationConfig,
) {
    const HDF5_MAGIC: &[u8] = b"\x89HDF\r\n\x1a\n";
    if bytes.starts_with(HDF5_MAGIC) {
        let filename = if name.is_empty() {
            "dropped.gtd".to_owned()
        } else {
            name.to_owned()
        };
        loader.spawn_gtd_bytes(bytes, filename, config);
    } else if let Ok(text) = str::from_utf8(&bytes) {
        let filename = if name.is_empty() { "dropped.log" } else { name };
        loader.spawn_log_text(
            text.to_owned(),
            filename.to_owned(),
            nav_points,
            assoc_config,
        );
    } else {
        *load_error = Some("Unrecognised file format".to_owned());
    }
}

impl App {
    fn handle_dropped_bytes(&mut self, bytes: Arc<[u8]>, name: &str) {
        let nav_points = self.snapshot_nav_points();
        handle_dropped_bytes_dispatch(
            &mut self.loader,
            &mut self.load_error,
            nav_points,
            bytes,
            name,
            self.processing_config,
            self.assoc_config,
        );
    }
}

/// Find the Unix-second time range of TPV points that lie within the given map
/// geographic bounds, considering only files/tracks currently enabled in `visibility`.
///
/// Returns `None` when no visible TPV points fall in the viewport.
fn tpv_time_range_in_bounds(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
    bounds: gt_map::GeoBounds,
) -> Option<(f64, f64)> {
    let mut t_min = f64::INFINITY;
    let mut t_max = f64::NEG_INFINITY;

    for (fi, file) in files.iter().enumerate() {
        let Some(fv) = visibility.files.get(fi) else {
            continue;
        };
        if !fv.enabled {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            let Some(tv) = fv.tracks.get(ti) else {
                continue;
            };
            if !tv.enabled {
                continue;
            }
            for point in &track.points {
                let lat = point.tpv.lat().as_degrees();
                let lon = point.tpv.lon().as_degrees();
                if lat < bounds.lat_min
                    || lat > bounds.lat_max
                    || lon < bounds.lon_min
                    || lon > bounds.lon_max
                {
                    continue;
                }
                let t = point.tpv.time().utc().timestamp() as f64;
                t_min = t_min.min(t);
                t_max = t_max.max(t);
            }
        }
    }

    if t_min.is_finite() && t_max.is_finite() {
        Some((t_min, t_max))
    } else {
        None
    }
}

/// Extract the GPS timestamp of the map-hovered TPV point (if any) so the plot
/// can draw a vertical cursor at the corresponding time.
fn extract_map_hover_time(
    files: &[LoadedFile],
    highlight: &MapHighlight,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let HighlightScope::Point(point_ref) = highlight.hover? else {
        return None;
    };
    if point_ref.category != DataCategory::Tpv {
        return None;
    }
    point_ref
        .track
        .fi
        .get(files)
        .and_then(|f| point_ref.track.index.get(&f.tracks))
        .and_then(|t| point_ref.point_index.get(&t.points))
        .map(|p| p.tpv.time().utc())
}

fn map_layer_to_setting(layer: MapLayer) -> crate::settings::MapLayerSetting {
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

fn theme_pref_to_setting(p: egui::ThemePreference) -> crate::settings::ThemeSetting {
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
/// `show_days` controls whether the days field is included.
/// `show_hours` controls whether the hours field is included.
/// Each component is independent; the total is clamped to `[min_secs, max_secs]`.
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
            .add(egui::DragValue::new(&mut d).range(0..=max_d).suffix("d"))
            .changed();
    }
    if show_hours {
        changed |= ui
            .add(egui::DragValue::new(&mut h).range(0..=23).suffix("h"))
            .changed();
    }
    changed |= ui
        .add(egui::DragValue::new(&mut m).range(0..=max_m).suffix("m"))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut s).range(0..=59).suffix("s"))
        .changed();

    if changed {
        let total = d * 86400 + h * 3600 + m * 60 + s;
        *value_secs = total.clamp(min_secs, max_secs);
    }
}

#[cfg(test)]
#[path = "app/ui_tests.rs"]
mod tests;
