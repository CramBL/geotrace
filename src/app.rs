use egui::{
    Button, CentralPanel, DragValue, Grid, Label, MenuBar, ProgressBar, RichText, ScrollArea,
    Sides, WidgetText, Window,
};
use egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE as ICON_ARROW_COUNTER_CLOCKWISE;
use egui_phosphor::regular::ARROWS_IN_LINE_HORIZONTAL as ICON_ARROWS_IN_LINE_HORIZONTAL;
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CHART_LINE_UP as ICON_CHART_LINE_UP;
use egui_phosphor::regular::CHECK as ICON_CHECK;
use egui_phosphor::regular::CHECK_CIRCLE as ICON_CHECK_CIRCLE;
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE as ICON_CLOCK_COUNTER_CLOCKWISE;
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::GAUGE as ICON_GAUGE;
use egui_phosphor::regular::GEAR as ICON_GEAR;
use egui_phosphor::regular::LINK_BREAK as ICON_LINK_BREAK;
use egui_phosphor::regular::MAP_PIN as ICON_MAP_PIN;
use egui_phosphor::regular::SCISSORS as ICON_SCISSORS;
use egui_phosphor::regular::SLIDERS_HORIZONTAL as ICON_SLIDERS_HORIZONTAL;
use egui_phosphor::regular::TAG as ICON_TAG;
use egui_phosphor::regular::TERMINAL_WINDOW as ICON_TERMINAL_WINDOW;
use egui_phosphor::regular::TEXT_AA as ICON_TEXT_AA;
use egui_phosphor::regular::TRASH as ICON_TRASH;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use egui_phosphor::regular::WAVE_SINE as ICON_WAVE_SINE;
use egui_phosphor::regular::X as ICON_X;
use egui_phosphor::regular::X_CIRCLE as ICON_X_CIRCLE;
mod auto_prune;
mod history;
mod history_db;
mod loader;
mod modals;
mod query;
mod settings_autosave;
mod snap;
mod snap_persist;
#[cfg(feature = "self-update")]
mod update;

use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, env, path::PathBuf, rc::Rc, str, sync::Arc};

use egui_tiles::{
    Container, Linear, LinearDir, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse,
};
use gt_filter::GlobalFilter;
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_map::{MapContextAction, MapLayer, NavMap};
use gt_plot::PlotState;
use gt_side_panel::{
    FilterPanelState, PanelContext, SnapPanelView, SnapRowView, TreeState, show_side_panel,
};
use gt_snap::wire::Costing;
use gt_track_builder::{GeneratedMarkerConfig, SegmentationConfig, TrackLayoutConfig};
use gt_types::{
    AssociationConfig, DataCategory, FileIdx, LoadWarning, LoadedFile, LoadedTrack, NavPoint,
    TrackIdx, TrackRef,
};
use gt_ui_types::{
    DisplayMask, HighlightScope, MapHighlight, SkyGlyphVariant, TrackDataVisibility,
};
use loader::{CompletedLoad, FinishedJob, LoadJobs, LoadOutcome};
use settings_autosave::{AppSnapshot, SettingsAutosaver};
use strum::IntoEnumIterator;

use modals::{
    SnapAutoChoice, SnapConsentChoice, show_about_dialog, show_delete_confirmation,
    show_load_warnings_dialog, show_mapbox_token_dialog, show_orphaned_event_markers_popup,
    show_recording_details_dialog, show_snap_auto_prompt, show_snap_consent_dialog,
    show_unassociated_popup,
};

/// Pane variants for the central area tiles tree.
enum MainPane {
    Map,
    Plot,
}

struct SharedAppState {
    loaded_files: LoadedFiles,
    tree: TreeState,
    highlight: MapHighlight,
    filter: GlobalFilter,
    /// Global per-category visibility of the map ink. UI to edit it lands
    /// with the display toggle popup; until then it stays all-visible.
    display_mask: DisplayMask,
    /// Which sky-glyph variant the map overlay draws.
    sky_glyph_variant: SkyGlyphVariant,
    /// Which parts of the clicked-point window are folded away. Mirrors the
    /// persisted map setting, like `sky_glyph_variant`.
    point_window_folds: gt_ui_types::PointWindowFolds,
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
    /// The recording whose metadata-details dialog is open, if any.
    metadata_popup: Option<gt_side_panel::RecordingDetails>,
    /// Set by the side panel's "Reset filters", consumed by the app to also
    /// clear the query filter (the query window is not part of shared state).
    clear_query_request: bool,
    /// Set by the map context menu's "Show sky trails", consumed by the app to
    /// open the sky trails window (which is not part of shared state).
    sky_trails_request: Option<gt_ui_types::SkyTrailsRequest>,
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
    db_ref: gt_history::DatabaseRef,
    filename: String,
    bytes: std::sync::Arc<[u8]>,
    /// Settings the stored tracks were built with.
    stored: gt_history::StoredSegmentation,
    /// 0-based positions of the recording's hidden tracks, re-applied to the view
    /// when the user keeps the stored tracks.
    hidden_positions: Vec<usize>,
    /// Whether marker-generation settings differ from the stored/default marker
    /// settings and will be rebuilt from the current app settings when opened.
    marker_settings_changed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct StartupOptions {
    pub fading_enabled: bool,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            fading_enabled: true,
        }
    }
}

/// Snap error data derived from one run for one track: the plot series and
/// the dense per-point values the query providers read. Built once per run
/// (see [`App::with_snap_error_derived`]).
struct SnapErrorDerived {
    /// Identity of the source run, for invalidation.
    run: gt_ui_types::ArcIdentity,
    /// The plot series, one entry per sent point.
    series: Arc<Vec<gt_ui_types::SnapErrorPoint>>,
    /// One slot per track point; `Some` exactly for sent points that came
    /// back snapped or interpolated - the fixed `snap_error` semantics
    /// (see docs/snap/design.md, "Query integration").
    values: Arc<Vec<Option<f64>>>,
}

impl SnapErrorDerived {
    fn build(run: gt_ui_types::ArcIdentity, source: &snap::SnapRun, track: &LoadedTrack) -> Self {
        let mut values = vec![None; track.points.len()];
        let series = source
            .result
            .points
            .iter()
            .filter_map(|p| {
                let nav = p.point.get(&track.points)?;
                if matches!(
                    p.kind,
                    gt_snap::wire::SnapPointKind::Snapped
                        | gt_snap::wire::SnapPointKind::Interpolated
                ) && let (Some(error), Some(slot)) =
                    (p.error_m, values.get_mut(p.point.as_usize()))
                {
                    *slot = Some(error);
                }
                Some(gt_ui_types::SnapErrorPoint {
                    x_secs: nav.tpv.time().as_secs_f64(),
                    error_m: p.error_m,
                    kind: Self::kind(p.kind),
                })
            })
            .collect();
        Self {
            run,
            series: Arc::new(series),
            values: Arc::new(values),
        }
    }

    /// Map gt-snap's wire-format point kind onto the plot's plain mirror
    /// (a mirror so `gt-ui-types` stays free of the gt-snap dependency;
    /// this function is the one place both types are visible).
    fn kind(kind: gt_snap::wire::SnapPointKind) -> gt_ui_types::SnapErrorKind {
        match kind {
            gt_snap::wire::SnapPointKind::Snapped => gt_ui_types::SnapErrorKind::Snapped,
            gt_snap::wire::SnapPointKind::Interpolated => gt_ui_types::SnapErrorKind::Interpolated,
            gt_snap::wire::SnapPointKind::Unsnapped => gt_ui_types::SnapErrorKind::Unsnapped,
        }
    }
}

