mod auto_prune;
mod backfill;
mod backfill_ui;
mod context_line;
mod day_failures;
mod day_fetch_queue;
mod day_fetch_status;
mod fix_positions;
mod flares;
mod frame;
mod history;
mod history_db;
mod history_open;
mod jamming;
mod loader;
mod log_association;
mod log_viewer;
mod mapbox_token;
mod mapbox_token_test;
mod modals;
mod panes;
mod query;
mod recording_name_template;
mod reference_window;
mod settings_autosave;
mod settings_ui;
mod snap;
mod snap_persist;
mod snap_state;
mod solar;
mod space_weather_warning;
mod storage;
mod storage_controls;
mod tec;
mod tec_mirrors_ui;
pub use storage::Storage;
#[cfg(feature = "self-update")]
pub mod update;

use std::collections::HashMap;
use std::{cell::RefCell, env, path::PathBuf, rc::Rc};

use egui_tiles::{Container, Linear, LinearDir, Tile, TileId, Tiles, Tree};
use gt_fetch::TransportSource;
use gt_filter::GlobalFilter;
use gt_loaded_files::{LoadedFileId, LoadedFiles};
use gt_log_view::{LoadedLog, LoadedLogs};
use gt_map::NavMap;
use gt_plot::PlotState;
use gt_side_panel::{FilterPanelState, TreeState};
use gt_snap::wire::Costing;
use gt_track_builder::SegmentationConfig;
use gt_types::{AssociationConfig, LoadWarning, TrackRef};
use gt_ui_types::{DisplayMask, MapHighlight, SkyGlyphVariant};
use loader::{CompletedLoad, FinishedJob, LoadJobs, LoadOutcome};
use log_viewer::LogViewerRequests;
use log_viewer::association_dialog::LogAssociationDialog;
use panes::MainPane;
use recording_name_template::TemplatePreviewRecording;
use settings_autosave::{AppSnapshot, SettingsAutosaver};
use settings_ui::SettingsPage;
use settings_ui::search::SettingsSearch;
use snap_state::{PendingSnapRequest, SnapErrorDerived, SnapReplacePrompt, SnapScopePrompt};
use strum::IntoEnumIterator;

struct SharedAppState {
    loaded_files: LoadedFiles,
    tree: TreeState,
    highlight: MapHighlight,
    filter: GlobalFilter,
    /// Global per-category visibility of the map ink.
    display_mask: DisplayMask,
    /// Which sky-glyph variant the map overlay draws.
    sky_glyph_variant: SkyGlyphVariant,
    /// Which parts of the clicked-point window are folded away. Mirrors the
    /// persisted map setting, like `sky_glyph_variant`.
    point_window_folds: gt_ui_types::PointWindowFolds,
    filter_state: FilterPanelState,
    plot_state: PlotState,
    /// The log match under the cursor: the map writes the hexagon it is on,
    /// the log viewer the row it is on, and each reads what the other wrote.
    log_hover: gt_ui_types::LogMatchHover,
    map_center_request: Option<(f64, f64)>,
    /// Requested screen position for the next sticky info popup, set by panel
    /// item clicks and consumed by `NavMap::draw` as the popup's default position.
    popup_pos_request: Option<egui::Pos2>,
    /// When `true`, `NavMap::draw` zooms the map to fit all currently visible data.
    zoom_to_visible_request: bool,
    /// Filename and warnings for the currently open data quality warnings dialog, if any.
    warnings_popup: Option<(String, Vec<LoadWarning>)>,
    /// The recording whose metadata-details dialog is open, if any.
    metadata_popup: Option<gt_side_panel::RecordingDetails>,
    /// Set by the side panel's "Reset filters", consumed by the app to also
    /// clear the query filter (the query window is not part of shared state).
    clear_query_request: bool,
    /// Set by the map context menu's "Show sky trails", consumed by the app to
    /// open the sky trails window (which is not part of shared state).
    sky_trails_request: Option<gt_ui_types::SkyTrailsRequest>,
    /// Set by a link in the map warning indicator's popup, consumed by the app
    /// to open the reference window (which is not part of shared state).
    reference_document_request: Option<gt_ui_types::reference::ReferenceDocument>,
    /// User template for the recording name shown in the side panel. See
    /// [`gt_fmt::render_name_template`].
    recording_name_template: String,
}