/// The dense per-component color slots the plot reads, from the settings
/// file's sparse recolored-component entries. Entries past the widest index
/// size the vector; anything absent stays the derived hue.
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
    unassociated_log_lines: Option<Vec<(chrono::DateTime<chrono::Utc>, String)>>,
    orphaned_event_markers: Option<Vec<(chrono::DateTime<chrono::Utc>, String)>>,
    mapbox_token_input: String,

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
    /// the side panel's manual snap trigger while consent is pending; lowered
    /// by the dialog.
    snap_consent_prompt: bool,
    /// Auto mode should sweep for unsnapped tracks: set when files load,
    /// indices shift, or auto mode turns on; consumed once per frame.
    snap_auto_sweep: bool,
    /// Per-run derived snap error data, keyed by track content and
    /// invalidated by the run's `Arc` identity. Downstream caches (the
    /// plot's mipmaps, the query fingerprint) key off the `Arc`s, so they
    /// must stay stable across frames and change exactly when the run does.
    snap_error_cache: HashMap<snap::TrackContentKey, SnapErrorDerived>,
    /// Session-only per-track costing overrides ("Snap again as…"). The
    /// override beats the declared travel mode and the configured default;
    /// content-keyed so it survives index shifts like the run stores. Not
    /// persisted: after a restart the stored run goes stale against the
    /// resolved default again - consistent, never misleading.
    snap_costing_overrides: HashMap<snap::TrackContentKey, Costing>,
    /// The track whose snap trigger raised the consent dialog. Queued when
    /// the dialog is accepted, dropped when it is declined.
    pending_snap: Option<TrackRef>,
    /// Tracks whose completed snapped track is toggled off the map. Session
    /// state, like the snap cache; cleared with the other per-track snap
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

    /// Whether the Settings window is currently open.
    settings_open: bool,
    /// Whether the About dialog is currently open.
    about_open: bool,
    /// The running crate version; fixed to a placeholder in tests so
    /// version-bearing UI snapshots stay stable across release bumps.
    app_version: &'static str,
    /// Active segmentation config - applied to all new file loads and re-segmentation.
    processing_config: SegmentationConfig,
    /// Active association config - applied to all new log loads.
    assoc_config: AssociationConfig,

    /// Background worker that owns the history database. All reads and edits go
    /// through it so the UI thread never blocks on disk I/O.
    history: history_db::HistoryWorker,
    /// Set when the database could not be opened because it is marked as locked
    /// (open for write). Drives a confirmation dialog offering to clear it.
    pending_history_unlock: Option<PathBuf>,
    /// Set when the database could not be opened because it is corrupted. Drives a
    /// dialog offering to recreate it (optionally keeping a backup).
    pending_db_corruption: Option<PathBuf>,
    /// "Keep a backup of the original database" tickbox state for the corruption
    /// recreate dialog.
    keep_db_backup: bool,
    /// Set when a recording is opened from history whose stored segmentation
    /// settings differ from the current ones. Drives the recalculate/use-stored
    /// prompt.
    pending_resegment: Option<ResegmentPrompt>,

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
    query_window: query::QueryWindow,
    /// The whole-track sky trails window.
    sky_trails_window: gt_map::SkyTrailsWindow,

    /// Toast notification queue - rendered every frame over the top of all content.
    toasts: egui_notify::Toasts,

    /// Background startup check for a newer GeoTrace release, plus its prompt.
    /// Only present in dist builds (the `self-update` feature).
    #[cfg(feature = "self-update")]
    update_checker: update::UpdateChecker,
    /// When `true`, check for updates on startup (also gated on release build and
    /// `GEOTRACE_OFFLINE` being unset). Mirrors `settings.update.check_on_startup`.
    update_check_on_startup: bool,
    /// A release version the user chose to skip. Suppresses the update prompt for
    /// exactly this version. Mirrors `settings.update.skipped_version`.
    skipped_version: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_files(cc, &[], StartupOptions::default())
    }

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
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        // GPU-instanced icon rendering; without a wgpu render state (or with
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

        let map = NavMap::new(cc.egui_ctx.clone());
        let loader = LoadJobs::new(cc.egui_ctx.clone());
        let snap = snap::SnapScheduler::new(cc.egui_ctx.clone());

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
        let (history, pending_history_unlock, pending_db_corruption) = {
            use gt_history::{DbError, HistoryDatabase};
            match gt_history::default_path() {
                Ok(path) => match gt_history::Database::open_or_create(&path) {
                    Ok(db) => (
                        history_db::HistoryWorker::spawn(db, cc.egui_ctx.clone()),
                        None,
                        None,
                    ),
                    Err(DbError::WriteLocked) => {
                        log::warn!(
                            "History database at {} is locked (marked open for write)",
                            path.display()
                        );
                        (history_db::HistoryWorker::disabled(), Some(path), None)
                    }
                    Err(e) => {
                        log::error!("History database at {} is unusable: {e}", path.display());
                        (history_db::HistoryWorker::disabled(), None, Some(path))
                    }
                },
                Err(e) => {
                    log::error!("Failed to locate history database: {e}");
                    (history_db::HistoryWorker::disabled(), None, None)
                }
            }
        };
        #[cfg(test)]
        let (history, pending_history_unlock, pending_db_corruption): (
            history_db::HistoryWorker,
            Option<PathBuf>,
            Option<PathBuf>,
        ) = (history_db::HistoryWorker::disabled(), None, None);

        // A fixed placeholder in tests so version-bearing UI snapshots stay
        // stable across release bumps; the real crate version otherwise.
        #[cfg(test)]
        let app_version = TEST_APP_VERSION;
        #[cfg(not(test))]
        let app_version = env!("CARGO_PKG_VERSION");

        let mut app = Self {
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
                map_center_request: None,
                popup_pos_request: None,
                zoom_to_visible_request: false,
                sky_trails_request: None,
                warnings_popup: None,
                metadata_popup: None,
                clear_query_request: false,
                recording_name_template: crate::settings::DEFAULT_RECORDING_NAME_TEMPLATE
                    .to_owned(),
            })),
            load_error: None,
            unassociated_log_lines: None,
            orphaned_event_markers: None,
            mapbox_token_input: String::new(),
            ctx: cc.egui_ctx.clone(),
            loader,
            snap,
            snap_settings: crate::settings::SnapSettings::default(),
            snap_consent_prompt: false,
            snap_auto_sweep: false,
            snap_error_cache: HashMap::new(),
            snap_costing_overrides: HashMap::new(),
            pending_snap: None,
            hidden_snapped: std::collections::HashSet::new(),
            tiles_tree,
            map_tile_id,
            plot_tile_id,
            config: SettingsAutosaver::new(AppSnapshot::default()),
            config_path,
            settings_open: false,
            about_open: false,
            app_version,
            processing_config: SegmentationConfig::default(),
            assoc_config: AssociationConfig::default(),
            history,
            pending_history_unlock,
            pending_db_corruption,
            keep_db_backup: true,
            pending_resegment: None,
            storage_enabled: true,
            auto_prune_enabled: false,
            auto_prune_max_bytes: 10 * 1024 * 1024 * 1024,
            auto_prune_confirm: true,
            pending_auto_prune: None,
            history_window: history::HistoryWindow::new(),
            query_window: query::QueryWindow::new(),
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

                        // Live-apply: re-derives only the analysis-dependent
                        // series, and keeps the loader in step for files loaded
                        // later. `set_analysis` is a no-op when unchanged.
                        self.loader.analysis_config = analysis;
                        // Slip markers share these detection params, but as
                        // load-time generated markers they only pick up the
                        // change on the next load or "Apply to loaded data".
                        self.processing_config.generated_markers.slip_elevation_mask_deg = analysis.elevation_mask_deg;
                        self.processing_config.generated_markers.slip_snr_drop_db = analysis.snr_drop_db;
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
                        ui.label(format!("{ICON_TAG} Recording name"))
                            .on_hover_text(
                                "Template for the name shown for each recording in the side \
                                 panel. Tokens: {title} {device} {identity} {filename}. Empty \
                                 tokens and their separators are dropped; unknown text is kept.",
                            );
                        let mut template = self.shared.borrow().recording_name_template.clone();
                        if ui
                            .text_edit_singleline(&mut template)
                            .on_hover_text("Tokens: {title} {device} {identity} {filename}")
                            .changed()
                        {
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
    fn apply_startup_settings(&mut self, s: &crate::settings::Settings) {
        if !s.map.mapbox_token.is_empty() {
            self.map.set_mapbox_token(s.map.mapbox_token.clone());
        }
        self.map.set_layer(map_layer_from_setting(s.map.layer));
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
                detect_slips: s.processing.detect_slips,
                // Slip markers share the slip-rate plot's detection params so
                // the two always agree.
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
                vis.set(k, s.plot.metric.get(&k).copied().unwrap_or(true));
            }
            let channel_vis = &mut shared.plot_state.channel_vis;
            for (name, &visible) in &s.plot.channel {
                channel_vis.set(name, visible);
            }
            shared.plot_state.channel_component_colors = s
                .plot
                .channel_colors
                .iter()
                .map(|(name, entries)| (name.clone(), dense_component_colors(entries)))
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
        self.snap.set_server_url(&s.snap.server_url);
        self.sync_db_path();
    }

    /// Whether to run the startup update check: enabled in settings, a release
    /// build (avoids hitting GitHub during development), and not offline.
    #[cfg(feature = "self-update")]
    fn should_check_for_updates(&self) -> bool {
        self.update_check_on_startup && !cfg!(debug_assertions) && !gt_types::env::offline()
    }

    /// The costing a track would snap with now: the session override beats
    /// the declared travel mode, which beats the configured default. An
    /// override makes even a road-less declared mode snappable - overriding
    /// wrong declarations is what it exists for.
    fn effective_costing(
        &self,
        file: &gt_types::LoadedFile,
        track: &gt_types::LoadedTrack,
    ) -> Option<Costing> {
        if let Some(&costing) = self
            .snap_costing_overrides
            .get(&snap::TrackContentKey::new(track))
        {
            return Some(costing);
        }
        snap::resolve_costing(
            file.metadata.travel_mode.as_ref(),
            self.snap_settings.costing,
        )
    }

    /// The wire costing for a panel-side mirror choice. Exhaustive both
    /// ways, so a costing added to either side fails to compile here.
    fn costing_from_choice(choice: gt_ui_types::SnapCosting) -> Costing {
        match choice {
            gt_ui_types::SnapCosting::Auto => Costing::Auto,
            gt_ui_types::SnapCosting::Bicycle => Costing::Bicycle,
            gt_ui_types::SnapCosting::Pedestrian => Costing::Pedestrian,
        }
    }

    /// The re-run submenu's choices, labeled from the wire type's canonical
    /// spelling (the single source, [`Costing::display_name`]).
    fn costing_choices() -> Vec<(gt_ui_types::SnapCosting, String)> {
        use strum::IntoEnumIterator;
        gt_ui_types::SnapCosting::iter()
            .map(|choice| {
                let label = Self::costing_from_choice(choice).display_name().to_owned();
                (choice, label)
            })
            .collect()
    }

    /// Act on a "Snap again as" choice: store the session override and run
    /// the track under it, through the consent dialog while consent is
    /// pending. The fresh run is not stale (the override feeds the
    /// effective parameters); a cached run under the chosen costing
    /// redisplays without a request.
    fn handle_snap_costing_request(
        &mut self,
        track_ref: TrackRef,
        choice: gt_ui_types::SnapCosting,
    ) {
        {
            let shared = self.shared.borrow();
            let Some(track) = track_ref.resolve(shared.loaded_files.files()) else {
                return;
            };
            self.snap_costing_overrides.insert(
                snap::TrackContentKey::new(track),
                Self::costing_from_choice(choice),
            );
        }
        self.handle_snap_request(track_ref);
    }

    /// The side panel's per-track snap view: scheduler activity and cached
    /// runs resolved against each file's declared travel mode and the
    /// configured costing. Tracks in the default idle state get no entry.
    fn snap_row_views(&self) -> HashMap<TrackRef, SnapRowView> {
        let shared = self.shared.borrow();
        let mut rows = HashMap::new();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            let declared = file.metadata.travel_mode.as_ref();
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let row = match self.effective_costing(file, track) {
                    None => {
                        // Without an override, `effective_costing` returns
                        // `None` only for a declared road-less mode, so
                        // `declared` is present here; skip (= idle)
                        // defensively rather than unwrap.
                        let Some(mode) = declared else { continue };
                        SnapRowView::Unsnappable {
                            travel_mode: mode.display_name().to_owned(),
                        }
                    }
                    Some(costing) => match self.snap.activity_for(track_ref) {
                        Some(snap::SnapActivity::Queued) => SnapRowView::Queued,
                        Some(snap::SnapActivity::InFlight {
                            completed_chunks,
                            total_chunks,
                        }) => SnapRowView::InFlight {
                            completed_chunks: *completed_chunks,
                            total_chunks: *total_chunks,
                        },
                        Some(snap::SnapActivity::Failed { error }) => SnapRowView::Failed {
                            error: error.clone(),
                        },
                        None => match self.snap.latest_run_for(track) {
                            Some(run) => {
                                let reasons = snap::stale_reasons(
                                    &run,
                                    self.snap_settings.params(costing),
                                    self.snap.current_host().as_deref(),
                                );
                                SnapRowView::Done {
                                    snapped: run.result.kind_counts.snapped,
                                    interpolated: run.result.kind_counts.interpolated,
                                    unsnapped: run.result.kind_counts.unsnapped,
                                    confidence_score: run.result.confidence_score,
                                    shown: !self.hidden_snapped.contains(&track_ref),
                                    stale: (!reasons.is_empty()).then_some(reasons),
                                    partial: run.result.partial,
                                    warnings: run.warnings.iter().map(snap::warning_line).collect(),
                                }
                            }
                            None => continue,
                        },
                    },
                };
                rows.insert(track_ref, row);
            }
        }
        rows
    }

    /// The map's snapped-track geometry: one entry per tree-visible track
    /// whose completed run is not toggled hidden. The per-run projection is
    /// shared via `Arc`, so assembling this each frame is cheap.
    fn snapped_tracks_view(&self) -> gt_ui_types::SnappedTracks {
        let shared = self.shared.borrow();
        let visibility = shared.tree.visibility();
        let mut snapped = gt_ui_types::SnappedTracks::default();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            let fi = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(fi, TrackIdx::new(ti));
                if self.hidden_snapped.contains(&track_ref) {
                    continue;
                }
                if !visibility.track_shown(track_ref) {
                    continue;
                }
                if let Some(run) = self.snap.latest_run_for(track) {
                    snapped
                        .by_track
                        .insert(track_ref, Arc::clone(&run.geometry));
                }
            }
        }
        snapped
    }

    /// The plot's snap error series: per sent point of each completed run,
    /// the point's plot time, its snap error, and its match kind. Unlike the
    /// map geometry this is not gated on visibility or the snapped-track
    /// toggle - the plot filters by its own track visibility, and hiding the
    /// snapped geometry on the map does not retract the error data.
    fn snap_error_view(&mut self) -> gt_ui_types::SnapErrorSeries {
        let mut series = gt_ui_types::SnapErrorSeries::default();
        self.with_snap_error_derived(&mut series, |track_ref, derived, series| {
            series
                .points_by_track
                .insert(track_ref, Arc::clone(&derived.series));
        });
        series
    }

    /// Per-track dense snap error values for the query providers, one entry
    /// per track with a completed run.
    fn snap_error_values(&mut self) -> HashMap<TrackRef, Arc<Vec<Option<f64>>>> {
        let mut values = HashMap::new();
        self.with_snap_error_derived(&mut values, |track_ref, derived, values| {
            values.insert(track_ref, Arc::clone(&derived.values));
        });
        values
    }

    /// Walk every track with a completed run, handing `f` its cached
    /// derived data. The data is built once per run and reused by `Arc`
    /// identity: downstream (the plot's mipmap cache, the query
    /// fingerprint) keys off the pointers, so a fresh allocation per frame
    /// would defeat every cache below this point. Entries for unloaded
    /// tracks are pruned.
    fn with_snap_error_derived<T>(
        &mut self,
        out: &mut T,
        mut f: impl FnMut(TrackRef, &SnapErrorDerived, &mut T),
    ) {
        let shared = self.shared.borrow();
        let mut seen: HashSet<snap::TrackContentKey> = HashSet::new();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let Some(run) = self.snap.latest_run_for(track) else {
                    continue;
                };
                let content = snap::TrackContentKey::new(track);
                seen.insert(content);
                let run_id = gt_ui_types::ArcIdentity::of(&run);
                if self
                    .snap_error_cache
                    .get(&content)
                    .is_none_or(|derived| derived.run != run_id)
                {
                    self.snap_error_cache
                        .insert(content, SnapErrorDerived::build(run_id, &run, track));
                }
                // Present by construction: inserted just above when absent.
                let Some(derived) = self.snap_error_cache.get(&content) else {
                    continue;
                };
                f(track_ref, derived, out);
            }
        }
        self.snap_error_cache
            .retain(|content, _| seen.contains(content));
    }

    /// Act on a snap trigger from the side panel: route through the consent
    /// dialog while consent is pending, queue the run otherwise.
    fn handle_snap_request(&mut self, track_ref: TrackRef) {
        if self.snap_settings.consent_granted() {
            self.queue_snap(track_ref);
        } else {
            self.pending_snap = Some(track_ref);
            self.snap_consent_prompt = true;
        }
    }

    /// Whether any loaded track can snap (no road-less declared mode).
    /// Gates the consent and auto-choice prompts: neither shows on an
    /// empty session.
    fn any_snappable_track(&self) -> bool {
        let shared = self.shared.borrow();
        shared.loaded_files.files().iter().any(|file| {
            !file.tracks.is_empty()
                && snap::resolve_costing(
                    file.metadata.travel_mode.as_ref(),
                    self.snap_settings.costing,
                )
                .is_some()
        })
    }

    /// Enqueue an automatic run for every snappable track without a
    /// displayed run. Hidden tracks park in the queue until shown; tracks
    /// with transient activity (queued, in flight, failed) are left alone
    /// by the scheduler, and stale runs are re-run manually only.
    fn queue_auto_snaps(&mut self) {
        // Offline pauses auto mode entirely (the scheduler would refuse
        // each request anyway; skipping documents the pause and saves the
        // per-track planning work).
        if snap::SnapScheduler::offline() || !self.snap_settings.auto_snap_active() {
            return;
        }
        let shared = self.shared.borrow();
        for (fi, file) in shared.loaded_files.files().iter().enumerate() {
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti));
                let Some(costing) = self.effective_costing(file, track) else {
                    continue;
                };
                if self.snap.latest_run_for(track).is_some() {
                    continue;
                }
                self.snap.request_snap(
                    track_ref,
                    track,
                    self.snap_settings.params(costing),
                    snap::SnapPriority::Auto,
                );
            }
        }
    }

    /// Queue a snap run for a track under its effective costing (session
    /// override, else declared travel mode, else the configured default).
    fn queue_snap(&mut self, track_ref: TrackRef) {
        let shared = self.shared.borrow();
        let files = shared.loaded_files.files();
        let Some(file) = track_ref.fi.get(files) else {
            return;
        };
        let Some(track) = track_ref.resolve(files) else {
            return;
        };
        let Some(costing) = self.effective_costing(file, track) else {
            return;
        };
        self.snap.request_snap(
            track_ref,
            track,
            self.snap_settings.params(costing),
            snap::SnapPriority::Manual,
        );
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
            layer: map_layer_to_setting(self.map.layer()),
            mapbox_token: self.map.mapbox_token().to_owned(),
            sync_to_map: s.plot_state.sync_to_map,
            display_mask: s.display_mask,
            sky_glyph_variant: s.sky_glyph_variant,
            point_window_folds: s.point_window_folds,
            sky_trail_opacity_percent: self.sky_trails_window.trail_opacity_percent().into(),
            theme,
            recording_name_template: s.recording_name_template.clone(),
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
                .clock_discontinuity_sigmas
                .into(),
            detect_slips: self.processing_config.generated_markers.detect_slips,
            elevation_mask_deg: s.plot_state.analysis.elevation_mask_deg.into(),
            mark_masked_fix: s.plot_state.mark_masked_fix,
            snr_drop_db: s.plot_state.analysis.snr_drop_db.into(),
            slip_window_min: s.plot_state.analysis.slip_window_min.into(),
            storage_enabled: self.storage_enabled,
            auto_prune_enabled: self.auto_prune_enabled,
            auto_prune_max_bytes: self.auto_prune_max_bytes,
            auto_prune_confirm: self.auto_prune_confirm,
            update_check_on_startup: self.update_check_on_startup,
            skipped_version: self.skipped_version.clone(),
            query_history_revision: self.query_window.history_revision(),
            snap: self.snap_settings.clone(),
        }
    }

    fn collect_settings_for_flush(&self) -> crate::settings::Settings {
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
                    .map(|(name, colors)| (name.clone(), sparse_component_colors(colors)))
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
                detect_slips: self.processing_config.generated_markers.detect_slips,
            },
            analysis: crate::settings::AnalysisSettings {
                elevation_mask_deg: s.plot_state.analysis.elevation_mask_deg,
                mark_masked_fix: s.plot_state.mark_masked_fix,
                snr_drop_db: s.plot_state.analysis.snr_drop_db,
                slip_window_min: s.plot_state.analysis.slip_window_min,
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

    /// Process a completed background load: integrate the result into shared state.
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
                // Stored recordings may carry cached snap runs; fetch them
                // (the response restores them into the session stores).
                if let Some(db_ref) = history.db_ref() {
                    self.history.load_snap_runs(db_ref.clone());
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
                    // The recording list now has a new entry, refresh it.
                    self.history_window.invalidate();
                    self.check_auto_prune();
                }
                self.snap_auto_sweep = true;
                if applied_current_marker_settings {
                    self.toasts
                        .info("Applied current marker settings to loaded data");
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
                    s.loaded_files.push(loaded, FileHistory::None);
                    s.sync_tree_from_loaded_files();
                    s.plot_state.integrate_file(fi, series);
                }
                if !unassociated.is_empty() {
                    self.unassociated_log_lines = Some(unassociated);
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

    fn sync_db_path(&mut self) {
        self.loader.db_path = if self.storage_enabled {
            self.history.path().map(std::path::Path::to_owned)
        } else {
            None
        };
    }

    /// Clear a stale write lock and bring the history database online, after the
    /// user confirmed no other process is using it.
    fn recover_history_database(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        use gt_history::HistoryDatabase;
        let result = gt_history::Database::clear_write_lock(path)
            .and_then(|()| gt_history::Database::open_or_create(path));
        match result {
            Ok(db) => {
                self.history = history_db::HistoryWorker::spawn(db, ctx.clone());
                self.sync_db_path();
                self.history_window.invalidate();
                self.toasts.info("Recovered the history database");
            }
            Err(e) => {
                log::error!("Failed to recover history database: {e}");
                self.toasts
                    .error(format!("Could not recover history database: {e}"));
            }
        }
    }

    /// Recreate a corrupted history database from scratch, optionally renaming the
    /// unreadable original to `<name>.corrupt.bak` first.
    fn recreate_history_database(
        &mut self,
        path: &std::path::Path,
        keep_backup: bool,
        ctx: &egui::Context,
    ) {
        use gt_history::HistoryDatabase;
        if keep_backup {
            let backup = corrupt_backup_path(path);
            if let Err(e) = std::fs::rename(path, &backup) {
                log::error!("Failed to back up corrupted database: {e}");
                self.toasts
                    .error(format!("Could not back up the database: {e}"));
                return;
            }
            log::info!("Backed up corrupted database to {}", backup.display());
        } else if let Err(e) = std::fs::remove_file(path) {
            log::error!("Failed to remove corrupted database: {e}");
            self.toasts
                .error(format!("Could not remove the database: {e}"));
            return;
        }

        match gt_history::Database::open_or_create(path) {
            Ok(db) => {
                self.history = history_db::HistoryWorker::spawn(db, ctx.clone());
                self.sync_db_path();
                self.history_window.invalidate();
                self.toasts.info("Created a fresh history database");
            }
            Err(e) => {
                log::error!("Failed to recreate history database: {e}");
                self.toasts
                    .error(format!("Could not recreate the database: {e}"));
            }
        }
    }

    /// Ask the history worker whether auto-pruning is needed. The result comes
    /// back as a [`history_db::Response::AutoPruned`]. Called after each
    /// successful GTD insert.
    fn check_auto_prune(&self) {
        if !self.auto_prune_enabled {
            return;
        }
        self.history
            .auto_prune(self.auto_prune_max_bytes, self.auto_prune_confirm);
    }

    /// Apply the history side of a "remove" confirmation: hide the affected
    /// recordings, or permanently delete them when the user opted in. The toast
    /// is shown when the worker confirms via [`Self::handle_history_response`].
    /// Bookkeeping after any structural change to `loaded_files` (removal,
    /// re-segmentation): track indices shifted, so the spatial index and all
    /// `TrackRef`-keyed transient state must be rebuilt or dropped. Every
    /// mutation site must call this - pairing the two here keeps a future
    /// mutation site from forgetting one of them.
    fn on_track_indices_changed(&mut self) {
        self.snap.reset_track_states();
        self.snap_auto_sweep = true;
        self.hidden_snapped.clear();
        self.sky_trails_window.invalidate();
        let s = self.shared.borrow();
        self.map.rebuild_spatial_index(&s.loaded_files);
    }

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

    /// Begin opening a recording from history. Reproduces the stored tracks,
    /// re-applies the hidden ones, and regenerates markers with current settings.
    /// When the stored track-splitting setting differs from the current one it
    /// raises a prompt instead (recalculate vs. use the stored tracks).
    fn begin_history_open(
        &mut self,
        db_ref: gt_history::DatabaseRef,
        stored: gt_history::StoredRecording,
    ) {
        // Reuse the original filename: the identity is the filename (with an
        // "auto:" prefix for auto-derived ones).
        let filename = db_ref
            .identity
            .strip_prefix("auto:")
            .unwrap_or(&db_ref.identity)
            .to_owned();

        let hidden_positions: Vec<usize> = stored
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.hidden)
            .map(|(i, _)| i)
            .collect();

        match stored.segmentation {
            // Stored track splitting differs from the current setting: let the
            // user choose before changing track ranges that hidden-track state
            // may refer to.
            Some(stored_settings)
                if !loader::track_split_matches_config(
                    &stored_settings,
                    &self.processing_config,
                ) && !stored.tracks.is_empty() =>
            {
                let marker_settings_changed = !loader::marker_settings_match_config(
                    &stored_settings,
                    &self.processing_config,
                );
                self.pending_resegment = Some(ResegmentPrompt {
                    db_ref,
                    filename,
                    bytes: stored.bytes.into(),
                    stored: stored_settings,
                    hidden_positions,
                    marker_settings_changed,
                });
            }
            // Track splitting matches: reproduce the stored tracks, re-apply
            // hidden ones, and rebuild generated markers from current settings.
            Some(stored_settings) => {
                let marker_settings_changed = !loader::marker_settings_match_config(
                    &stored_settings,
                    &self.processing_config,
                );
                let config = loader::config_from_stored_segmentation(
                    &stored_settings,
                    self.processing_config,
                );
                self.loader.spawn_gtd_from_history(
                    stored.bytes.into(),
                    filename,
                    config,
                    loader::HistoryOpen::ApplyHidden {
                        db_ref,
                        positions: hidden_positions,
                        applied_current_marker_settings: marker_settings_changed,
                    },
                );
            }
            // Older recording with no stored settings: load with current settings.
            None => {
                self.loader.spawn_gtd_from_history(
                    stored.bytes.into(),
                    filename,
                    self.processing_config,
                    loader::HistoryOpen::ApplyHidden {
                        db_ref,
                        positions: hidden_positions,
                        applied_current_marker_settings: false,
                    },
                );
            }
        }
    }

    /// Apply a result delivered by the history worker thread.
    fn handle_history_response(&mut self, resp: history_db::Response) {
        use history_db::Response;
        match resp {
            Response::Listed(Ok(entries)) => self.history_window.set_entries(entries),
            Response::Listed(Err(e)) => {
                self.history_window
                    .set_error(format!("Failed to load history: {e}"));
            }
            Response::Opened { db_ref, result } => match result {
                Ok(stored) => self.begin_history_open(db_ref, stored),
                Err(e) => {
                    log::error!("Failed to load recording from history: {e}");
                    self.toasts.error(format!("Could not open recording: {e}"));
                }
            },
            Response::Mutated { op, result } => match result {
                Ok(()) => {
                    // Keep loaded recordings pointing at the renamed identity so
                    // later history operations on them still resolve.
                    if let history_db::DbOp::IdentityRenamed { old, new } = &op {
                        self.shared
                            .borrow_mut()
                            .loaded_files
                            .rename_identity(old, new);
                    }
                    self.history_window.invalidate();
                    self.toasts.info(mutation_toast(&op));
                }
                Err(e) => {
                    log::error!("History update failed: {e}");
                    self.toasts.error(format!("History update failed: {e}"));
                }
            },
            Response::PrunePreview(Ok(refs)) => self.history_window.set_prune_preview(refs),
            Response::PrunePreview(Err(e)) => log::error!("Prune preview failed: {e}"),
            Response::AutoPruned(Ok(auto_prune::AutoPruneOutcome::NotNeeded)) => {}
            Response::AutoPruned(Ok(auto_prune::AutoPruneOutcome::PrunedSilently(n))) => {
                self.history_window.invalidate();
                let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                self.toasts
                    .info(format!("Auto-pruned {n} {rec_label}"))
                    .duration(Some(std::time::Duration::from_secs(4)));
            }
            Response::AutoPruned(Ok(auto_prune::AutoPruneOutcome::NeedsConfirmation(
                candidates,
            ))) => {
                self.pending_auto_prune = Some(candidates);
            }
            Response::AutoPruned(Err(e)) => log::error!("Auto-prune failed: {e}"),
            // A failed cache write costs only the persisted copy - the
            // session stores keep working - so this logs instead of toasting.
            Response::SnapRunsStored(result) => {
                if let Err(e) = result {
                    log::warn!("Storing snap runs failed: {e}");
                }
            }
            Response::SnapRunsLoaded { db_ref, blob } => match blob {
                Ok(Some(bytes)) => self.restore_snap_runs(&db_ref, &bytes),
                Ok(None) => {}
                Err(e) => log::warn!("Loading stored snap runs failed: {e}"),
            },
        }
    }

    /// Seed a recording's stored snap runs into the session stores. Runs
    /// are matched to tracks by content fingerprint, so index shifts or a
    /// re-segmentation since storage simply leave non-matching entries
    /// unrestored. Each run restores once, to its first matching track
    /// among the files loaded from this recording; the content-keyed
    /// stores serve every duplicate of that track from the same entry.
    fn restore_snap_runs(&mut self, db_ref: &gt_history::DatabaseRef, blob: &[u8]) {
        let Some(stored) = snap_persist::decode(blob) else {
            return;
        };
        let shared = self.shared.borrow();
        let view = shared.loaded_files.view();
        for run in stored {
            let target = view.entries().enumerate().find_map(|(fi, entry)| {
                if entry.history().db_ref() != Some(db_ref) {
                    return None;
                }
                entry
                    .file()
                    .tracks
                    .iter()
                    .position(|track| run.matches(track))
                    .map(|ti| TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti)))
            });
            let Some(track_ref) = target else {
                continue;
            };
            let Some(track) = track_ref.resolve(view.files()) else {
                continue;
            };
            self.snap.restore_run(track_ref, track, run.into_run());
        }
    }

    /// Persist the latest snap runs of every history-stored file that owns
    /// one of the just-completed tracks. The whole file's runs are written
    /// each time (the blob holds all of them), so the stored copy always
    /// mirrors the session's latest state.
    fn persist_snap_runs(&self, completed: &[snap::TrackContentKey]) {
        let shared = self.shared.borrow();
        for entry in shared.loaded_files.view().entries() {
            let file = entry.file();
            let affected = file
                .tracks
                .iter()
                .any(|track| completed.contains(&snap::TrackContentKey::new(track)));
            if !affected {
                continue;
            }
            let Some(db_ref) = entry.history().db_ref().cloned() else {
                continue;
            };
            let runs: Vec<(&gt_types::LoadedTrack, std::sync::Arc<snap::SnapRun>)> = file
                .tracks
                .iter()
                .filter_map(|track| self.snap.latest_run_for(track).map(|run| (track, run)))
                .collect();
            let blob = snap_persist::encode(runs.iter().map(|(track, run)| (*track, run.as_ref())));
            self.history.store_snap_runs(db_ref, blob);
        }
    }
}

/// Value an advanced snap option is seeded with when enabled, chosen once
/// per option so enabling is a meaningful starting point rather than a
/// range edge. Search radius: the tuned fixture's value, comfortably wider
/// than typical GNSS noise.
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

/// Backup path for a corrupted database: appends `.corrupt.bak` to the file name
/// (e.g. `geotrace.h5` -> `geotrace.h5.corrupt.bak`).
fn corrupt_backup_path(path: &std::path::Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(".corrupt.bak");
    path.with_file_name(name)
}

/// Build the completion toast for a finished history mutation.
fn mutation_toast(op: &history_db::DbOp) -> String {
    use history_db::{DbOp, DeleteReason};
    match op {
        DbOp::TracksHidden { count } => {
            let tracks = gt_fmt::pluralize(*count, "track", "tracks");
            format!("Hid {count} {tracks} in history")
        }
        DbOp::TracksDeleted { count } => {
            let tracks = gt_fmt::pluralize(*count, "track", "tracks");
            format!("Permanently deleted {count} {tracks} from history")
        }
        DbOp::RecordingsDeleted { count, reason } => {
            let rec = gt_fmt::pluralize(*count, "recording", "recordings");
            match reason {
                DeleteReason::Manual => format!("Deleted {count} {rec} from history"),
                DeleteReason::Prune => format!("Pruned {count} {rec} from history"),
                DeleteReason::AutoPrune => format!("Auto-pruned {count} {rec}"),
            }
        }
        DbOp::IdentityRenamed { new, .. } => {
            let (name, _) = gt_loaded_files::display_identity(new);
            format!("Renamed identity to \"{name}\"")
        }
    }
}