impl SharedAppState {
    fn sync_tree_from_loaded_files(&mut self) {
        self.tree.sync_from_loaded_files(self.loaded_files.files());
    }
}

/// A recording opened from history whose stored track-splitting settings differ
/// from the app's current ones. The user must choose to recalculate the tracks
/// (current split setting) or use the stored tracks (previous split setting).
struct ResegmentPrompt {
    db_ref: gt_store::DatabaseRef,
    filename: String,
    bytes: std::sync::Arc<[u8]>,
    /// Settings the stored tracks were built with.
    stored: gt_store::StoredSegmentation,
    /// 0-based positions of the recording's hidden tracks, re-applied to the view
    /// when the user keeps the stored tracks.
    hidden_positions: Vec<usize>,
    /// Whether marker-generation settings differ from the stored/default marker
    /// settings and will be rebuilt from the current app settings when opened.
    marker_settings_changed: bool,
}

fn transport_source(offline: bool) -> TransportSource {
    if offline {
        TransportSource::Offline
    } else {
        TransportSource::Network
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StartupOptions {
    pub fading_enabled: bool,
    /// Whether the app runs without network access: no interference
    /// downloads, no snapping, no update check, no map tiles.
    ///
    /// `main` sets it from the `--offline` flag, and is the only place that
    /// reads the command line. Everything downstream is handed the value.
    pub offline: bool,
    /// Which databases the run opens.
    pub storage: storage::Storage,
    /// The running version, shown in the About dialog and used by the update
    /// check. Tests pin it to a fixed string so version-bearing snapshots
    /// survive a release bump.
    pub app_version: &'static str,
}

/// The dense per-component color slots the plot reads, from the settings
/// file's sparse recolored-component entries. Entries past the widest index
/// size the vector. Anything absent stays the derived hue.
fn dense_component_colors(
    entries: &[crate::settings::ComponentColor],
) -> Vec<Option<egui::Color32>> {
    let len = entries.iter().map(|e| e.component + 1).max().unwrap_or(0);
    let mut colors: Vec<Option<egui::Color32>> = vec![None; len];
    for entry in entries {
        if let Some(slot) = colors.get_mut(entry.component) {
            let [r, g, b, a] = entry.rgba;
            *slot = Some(egui::Color32::from_rgba_premultiplied(r, g, b, a));
        }
    }
    colors
}

/// The inverse of [`dense_component_colors`]: only overridden slots are
/// stored (TOML cannot hold `None` array slots).
fn sparse_component_colors(
    colors: &[Option<egui::Color32>],
) -> Vec<crate::settings::ComponentColor> {
    colors
        .iter()
        .enumerate()
        .filter_map(|(component, c)| {
            c.map(|c| crate::settings::ComponentColor {
                component,
                rgba: c.to_array(),
            })
        })
        .collect()
}

/// The fixed version string injected in place of the real crate version in
/// tests, so every version-bearing UI snapshot stays stable across release
/// bumps. The one placeholder for the whole app (the About dialog and the
/// update prompt both flow through it).
#[cfg(test)]
pub(crate) const TEST_APP_VERSION: &str = "0.0.0-test";

pub struct App {
    map: NavMap,
    shared: Rc<RefCell<SharedAppState>>,
    load_error: Option<String>,
    /// The logs loaded in this session. Logs are not recordings: they appear in
    /// neither the file tree nor the plot.
    logs: LoadedLogs,
    orphaned_event_markers: Option<Vec<(chrono::DateTime<chrono::Utc>, String)>>,
    /// The Mapbox token under edit, shared by the map's token dialog and the
    /// settings window's Interface page.
    mapbox_token_field: mapbox_token::MapboxTokenField,
    mapbox_token_test: mapbox_token_test::MapboxTokenTest,

    /// Egui context - cloned into background threads for `request_repaint`.
    ctx: egui::Context,
    /// Manages background load threads and the file-picker dialog thread.
    loader: LoadJobs,
    /// Schedules snap-to-road runs and holds per-track snap activity and the
    /// session result cache.
    snap: snap::SnapScheduler,
    /// Persisted snap-to-road configuration: server URL, default costing, and
    /// the upload-consent acknowledgment.
    snap_settings: crate::settings::SnapSettings,
    /// Whether the snap upload-consent dialog is currently shown. Raised by
    /// the side panel's manual snap trigger while consent is pending, lowered
    /// by the dialog.
    snap_consent_prompt: bool,
    /// Auto mode should sweep for unsnapped tracks: set when files load,
    /// indices shift, or auto mode turns on. Consumed once per frame.
    snap_auto_sweep: bool,
    /// Per-run derived snap error data, keyed by track content and
    /// invalidated by the run's `Arc` identity. Downstream caches (the
    /// plot's mipmaps, the query fingerprint) key off the `Arc`s, so they
    /// must stay stable across frames and change exactly when the run does.
    snap_error_cache: HashMap<snap::TrackContentKey, SnapErrorDerived>,
    /// Session-only per-track costing overrides ("Snap again as…"). The
    /// override beats the declared travel mode and the configured default.
    /// Content-keyed so it survives index shifts like the run stores. Not
    /// persisted: after a restart the stored run goes stale against the
    /// resolved default again.
    snap_costing_overrides: HashMap<snap::TrackContentKey, Costing>,
    /// The snap trigger that raised the consent dialog. Run when the
    /// dialog is accepted, dropped when it is declined.
    pending_snap: PendingSnapRequest,
    /// The costing choice waiting on the replace-cached-run dialog.
    snap_replace_prompt: Option<SnapReplacePrompt>,
    /// The recording-level costing choice waiting on the scope dialog.
    snap_scope_prompt: Option<SnapScopePrompt>,
    /// Tracks whose completed snapped track is toggled off the map. Session
    /// state, like the snap cache. Cleared with the other per-track snap
    /// state when indices shift.
    hidden_snapped: std::collections::HashSet<TrackRef>,

    /// Tiles tree for the central area - map (top) and plot (bottom).
    tiles_tree: Tree<MainPane>,
    /// TileId of the map pane - used to read/write the split ratio.
    map_tile_id: TileId,
    /// TileId of the plot pane - toggled visible/invisible via the menu button.
    plot_tile_id: TileId,

    /// Detects settings changes and drives debounced write-through to disk.
    config: SettingsAutosaver,
    /// Path to config file - if None, settings are not loaded from or saved to disk.
    config_path: Option<PathBuf>,

    settings_open: bool,
    /// Category the settings window shows. Session state: the window opens on
    /// [`SettingsPage::Processing`] every run.
    settings_page: SettingsPage,
    settings_search: SettingsSearch,
    about_open: bool,
    /// The running crate version. Fixed to a placeholder in tests so
    /// version-bearing UI snapshots stay stable across release bumps.
    app_version: &'static str,
    /// Active segmentation config - applied to all new file loads and re-segmentation.
    processing_config: SegmentationConfig,
    /// Active association config - applied to all new log loads.
    assoc_config: AssociationConfig,

    /// Background worker that owns the history database. All reads and edits go
    /// through it so the UI thread never blocks on disk I/O.
    history: history_db::HistoryWorker,
    /// Queues and ingests interference days for loaded tracks.
    jamming: jamming::JammingScheduler,
    /// Where the receiver was over time, for the context lines whose value
    /// depends on position.
    fix_positions: fix_positions::FixPositions,
    /// Queues and ingests geomagnetic index days for loaded tracks.
    geomagnetic_indices: solar::GeomagneticIndexScheduler,
    /// Persisted geomagnetic index configuration: the host serving Kp and
    /// Hp30.
    geomagnetic_index_settings: crate::settings::GeomagneticIndexSettings,
    /// Queues and ingests TEC map days for loaded tracks.
    tec_maps: tec::TecMapScheduler,
    /// Persisted TEC configuration: the host serving the ionosphere maps.
    tec_settings: crate::settings::TecSettings,
    /// Queues and ingests solar flare days for loaded tracks.
    solar_flares: flares::SolarFlareScheduler,
    /// What the archived environment values say about the loaded recordings.
    space_weather_warning: space_weather_warning::SpaceWeatherWarning,
    /// Persisted solar flare configuration: the host serving the catalog and
    /// the user's key for it.
    solar_flare_settings: crate::settings::SolarFlareSettings,
    /// No network access this run. Set once from [`StartupOptions`].
    offline: bool,
    interference_backfill_ui: backfill_ui::BackfillUi<backfill_ui::InterferenceBackfill>,
    geomagnetic_index_backfill_ui: backfill_ui::BackfillUi<backfill_ui::GeomagneticIndexBackfill>,
    tec_map_backfill_ui: backfill_ui::BackfillUi<backfill_ui::TecMapBackfill>,
    solar_flare_backfill_ui: backfill_ui::BackfillUi<backfill_ui::SolarFlareBackfill>,
    interference_settings: crate::settings::InterferenceSettings,
    /// Set when the recordings database could not be opened. Drives the
    /// prompt for whichever failure it was.
    history_failure: Option<storage::HistoryFailure>,
    /// "Keep a backup of the original database" tickbox state for the corruption
    /// recreate dialog.
    keep_db_backup: bool,
    /// Set when a recording is opened from history whose stored segmentation
    /// settings differ from the current ones. Drives the recalculate/use-stored
    /// prompt.
    pending_resegment: Option<ResegmentPrompt>,

    storage_settings: crate::settings::StorageSettings,
    /// Recordings selected for auto-pruning, waiting for the user to confirm.
    pending_auto_prune: Option<Vec<gt_store::DatabaseRef>>,

    /// History window state.
    history_window: history::HistoryWindow,
    query_window: query::QueryWindow,
    /// The log viewer window, opened whenever a log finishes loading.
    log_viewer: log_viewer::LogViewerWindow,
    /// What the log viewer requested while it drew, applied once the shared
    /// state it drew from is no longer borrowed.
    log_viewer_requests: LogViewerRequests,
    /// The association dialog of the log it names, shown until the user
    /// decides.
    association_dialog: Option<LogAssociationDialog>,
    /// Whether a loading log raises the association dialog. Off leaves a log
    /// to associate by itself where exactly one loaded recording overlaps it.
    ask_log_association_target: bool,
    /// The reference material window, opened from the settings page of the
    /// data source it describes.
    reference_window: reference_window::ReferenceWindow,
    /// The whole-track sky trails window.
    sky_trails_window: gt_map::SkyTrailsWindow,

    /// Toast notification queue - rendered every frame over the top of all content.
    toasts: egui_notify::Toasts,

    /// Background startup check for a newer GeoTrace release, plus its prompt.
    /// Only present in dist builds (the `self-update` feature).
    #[cfg(feature = "self-update")]
    update_checker: update::UpdateChecker,
    /// When `true`, check for updates on startup (also gated on a release build
    /// and on not running offline). Mirrors `settings.update.check_on_startup`.
    update_check_on_startup: bool,
    /// A release version the user chose to skip. Suppresses the update prompt for
    /// exactly this version. Mirrors `settings.update.skipped_version`.
    skipped_version: Option<String>,
}

impl App {
    pub fn new_with_files(
        cc: &eframe::CreationContext<'_>,
        paths: &[PathBuf],
        options: StartupOptions,
    ) -> Self {
        let default_path = crate::settings::settings_path();
        Self::new_with_config(cc, paths, default_path, options)
    }

    pub fn new_with_config(
        cc: &eframe::CreationContext<'_>,
        paths: &[PathBuf],
        config_path: Option<PathBuf>,
        options: StartupOptions,
    ) -> Self {
        cc.egui_ctx
            .set_fonts(gt_ui_theme::fonts::font_definitions());

        // GPU-instanced icon rendering. Without a wgpu render state (or with
        // a corrupted embed, which NavMap reports) the map falls back to the
        // CPU mesh path. Dithering on, matching eframe's renderer default.
        if let Some(render_state) = &cc.wgpu_render_state
            && let Ok(library) = gt_map::icon_mesh::IconMeshLibrary::embedded()
        {
            gt_map::icon_mesh::gpu::install(&cc.egui_ctx, render_state, &library, true);
        }

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

        let map = NavMap::new(
            cc.egui_ctx.clone(),
            if options.offline {
                gt_map::TileAccess::Offline
            } else {
                gt_map::TileAccess::Network
            },
        );
        let loader = LoadJobs::new(cc.egui_ctx.clone());
        let snap = snap::SnapScheduler::new(
            cc.egui_ctx.clone(),
            transport_source(options.offline),
            options.offline,
        );

        // Build the central-area tiles tree: map on top, plot on bottom.
        // The split ratio and panel visibility are applied from settings below.
        let mut tiles: Tiles<MainPane> = Tiles::default();
        let map_tile_id = tiles.insert_pane(MainPane::Map);
        let plot_tile_id = tiles.insert_pane(MainPane::Plot);
        let root_tile_id = tiles.insert_new(Tile::Container(Container::Linear(
            Linear::new_binary(LinearDir::Vertical, [map_tile_id, plot_tile_id], 0.6),
        )));
        let tiles_tree = Tree::new("main_tiles", root_tile_id, tiles);

        // `loader.db_path` starts `None` and is populated by `sync_db_path`
        // (called from `apply_startup_settings`) once the history worker has
        // a path.
        let storage::OpenStorage {
            history,
            history_failure,
            archive,
            geomagnetic_indices,
            tec_maps,
            solar_flares,
        } = options.storage.open(&cc.egui_ctx);

        let jamming = jamming::JammingScheduler::new(
            cc.egui_ctx.clone(),
            archive,
            gt_jam::DEFAULT_BASE_URL.to_owned(),
            transport_source(options.offline),
        );
        let geomagnetic_indices = solar::GeomagneticIndexScheduler::new(
            cc.egui_ctx.clone(),
            geomagnetic_indices,
            gt_solar::DEFAULT_BASE_URL.to_owned(),
            transport_source(options.offline),
        );
        let tec_settings = crate::settings::TecSettings::default();
        let tec_maps = tec::TecMapScheduler::new(
            cc.egui_ctx.clone(),
            tec_maps,
            tec_settings.mirrors.clone(),
            tec_settings.earthdata_token(),
            transport_source(options.offline),
        );
        let solar_flare_settings = crate::settings::SolarFlareSettings::default();
        let solar_flares = flares::SolarFlareScheduler::new(
            cc.egui_ctx.clone(),
            solar_flares,
            gt_flare::DEFAULT_BASE_URL.to_owned(),
            solar_flare_settings.api_key(),
            transport_source(options.offline),
        );
        let app_version = options.app_version;

        let mut app = Self {
            jamming,
            fix_positions: fix_positions::FixPositions::default(),
            geomagnetic_indices,
            geomagnetic_index_settings: crate::settings::GeomagneticIndexSettings::default(),
            tec_maps,
            tec_settings,
            solar_flares,
            solar_flare_settings,
            space_weather_warning: space_weather_warning::SpaceWeatherWarning::default(),
            offline: options.offline,
            interference_backfill_ui: backfill_ui::BackfillUi::default(),
            geomagnetic_index_backfill_ui: backfill_ui::BackfillUi::default(),
            tec_map_backfill_ui: backfill_ui::BackfillUi::default(),
            solar_flare_backfill_ui: backfill_ui::BackfillUi::default(),
            interference_settings: crate::settings::InterferenceSettings::default(),
            map,
            shared: Rc::new(RefCell::new(SharedAppState {
                loaded_files: LoadedFiles::new(),
                tree: TreeState::new(),
                highlight: MapHighlight {
                    fading_enabled: options.fading_enabled,
                    ..Default::default()
                },
                filter: GlobalFilter::default(),
                display_mask: DisplayMask::default(),
                sky_glyph_variant: SkyGlyphVariant::default(),
                point_window_folds: gt_ui_types::PointWindowFolds::default(),
                filter_state: FilterPanelState::default(),
                plot_state: PlotState::default(),
                log_hover: gt_ui_types::LogMatchHover::default(),
                map_center_request: None,
                popup_pos_request: None,
                zoom_to_visible_request: false,
                sky_trails_request: None,
                reference_document_request: None,
                warnings_popup: None,
                metadata_popup: None,
                clear_query_request: false,
                recording_name_template: crate::settings::DEFAULT_RECORDING_NAME_TEMPLATE
                    .to_owned(),
            })),
            load_error: None,
            logs: LoadedLogs::default(),
            orphaned_event_markers: None,
            mapbox_token_field: mapbox_token::MapboxTokenField::default(),
            mapbox_token_test: mapbox_token_test::MapboxTokenTest::default(),
            ctx: cc.egui_ctx.clone(),
            loader,
            snap,
            snap_settings: crate::settings::SnapSettings::default(),
            snap_consent_prompt: false,
            snap_auto_sweep: false,
            snap_error_cache: HashMap::new(),
            snap_costing_overrides: HashMap::new(),
            pending_snap: PendingSnapRequest::default(),
            snap_replace_prompt: None,
            snap_scope_prompt: None,
            hidden_snapped: std::collections::HashSet::new(),
            tiles_tree,
            map_tile_id,
            plot_tile_id,
            config: SettingsAutosaver::new(AppSnapshot::default()),
            config_path,
            settings_open: false,
            settings_page: SettingsPage::default(),
            settings_search: SettingsSearch::default(),
            about_open: false,
            app_version,
            processing_config: SegmentationConfig::default(),
            assoc_config: AssociationConfig::default(),
            history,
            history_failure,
            keep_db_backup: true,
            pending_resegment: None,
            storage_settings: crate::settings::StorageSettings::default(),
            pending_auto_prune: None,
            history_window: history::HistoryWindow::new(),
            query_window: query::QueryWindow::new(),
            log_viewer: log_viewer::LogViewerWindow::new(),
            log_viewer_requests: LogViewerRequests::default(),
            association_dialog: None,
            ask_log_association_target: true,
            reference_window: reference_window::ReferenceWindow::new(),
            sky_trails_window: gt_map::SkyTrailsWindow::default(),
            toasts: egui_notify::Toasts::default(),
            #[cfg(feature = "self-update")]
            update_checker: update::UpdateChecker::new(app_version),
            update_check_on_startup: true,
            skipped_version: None,
        };

        app.apply_startup_settings(&loaded_settings);
        let initial_snapshot = app.collect_snapshot();
        app.config = SettingsAutosaver::new(initial_snapshot);

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
                app.loader.spawn_log_path(path.clone());
            }
        }

        app
    }

    fn spawn_load_path(&mut self, path: PathBuf) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "gtd" {
            self.loader.spawn_gtd_path(path, self.processing_config);
        } else {
            self.loader.spawn_log_path(path);
        }
    }

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

    /// The recording the name-template preview renders against: the first loaded
    /// one, else the most recent one in history.
    fn name_template_preview_recording(&self) -> Option<TemplatePreviewRecording> {
        let loaded = self
            .shared
            .borrow()
            .loaded_files
            .view()
            .get(0)
            .map(TemplatePreviewRecording::from_loaded_file);
        loaded.or_else(|| {
            self.history_window
                .latest_listed_recording()
                .map(TemplatePreviewRecording::from_history_entry)
        })
    }

    /// Snapshot of all settings-relevant state for change detection.
    fn collect_snapshot(&self) -> AppSnapshot {
        let s = self.shared.borrow();
        let theme = self
            .ctx
            .options(|o| settings_ui::theme_pref_to_setting(o.theme_preference));
        let vis = &s.plot_state.metric_vis;
        AppSnapshot {
            show_grid: s.plot_state.show_grid,
            line_width: s.plot_state.line_width.into(),
            panel_visible: self.tiles_tree.tiles.is_visible(self.plot_tile_id),
            split_ratio: self.get_split_ratio().into(),
            // `from_fn` invokes the closure in index order 0..COUNT, matching
            // the iterator, so each slot gets the visibility of the metric at
            // that position. The length is COUNT, so `next()` is always `Some`.
            metrics: {
                let mut kinds = crate::settings::MetricKind::iter();
                std::array::from_fn(|_| kinds.next().is_none_or(|k| vis.field(k)))
            },
            show_advanced_metrics: s.plot_state.show_advanced_metrics,
            channels: s.plot_state.channel_vis.entries(),
            show_channels: s.plot_state.show_channels,
            show_solar_flares: s.plot_state.show_solar_flares,
            layer: settings_ui::map_layer_to_setting(self.map.layer()),
            mapbox_token: self.map.mapbox_token().to_owned(),
            sync_to_map: s.plot_state.sync_to_map,
            display_mask: s.display_mask,
            sky_glyph_variant: s.sky_glyph_variant,
            point_window_folds: s.point_window_folds,
            sky_trail_opacity_percent: self.sky_trails_window.trail_opacity_percent().into(),
            tec_heatmap_opacity_percent: self.map.tec_heatmap_opacity_percent().into(),
            theme,
            recording_name_template: s.recording_name_template.clone(),
            track_split_gap_seconds: self
                .processing_config
                .track_layout
                .track_split_gap
                .to_std()
                .map_or(300, |d| d.as_secs()),
            log_association_window_s: self.assoc_config.log_association_window_s,
            ask_log_association_target: self.ask_log_association_target,
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
                .clock_discontinuity_sigmas
                .into(),
            detect_clock_offset_excursions: self
                .processing_config
                .generated_markers
                .detect_clock_offset_excursions,
            detect_slips: self.processing_config.generated_markers.detect_slips,
            elevation_mask_deg: s.plot_state.analysis.elevation_mask_deg.into(),
            mark_masked_fix: s.plot_state.mark_masked_fix,
            snr_drop_db: s.plot_state.analysis.snr_drop_db.into(),
            slip_window_min: s.plot_state.analysis.slip_window_min.into(),
            clock_excursion_threshold_s: s.plot_state.analysis.clock_excursion_threshold_s.into(),
            storage_enabled: self.storage_settings.enabled,
            auto_prune_enabled: self.storage_settings.auto_prune_enabled,
            auto_prune_max_bytes: self.storage_settings.auto_prune_max_bytes,
            auto_prune_confirm: self.storage_settings.auto_prune_confirm,
            update_check_on_startup: self.update_check_on_startup,
            skipped_version: self.skipped_version.clone(),
            query_history_revision: self.query_window.history_revision(),
            snap: self.snap_settings.clone(),
            geomagnetic_indices: self.geomagnetic_index_settings.clone(),
            interference: self.interference_settings.clone(),
            tec: self.tec_settings.clone(),
            solar_flares: self.solar_flare_settings.clone(),
        }
    }

    /// Re-segment all loaded recordings using the current `processing_config`.
    ///
    /// A recording without nav points has no track structure to re-segment and
    /// is left unchanged.
    fn apply_resegmentation(&mut self) {
        let config = self.processing_config;
        {
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            for file in s.loaded_files.files_mut() {
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
                let all_channels = gt_track_builder::reassemble_channels(&file.tracks);
                let filename = file.metadata.filename.clone();
                let source = file.source.clone();
                let file_meta = gt_track_builder::FileMeta::from(&file.metadata);
                *file = gt_track_builder::build_loaded_file(
                    filename,
                    &all_points,
                    &all_custom_markers,
                    all_event_markers,
                    event_marker_styles,
                    &all_channels,
                    &config,
                    source,
                    file_meta,
                    file.load_warnings.clone(),
                );
            }
            s.plot_state.rebuild_all(&s.loaded_files);
            s.tree.reset_for_files(&s.loaded_files);
        }
        self.on_track_indices_changed();
    }

    fn handle_completed_load(&mut self, completed: CompletedLoad, now: f64) {
        match completed.outcome {
            Ok(LoadOutcome::GtdFile {
                file,
                series,
                history,
                applied_current_marker_settings,
            }) => {
                let was_stored = history.is_stored();
                log::info!(
                    "Loaded '{}': {} track(s), stored in history: {was_stored}",
                    completed.filename,
                    file.tracks.len()
                );
                // Stored recordings may have cached snap runs. The response
                // restores them into the session stores.
                if let Some(db_ref) = history.db_ref() {
                    self.history.load_snap_runs(db_ref.clone());
                    self.history.load_attached_logs(db_ref.clone());
                }
                for track in &file.tracks {
                    self.jamming.request_days_for(track.metadata.time_range);
                    self.geomagnetic_indices
                        .request_days_for(track.metadata.time_range);
                    self.tec_maps.request_days_for(track.metadata.time_range);
                    self.solar_flares
                        .request_days_for(track.metadata.time_range);
                }
                let orphans: Vec<(chrono::DateTime<chrono::Utc>, String)> = file
                    .orphaned_event_markers
                    .iter()
                    .map(|m| (m.time, m.variant_path.clone()))
                    .collect();
                let mut s = self.shared.borrow_mut();
                let fi = s.loaded_files.len();
                s.loaded_files.push(file, history);
                s.sync_tree_from_loaded_files();
                s.plot_state.integrate_file(fi, series);
                drop(s);
                if !orphans.is_empty() {
                    self.orphaned_event_markers = Some(orphans);
                }
                self.load_error = None;
                self.loader.finishing_jobs.push(FinishedJob {
                    filename: completed.filename,
                    elapsed_secs: completed.elapsed_secs,
                    completed_at: now,
                });
                if was_stored {
                    self.history_window.invalidate();
                    self.check_auto_prune();
                }
                self.snap_auto_sweep = true;
                if applied_current_marker_settings {
                    self.toasts
                        .info("Applied current marker settings to loaded data");
                }
            }
            Ok(LoadOutcome::Log {
                filename,
                parsed,
                restored,
            }) => {
                let window = chrono::Duration::seconds(
                    i64::try_from(self.assoc_config.log_association_window_s).unwrap_or(i64::MAX),
                );
                let mut log = LoadedLog::new(filename, parsed, window);
                let mut restored_target = None;
                let restored_from_history = restored.is_some();
                if let Some(restore) = restored {
                    restored_target = self.loaded_recording_id(&restore.attachment.recording);
                    log.restore_attachment(restore.attachment, restore.filters);
                }
                // Associating runs on the UI thread, as the spatial-index
                // rebuild after a recording load does: one binary search per
                // entry, spread over gt-logfile's workers.
                let shared = self.shared.borrow();
                let recordings = shared.loaded_files.view();
                let unambiguous = log
                    .rank_association_candidates(&recordings)
                    .unambiguous_target();
                let ask = self.ask_log_association_target
                    && !restored_from_history
                    && !recordings.is_empty();
                // Anchoring without the user choosing is safe only where there
                // is nothing to choose between, and the dialog is that choice.
                let target = if restored_from_history {
                    restored_target
                } else if ask {
                    None
                } else {
                    unambiguous
                };
                log.associate_with(target, &recordings);
                drop(shared);
                log::info!(
                    "Loaded log {:?}: {} entries, {} of them associated",
                    log.name(),
                    log.parsed().entries().len(),
                    log.associated_entry_count()
                );
                let log_id = self.logs.push(log);
                self.log_viewer.open_on_newly_loaded_log(&self.logs);
                if ask {
                    self.association_dialog = Some(LogAssociationDialog::new(log_id, unambiguous));
                }
                self.load_error = None;
                self.loader.finishing_jobs.push(FinishedJob {
                    filename: completed.filename,
                    elapsed_secs: completed.elapsed_secs,
                    completed_at: now,
                });
            }
            Err(e) => {
                log::error!("Background load failed: {e}");
                self.load_error = Some(e);
            }
        }
    }

    /// The loaded recording stored as `recording`, or `None` once it has been
    /// unloaded.
    fn loaded_recording_id(&self, recording: &gt_store::DatabaseRef) -> Option<LoadedFileId> {
        let shared = self.shared.borrow();
        let recordings = shared.loaded_files.view();
        recordings
            .entries()
            .find(|entry| entry.history().db_ref() == Some(recording))
            .map(|entry| entry.id())
    }

    /// Bookkeeping after any structural change to `loaded_files` (removal,
    /// re-segmentation): track indices shifted, so the spatial index and all
    /// `TrackRef`-keyed transient state must be rebuilt or dropped. Every
    /// mutation site must call this.
    fn on_track_indices_changed(&mut self) {
        self.snap.reset_track_states();
        self.snap_auto_sweep = true;
        self.hidden_snapped.clear();
        self.sky_trails_window.invalidate();
        let s = self.shared.borrow();
        self.map.rebuild_spatial_index(&s.loaded_files);
        self.logs.reassociate_all(&s.loaded_files.view());
    }

    /// Hides the affected recordings, or deletes them when
    /// `outcome.permanent`. The toast is shown when the worker confirms via
    /// [`Self::handle_history_response`].
    fn apply_remove_outcome(&self, outcome: &modals::RemoveOutcome) {
        for removal in &outcome.affected {
            if outcome.permanent {
                self.history
                    .delete_tracks(removal.db_ref.clone(), removal.track_indices.clone());
            } else {
                self.history.set_tracks_hidden(
                    removal.db_ref.clone(),
                    removal.track_indices.clone(),
                    true,
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "app/ui_tests.rs"]
mod tests;