/// Behavior implementation that renders each pane of the central tiles tree.
struct MainBehavior<'a> {
    map: &'a mut NavMap,
    state: &'a mut SharedAppState,
    plot_hover_scope: Option<HighlightScope>,
    map_hover_time: Option<chrono::DateTime<chrono::Utc>>,
    /// Time span of the match hovered in the query results table, shaded on
    /// the plot (one frame behind the query window, like `query_matches`).
    match_hover_time_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    toggle_plot_request: bool,
    /// Matches of the last query run, drawn as halos (one frame behind the
    /// query window, which renders after the tiles tree).
    query_matches: Option<&'a gt_ui_types::QueryMatches>,
    /// Snapped-track geometry of completed, shown snap runs.
    snapped_tracks: &'a gt_ui_types::SnappedTracks,
    /// Snap error per track of completed snap runs, for the plot.
    snap_error: &'a gt_ui_types::SnapErrorSeries,
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
                    &mut s.display_mask,
                    &mut s.sky_glyph_variant,
                    &mut s.point_window_folds,
                    s.tree.event_marker_visibility(),
                    s.tree.generated_marker_visibility(),
                    self.query_matches,
                    Some(self.snapped_tracks),
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
                        MapContextAction::ShowSkyTrails(request) => {
                            s.sky_trails_request = Some(request);
                        }
                    }
                }
            }
            MainPane::Plot => {
                egui::Panel::top("plot_header").show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(ICON_CARET_DOWN)
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
                    self.match_hover_time_range,
                    map_sync_x_range,
                    self.snap_error,
                    &mut s.plot_state,
                );
            }
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &MainPane) -> WidgetText {
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
        // Kick off the one-shot startup update check (no-op after the first
        // frame, and only when enabled / release build / not offline).
        #[cfg(feature = "self-update")]
        if self.should_check_for_updates() {
            self.update_checker.start(ui.ctx());
        }

        // Drain background load results first so newly loaded data is
        // visible in the same frame that it arrives.
        let completed_loads: Vec<CompletedLoad> = self.loader.drain();
        let frame_time = ui.ctx().input(|i| i.time);
        for completed in completed_loads {
            self.handle_completed_load(completed, frame_time);
        }

        // Apply any results the history worker has finished since last frame.
        for resp in self.history.poll() {
            self.handle_history_response(resp);
        }

        // Apply finished snap runs and progress updates, persist completed
        // runs of history-stored files, and let the queue react to
        // visibility changes (parked entries may become eligible).
        let completed_snaps = self.snap.poll();
        if !completed_snaps.is_empty() {
            self.persist_snap_runs(&completed_snaps);
        }
        self.snap
            .set_visibility(self.shared.borrow().tree.visibility());
        if std::mem::take(&mut self.snap_auto_sweep) {
            self.queue_auto_snaps();
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
            if delete_pressed && !s.tree.selection.is_empty() && s.tree.pending_unload.is_none() {
                // Delete key unloads the selection from the view (non-destructive,
                // recordings stay in history).
                s.tree.pending_unload = Some(s.tree.selection.iter().cloned().collect());
            }
        }

        egui::Panel::top("top_panel").show_inside(ui, |ui| {
            MenuBar::new().ui(ui, |ui| {
                // Left zone - the File menu
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        self.loader.open_file_dialog();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("About GeoTrace").clicked() {
                        self.about_open = true;
                        ui.close();
                    }
                });

                // Right zone - utility windows and preferences, trailing-aligned
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::widgets::global_theme_preference_buttons(ui);

                    ui.separator();

                    if ui
                        .selectable_label(self.settings_open, ICON_GEAR)
                        .on_hover_text("Settings")
                        .clicked()
                    {
                        self.settings_open = !self.settings_open;
                    }

                    if ui
                        .selectable_label(self.history_window.open, ICON_CLOCK_COUNTER_CLOCKWISE)
                        .on_hover_text("Browse and re-open previously recorded sessions")
                        .clicked()
                    {
                        self.history_window.open = !self.history_window.open;
                        self.history_window.invalidate();
                    }

                    // While a query is filtering the map but its window is
                    // closed, the button turns amber with a "!" so the active
                    // filter is not forgotten; right-click clears it. The amber
                    // is dimmed in light mode, where the bright tone glares.
                    let query_active = self.query_window.filter_active();
                    let show_alert = query_active && !self.query_window.open;
                    let query_label = if show_alert {
                        RichText::new(format!("{ICON_TERMINAL_WINDOW} !"))
                            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode))
                    } else {
                        RichText::new(ICON_TERMINAL_WINDOW)
                    };
                    let query_button = ui.selectable_label(self.query_window.open, query_label);
                    let query_button = if query_active {
                        query_button.on_hover_text(format!(
                            "Query the loaded data. A query filter is active {} \
                             right-click to clear it.",
                            gt_ui_theme::EM_DASH
                        ))
                    } else {
                        query_button.on_hover_text("Query the loaded data")
                    };
                    if query_button.clicked() {
                        self.query_window.open = !self.query_window.open;
                    }
                    if query_active {
                        query_button.context_menu(|ui| {
                            if ui
                                .button(format!("{ICON_TRASH} Clear query filter"))
                                .clicked()
                            {
                                self.query_window.clear_filter();
                                ui.close();
                            }
                        });
                    }

                    // A subtle "update available" hint for builds that can't
                    // self-update (Homebrew, MSI, manual download). Self-updatable
                    // installs get the prompt instead of this badge.
                    #[cfg(feature = "self-update")]
                    if let Some(new_version) = self.update_checker.badge_version() {
                        ui.separator();
                        let text = RichText::new("Update available")
                            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode));
                        if ui
                            .add(Label::new(text).sense(egui::Sense::click()))
                            .on_hover_text(format!(
                                "GeoTrace {new_version} is available (current: {}). Update \
                                 through your package manager, or open the releases page.",
                                self.app_version
                            ))
                            .clicked()
                        {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(update::RELEASES_URL));
                        }
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

        // Snap view for the side panel, resolved once per frame and shared by
        // the docked and detached call sites. The trigger's request is drained
        // after the panel, mirroring the other panel requests.
        let snap_rows = self.snap_row_views();
        let snap_costing_choices = Self::costing_choices();
        let snap_progress = {
            let progress = self.snap.progress();
            gt_side_panel::SnapProgressView {
                in_flight: progress
                    .in_flight
                    .map(|run| gt_side_panel::SnapInFlightView {
                        track: run.track,
                        completed_chunks: run.completed_chunks,
                        total_chunks: run.total_chunks,
                    }),
                queued: progress.queued,
            }
        };
        let snap_view = SnapPanelView {
            offline: snap::SnapScheduler::offline(),
            consent_pending: !self.snap_settings.consent_granted(),
            rows: &snap_rows,
            costing_choices: &snap_costing_choices,
            progress: &snap_progress,
        };
        let mut snap_request: Option<TrackRef> = None;
        let mut snap_visibility_request: Option<TrackRef> = None;
        let mut snap_costing_request: Option<(TrackRef, gt_ui_types::SnapCosting)> = None;
        let mut sky_trails_request: Option<gt_ui_types::SkyTrailsRequest> = None;

        let detached = self.shared.borrow().tree.detached;
        if !detached {
            egui::Panel::left("track_data_panel")
                .min_size(240.0)
                .show_inside(ui, |ui| {
                    let mut refmut = self.shared.borrow_mut();
                    let s = &mut *refmut;
                    let loaded_files = s.loaded_files.view();
                    show_side_panel(
                        ui,
                        &mut PanelContext {
                            loaded_files,
                            tree: &mut s.tree,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            map_center_request: &mut s.map_center_request,
                            popup_pos_request: &mut s.popup_pos_request,
                            zoom_to_visible_request: &mut s.zoom_to_visible_request,
                            warnings_request: &mut s.warnings_popup,
                            clear_query_request: &mut s.clear_query_request,
                            display_mask: s.display_mask,
                            recording_name_template: &s.recording_name_template,
                            metadata_request: &mut s.metadata_popup,
                            snap: snap_view,
                            snap_request: &mut snap_request,
                            snap_visibility_request: &mut snap_visibility_request,
                            snap_costing_request: &mut snap_costing_request,
                            sky_trails_request: &mut sky_trails_request,
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
            Window::new("Track data")
                .id(egui::Id::new("detached_panel"))
                .open(&mut is_open)
                .default_pos(egui::pos2(10.0, 30.0))
                .default_width(320.0)
                .min_width(240.0)
                .resizable(true)
                .show(ui.ctx(), |ui| {
                    let mut refmut = self.shared.borrow_mut();
                    let s = &mut *refmut;
                    let loaded_files = s.loaded_files.view();
                    show_side_panel(
                        ui,
                        &mut PanelContext {
                            loaded_files,
                            tree: &mut s.tree,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            map_center_request: &mut s.map_center_request,
                            popup_pos_request: &mut s.popup_pos_request,
                            zoom_to_visible_request: &mut s.zoom_to_visible_request,
                            warnings_request: &mut s.warnings_popup,
                            clear_query_request: &mut s.clear_query_request,
                            display_mask: s.display_mask,
                            recording_name_template: &s.recording_name_template,
                            metadata_request: &mut s.metadata_popup,
                            snap: snap_view,
                            snap_request: &mut snap_request,
                            snap_visibility_request: &mut snap_visibility_request,
                            snap_costing_request: &mut snap_costing_request,
                            sky_trails_request: &mut sky_trails_request,
                        },
                    );
                });
            if !is_open {
                self.shared.borrow_mut().tree.detached = false;
            }
        }

        if let Some(track_ref) = snap_request {
            self.handle_snap_request(track_ref);
        }
        if let Some((track_ref, choice)) = snap_costing_request {
            self.handle_snap_costing_request(track_ref, choice);
        }
        if let Some(track_ref) = snap_visibility_request
            && !self.hidden_snapped.remove(&track_ref)
        {
            self.hidden_snapped.insert(track_ref);
        }

        // "Show sky trails" from either the side panel or the map context
        // menu (the latter routed through shared state) opens the window.
        let map_trails_request = self.shared.borrow_mut().sky_trails_request.take();
        if let Some(request) = sky_trails_request.or(map_trails_request) {
            self.sky_trails_window.open(request);
        }

        // "Reset filters" also drops the query filter so the map fully clears.
        if std::mem::take(&mut self.shared.borrow_mut().clear_query_request) {
            self.query_window.clear_filter();
        }

        // Assembled after the panel so a visibility toggle takes effect in
        // the same frame's map render.
        let snapped_tracks = self.snapped_tracks_view();
        let snap_error = self.snap_error_view();
        let snap_error_values = self.snap_error_values();

        CentralPanel::default().show_inside(ui, |ui| {
            let panel_rect = ui.max_rect();
            let mut s = self.shared.borrow_mut();
            let plot_hover_scope = match s.highlight.hover {
                Some(HighlightScope::File { .. })
                | Some(HighlightScope::Track(_))
                | Some(HighlightScope::TrackCategory { .. }) => s.highlight.hover,
                Some(HighlightScope::Point(_)) | None => None,
            };
            // Falls back to the sky-trails scrubber's instant (written by that
            // window last frame) so playback draws the same plot time line a
            // track-point hover does.
            let map_hover_time =
                extract_map_hover_time(&s.loaded_files, &s.highlight).or(s.highlight.scrub_time);
            let match_hover_time_range =
                extract_match_hover_time_range(&s.loaded_files, &s.highlight);

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
                    match_hover_time_range,
                    toggle_plot_request: false,
                    query_matches: self.query_window.matches(),
                    snapped_tracks: &snapped_tracks,
                    snap_error: &snap_error,
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
                if let Some(cursor_time) = s.plot_state.hovered_time {
                    let closest = gt_plot::find_closest_tpv(
                        &s.loaded_files,
                        s.tree.visibility(),
                        &s.filter,
                        cursor_time,
                    );
                    s.highlight.plot_hover_time = closest.map(|_| cursor_time);
                    s.highlight.plot_hover_point = closest;
                    // `plot_cursor_snapped` is computed inside `show_track_plot`
                    // using a 2-D screen-space check (both time and metric value)
                    // so the overlay only triggers when egui_plot would also
                    // show a hover label.
                    s.highlight.plot_hover_snapped = s.plot_state.plot_cursor_snapped;
                } else {
                    s.highlight.plot_hover_time = None;
                    s.highlight.plot_hover_point = None;
                    s.highlight.plot_hover_snapped = false;
                }
            } else {
                s.plot_state.hovered_time = None;
                s.highlight.plot_hover_time = None;
                s.highlight.plot_hover_point = None;
                s.highlight.plot_hover_snapped = false;

                let btn_size = egui::vec2(28.0, 22.0);
                let btn_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.min.x + 8.0, panel_rect.max.y - btn_size.y - 8.0),
                    btn_size,
                );
                if ui
                    .put(btn_rect, Button::new(ICON_CHART_LINE_UP).small())
                    .on_hover_text("Show plot")
                    .clicked()
                {
                    self.tiles_tree.tiles.toggle_visibility(self.plot_tile_id);
                }
            }

            // After the plot-hover forwarding: match-table row hover writes
            // the same cross-highlight fields and must win for the frame.
            let SharedAppState {
                loaded_files,
                tree,
                highlight,
                filter,
                map_center_request,
                popup_pos_request,
                plot_state,
                ..
            } = &mut *s;
            // The map and plot consumed last frame's hovered match above;
            // clearing here keeps it set only while a header is hovered.
            highlight.hover_match = None;
            // Likewise the scrub line: cleared here (after the plot read last
            // frame's value) so it stays set only while the sky-trails window
            // below is driving a scrub, and vanishes as soon as it closes.
            highlight.scrub_time = None;
            self.query_window.show(
                ui.ctx(),
                query::RunInputs {
                    loaded_files: loaded_files.view(),
                    visibility: tree.visibility(),
                    filter,
                    snap_errors: &snap_error_values,
                },
                highlight,
                &mut query::MatchMapRequests {
                    map_center: map_center_request,
                    popup_pos: popup_pos_request,
                },
            );
            self.sky_trails_window.show(
                ui.ctx(),
                loaded_files.files(),
                plot_state.analysis.elevation_mask_deg,
                highlight,
            );
        });

        let apply_resegment = self.show_settings_window(ui);
        if apply_resegment {
            self.apply_resegmentation();
        }

        if self.map.layer() == MapLayer::Satellite && !self.map.has_mapbox_token() {
            show_mapbox_token_dialog(ui, &mut self.map, &mut self.mapbox_token_input);
        }

        show_about_dialog(ui, &mut self.about_open, self.app_version);

        // Auto mode armed without acknowledged uploads (the checkbox was
        // enabled, or the server host changed): consent is asked on the
        // first load with a snappable track, before anything is sent.
        if self.snap_settings.auto_snap == Some(true)
            && !self.snap_settings.consent_granted()
            && !self.snap_consent_prompt
            && self.any_snappable_track()
        {
            self.snap_consent_prompt = true;
        }

        if self.snap_consent_prompt {
            let ask_auto = self.snap_settings.auto_snap.is_none();
            match show_snap_consent_dialog(ui, &self.snap_settings.server_url, ask_auto) {
                Some(SnapConsentChoice::Accepted { auto_snap }) => {
                    self.snap_settings.acknowledge_consent();
                    if let Some(auto) = auto_snap {
                        self.snap_settings.auto_snap = Some(auto);
                    }
                    self.snap_consent_prompt = false;
                    // The click that raised the dialog proceeds now that
                    // uploads are acknowledged.
                    if let Some(track_ref) = self.pending_snap.take() {
                        self.queue_snap(track_ref);
                    }
                    self.snap_auto_sweep = true;
                }
                Some(SnapConsentChoice::Declined) => {
                    // The acknowledgment stays unset (the next manual
                    // trigger re-prompts), but the auto choice persists as
                    // off: declined consent never leaves auto uploads armed.
                    self.snap_settings.auto_snap = Some(false);
                    self.snap_consent_prompt = false;
                    self.pending_snap = None;
                }
                None => {}
            }
        } else if self.snap_settings.auto_snap.is_none()
            && self.snap_settings.consent_granted()
            && self.any_snappable_track()
        {
            // Uploads were acknowledged before auto mode existed: ask the
            // mode choice once, before anything would auto-upload.
            if let Some(choice) = show_snap_auto_prompt(ui, &self.snap_settings.server_url) {
                self.snap_settings.auto_snap = Some(choice == SnapAutoChoice::Automatic);
                self.snap_auto_sweep = true;
            }
        }

        // Loading progress overlay in the bottom-right corner. Shows in-flight
        // jobs with a live elapsed timer, plus recently completed jobs that fade
        // out over ~3 seconds so the user can see how long it took.
        let now = ui.ctx().input(|i| i.time);
        let any_finishing = !self.loader.finishing_jobs.is_empty();
        self.loader.expire_finished(now);

        if !self.loader.loading_jobs.is_empty() || any_finishing {
            // Keep repainting while jobs are active or fading.
            ui.ctx().request_repaint();

            Window::new("##loading_progress")
                .title_bar(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(260.0);
                    // Cap the width so a long recording name truncates instead of
                    // stretching this auto-sized overlay across the window.
                    ui.set_max_width(340.0);

                    for job in &self.loader.loading_jobs {
                        let elapsed = job.started_at.elapsed().as_secs_f32();
                        Sides::new().shrink_left().truncate().show(
                            ui,
                            |ui| {
                                ui.spinner();
                                ui.add(
                                    Label::new(RichText::new(&job.filename).strong()).truncate(),
                                )
                                .on_hover_text(&job.filename);
                            },
                            |ui| {
                                ui.label(RichText::new(format!("{elapsed:.1}s")).small().weak());
                            },
                        );
                        ui.add(
                            ProgressBar::new(job.progress)
                                .animate(true)
                                .desired_width(240.0)
                                .text(job.stage),
                        );
                        ui.add_space(2.0);
                    }

                    for job in &self.loader.finishing_jobs {
                        let since = (now - job.completed_at) as f32;
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
                        Sides::new().shrink_left().truncate().show(
                            ui,
                            |ui| {
                                ui.label(RichText::new(ICON_CHECK).color(color).small());
                                ui.add(
                                    Label::new(RichText::new(&job.filename).color(color).strong())
                                        .truncate(),
                                )
                                .on_hover_text(&job.filename);
                            },
                            |ui| {
                                ui.label(
                                    RichText::new(format!("{:.1}s", job.elapsed_secs))
                                        .color(weak_color)
                                        .small(),
                                );
                            },
                        );
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
                        gt_ui_theme::error_indicator(ui.visuals().dark_mode),
                        format!("{ICON_WARNING} {error}"),
                    );
                    dismiss = ui.small_button(ICON_X).clicked();
                });
            }
        });
        if dismiss {
            self.load_error = None;
        }

        // Unload (context menu / Delete key): remove items from the view only.
        // The recordings stay in history, so no confirmation is needed.
        let unloaded = {
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            if let Some(items) = s.tree.pending_unload.take() {
                modals::execute_delete(&items, &mut s.loaded_files, &mut s.tree);
                s.plot_state.rebuild_all(&s.loaded_files);
                Some(items.len())
            } else {
                None
            }
        };
        if let Some(count) = unloaded {
            self.on_track_indices_changed();
            log::info!("Unloaded {count} item(s) from view");
        }

        let remove_outcome = {
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            let outcome = show_delete_confirmation(ui, &mut s.tree, &mut s.loaded_files);
            if outcome.is_some() {
                s.plot_state.rebuild_all(&s.loaded_files);
            }
            outcome
        };
        if let Some(outcome) = remove_outcome {
            self.on_track_indices_changed();
            self.apply_remove_outcome(&outcome);
        }

        show_unassociated_popup(ui, &mut self.unassociated_log_lines);
        show_orphaned_event_markers_popup(ui, &mut self.orphaned_event_markers);
        show_load_warnings_dialog(ui, &mut self.shared.borrow_mut().warnings_popup);
        show_recording_details_dialog(ui, &mut self.shared.borrow_mut().metadata_popup);

        let prev_storage = self.storage_enabled;
        let loaded_metas: Vec<gt_history::RecordingMeta> = {
            let s = self.shared.borrow();
            s.loaded_files.view().recording_metas()
        };
        self.history_window.show(
            ui.ctx(),
            &self.history,
            &loaded_metas,
            &mut self.storage_enabled,
            &mut self.auto_prune_enabled,
            &mut self.auto_prune_max_bytes,
            &mut self.auto_prune_confirm,
        );
        if self.storage_enabled != prev_storage {
            self.sync_db_path();
        }

        // Locked-database recovery prompt: the file is marked open for write
        // (usually a stale flag from an unclean exit). Clearing it is destructive
        // if another process really is using it, so it requires confirmation.
        if let Some(path) = self.pending_history_unlock.clone() {
            let mut do_clear = false;
            let mut cancel = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            Window::new("History database locked")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.label("The recording history is marked as open for write.");
                    ui.label(
                        "This usually means GeoTrace did not shut down cleanly, but another program may still have the database open.",
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Only continue if no other program is using the database - otherwise it could be corrupted.",
                        )
                        .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                RichText::new("Clear lock and open")
                                    .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                            )
                            .clicked()
                        {
                            do_clear = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if do_clear {
                self.recover_history_database(&path, ui.ctx());
                self.pending_history_unlock = None;
            } else if cancel {
                self.pending_history_unlock = None;
            }
        }

        // Corrupted-database prompt: the file exists but could not be opened.
        // Offer to recreate it, optionally keeping a backup of the original.
        if let Some(path) = self.pending_db_corruption.clone() {
            let mut do_recreate = false;
            let mut cancel = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            Window::new("History database is corrupted")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.label("The recording history database could not be opened.");
                    ui.label("You can try to recover it manually, or recreate a fresh one.");
                    ui.add_space(4.0);
                    ui.checkbox(
                        &mut self.keep_db_backup,
                        "Keep a backup of the original database",
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                RichText::new("Recreate database")
                                    .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                            )
                            .clicked()
                        {
                            do_recreate = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if do_recreate {
                self.recreate_history_database(&path, self.keep_db_backup, ui.ctx());
                self.pending_db_corruption = None;
            } else if cancel {
                self.pending_db_corruption = None;
            }
        }

        // Re-segment prompt: a recording opened from history was stored with a
        // different track-splitting setting than the current one.
        if let Some(prompt) = self.pending_resegment.take() {
            let current = loader::stored_segmentation_from_config(&self.processing_config);
            let mut recalculate = false;
            let mut use_stored = false;
            let mut cancel = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            let fmt_gap = |us: i64| format!("{} s", us / 1_000_000);
            Window::new("Track splitting differs")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    // Bound the width so a long recording name wraps this sentence
                    // instead of stretching the dialog across the screen.
                    ui.set_max_width(460.0);
                    ui.add(Label::new(format!(
                        "'{}' was stored with a different track-splitting setting than the current one.",
                        prompt.filename
                    )).wrap());
                    ui.add_space(4.0);
                    Grid::new("resegment_settings")
                        .num_columns(3)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("");
                            ui.strong("Stored");
                            ui.strong("Current");
                            ui.end_row();
                            ui.label("Split gap");
                            ui.label(fmt_gap(prompt.stored.track_split_gap_us));
                            ui.label(fmt_gap(current.track_split_gap_us));
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button("Use stored tracks")
                            .on_hover_text("Open the tracks as stored, with their previous settings")
                            .clicked()
                        {
                            use_stored = true;
                        }
                        if ui
                            .button("Recalculate with current settings")
                            .on_hover_text(
                                "Re-split the recording with the current settings, replacing the stored tracks",
                            )
                            .clicked()
                        {
                            recalculate = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if recalculate {
                self.loader.spawn_gtd_from_history(
                    prompt.bytes,
                    prompt.filename,
                    self.processing_config,
                    loader::HistoryOpen::Recalculate {
                        db_ref: prompt.db_ref,
                        applied_current_marker_settings: prompt.marker_settings_changed,
                    },
                );
                self.history_window.invalidate();
            } else if use_stored {
                let config =
                    loader::config_from_stored_segmentation(&prompt.stored, self.processing_config);
                self.loader.spawn_gtd_from_history(
                    prompt.bytes,
                    prompt.filename,
                    config,
                    loader::HistoryOpen::ApplyHidden {
                        db_ref: prompt.db_ref,
                        positions: prompt.hidden_positions,
                        applied_current_marker_settings: prompt.marker_settings_changed,
                    },
                );
            } else if !cancel {
                // No choice yet: keep the prompt open for the next frame.
                self.pending_resegment = Some(prompt);
            }
        }

        // Auto-prune confirmation dialog.
        if let Some(refs) = &self.pending_auto_prune {
            let max_gb = self.auto_prune_max_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let n = refs.len();
            let mut do_prune = false;
            let mut cancel = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            Window::new("Auto-prune")
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    // Bound the width so a long recording identity truncates instead
                    // of stretching this auto-sized dialog.
                    ui.set_max_width(460.0);
                    let rec_label = gt_fmt::pluralize(n, "recording", "recordings");
                    ui.label(format!(
                        "{n} {rec_label} will be deleted to keep storage under {max_gb:.1} GB"
                    ));
                    ui.add_space(4.0);
                    ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        for r in refs {
                            let label = format!("{}/{}", r.identity, r.group_name);
                            ui.add(Label::new(label.as_str()).truncate())
                                .on_hover_text(label.as_str());
                        }
                    });
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(
                                RichText::new("Delete these recordings")
                                    .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
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
                self.history
                    .delete_recordings(candidates, history_db::DeleteReason::AutoPrune);
            } else if cancel {
                self.pending_auto_prune = None;
            }
        }

        // Show the self-update prompt (if an in-place update was found and not
        // skipped). Package-manager/manual builds show the menu-bar badge instead.
        #[cfg(feature = "self-update")]
        if let Some(event) = self
            .update_checker
            .ui(ui.ctx(), self.skipped_version.as_deref())
        {
            match event {
                update::UpdateEvent::Skip(version) => self.skipped_version = Some(version),
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
    loader: &mut LoadJobs,
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

/// The time span of the match hovered in the query results table (first to
/// last matched point), for the plot's shaded band.
fn extract_match_hover_time_range(
    files: &[LoadedFile],
    highlight: &MapHighlight,
) -> Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> {
    let hm = highlight.hover_match?;
    let track = hm
        .track
        .fi
        .get(files)
        .and_then(|f| hm.track.index.get(&f.tracks))?;
    let time = |pi: usize| track.points.get(pi).map(|p| p.tpv.time().utc());
    Some((time(hm.start)?, time(hm.end.checked_sub(1)?)?))
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

#[cfg(test)]
#[path = "app/ui_tests.rs"]
mod tests;
