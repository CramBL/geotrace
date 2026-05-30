mod loader;
mod modals;

const STAGE_STARTING: &str = "Starting…";
const STAGE_READING: &str = "Reading…";
const STAGE_PARSING: &str = "Parsing…";
const STAGE_PROCESSING: &str = "Processing…";
const STAGE_PLOTTING: &str = "Building plot data…";

use std::{
    cell::RefCell,
    env, fs,
    path::PathBuf,
    rc::Rc,
    str,
    sync::{Arc, mpsc},
    thread,
};

use egui_tiles::{
    Container, Linear, LinearDir, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse,
};
use gt_map::{MapContextAction, MapLayer, NavMap};
use gt_plot::PlotState;
use gt_side_panel::{FilterPanelState, PanelContext, TreeState, show_side_panel};
use gt_types::{
    DataCategory, FileIdx, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, NavPoint,
    TripDataVisibility, TripIdx,
};

use modals::{
    show_delete_confirmation, show_mapbox_token_dialog, show_orphaned_event_markers_popup,
    show_unassociated_popup,
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
    /// When `true`, the plot automatically pans to show the time range of TPV
    /// points visible in the current map viewport.
    sync_plot_to_map: bool,
}

pub struct App {
    map: NavMap,
    shared: Rc<RefCell<SharedAppState>>,
    load_error: Option<String>,
    unassociated_log_lines: Option<Vec<String>>,
    orphaned_event_markers: Option<Vec<String>>,
    mapbox_token_input: String,

    /// Egui context — cloned into background threads for `request_repaint`.
    ctx: egui::Context,
    /// Sending half of the load-result channel; cloned for each spawned thread.
    load_tx: mpsc::Sender<loader::LoadMessage>,
    /// Receiving half — drained once per frame in `ui()`.
    load_rx: mpsc::Receiver<loader::LoadMessage>,
    /// Jobs that are currently in-flight (used for the progress overlay).
    loading_jobs: Vec<loader::LoadingJob>,
    /// Jobs that have completed and are fading out of the progress overlay.
    finishing_jobs: Vec<loader::FinishedJob>,
    /// Monotonically increasing counter for assigning unique job IDs.
    next_load_id: u64,
    /// One-shot channel from a background file-picker thread.
    ///
    /// `rfd::FileDialog::pick_file()` blocks its calling thread; running it on
    /// the render thread freezes the egui loop and on Wayland the compositor
    /// stops delivering events, making the window appear unresponsive.  The
    /// dialog is therefore spawned on a dedicated thread; the chosen path (or
    /// `None` on cancellation) is sent here and consumed on the next frame.
    file_dialog_rx: Option<mpsc::Receiver<Option<PathBuf>>>,

    /// Tiles tree for the central area — map (top) and plot (bottom).
    tiles_tree: Tree<MainPane>,
    /// TileId of the map pane — used to read/write the split ratio.
    map_tile_id: TileId,
    /// TileId of the plot pane — toggled visible/invisible via the menu button.
    plot_tile_id: TileId,

    /// `true` when settings have changed since the last flush.
    config_dirty: bool,
    /// Instant of the most recent settings change; drives the debounce window.
    config_last_changed: Option<std::time::Instant>,
    /// Snapshot of settings-affecting state from the previous frame, used for
    /// change detection without instrumenting every individual change site.
    prev_snapshot: AppSnapshot,
}

/// Compact snapshot of all settings-relevant app state.
///
/// `f32` fields are stored as bit patterns (`u32`) so the struct can derive
/// `PartialEq` without triggering the `float_cmp` lint.
#[derive(PartialEq)]
struct AppSnapshot {
    show_grid: bool,
    panel_visible: bool,
    split_ratio_bits: u32,
    metric_sats_seen: bool,
    metric_sats_fix: bool,
    metric_gps_seen: bool,
    metric_gps_fix: bool,
    metric_glonass_seen: bool,
    metric_glonass_fix: bool,
    metric_galileo_seen: bool,
    metric_galileo_fix: bool,
    metric_beidou_seen: bool,
    metric_beidou_fix: bool,
    metric_velocity: bool,
    metric_eph: bool,
    metric_heading_deg: bool,
    layer: crate::settings::MapLayerSetting,
    mapbox_token: String,
    sync_to_map: bool,
    theme: crate::settings::ThemeSetting,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_files(cc, &[])
    }

    pub fn new_with_files(cc: &eframe::CreationContext<'_>, paths: &[PathBuf]) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        // Register the SVG image loaders (must come before register_marker_icons).
        egui_extras::install_image_loaders(&cc.egui_ctx);
        // Pre-register the compiled-in SVG marker icons so the texture cache
        // can serve them without any per-frame heap allocation.
        gt_map::register_marker_icons(&cc.egui_ctx);

        let mut loaded_settings = crate::settings::load_settings();

        // One-time migration: pick up mapbox_token and map_layer from the old
        // eframe storage when config.toml doesn't exist yet.
        if crate::settings::settings_path().is_none_or(|p| !p.exists()) {
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
        let (load_tx, load_rx) = mpsc::channel::<loader::LoadMessage>();

        // Build the central-area tiles tree: map on top, plot on bottom.
        // The split ratio and panel visibility are applied from settings below.
        let mut tiles: Tiles<MainPane> = Tiles::default();
        let map_tile_id = tiles.insert_pane(MainPane::Map);
        let plot_tile_id = tiles.insert_pane(MainPane::Plot);
        let root_tile_id = tiles.insert_new(Tile::Container(Container::Linear(
            Linear::new_binary(LinearDir::Vertical, [map_tile_id, plot_tile_id], 0.6),
        )));
        let tiles_tree = Tree::new("main_tiles", root_tile_id, tiles);

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
                sync_plot_to_map: true,
            })),
            load_error: None,
            unassociated_log_lines: None,
            orphaned_event_markers: None,
            mapbox_token_input: String::new(),
            ctx: cc.egui_ctx.clone(),
            load_tx,
            load_rx,
            loading_jobs: Vec::new(),
            finishing_jobs: Vec::new(),
            next_load_id: 0,
            file_dialog_rx: None,
            tiles_tree,
            map_tile_id,
            plot_tile_id,
            config_dirty: false,
            config_last_changed: None,
            prev_snapshot: AppSnapshot::default(),
        };

        app.apply_startup_settings(&loaded_settings);
        app.prev_snapshot = app.collect_snapshot();

        for path in paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "nvd" {
                app.spawn_load_nvd_path(path.clone());
            } else {
                app.spawn_load_log_path(path.clone());
            }
        }

        app
    }

    fn alloc_load_id(&mut self) -> u64 {
        let id = self.next_load_id;
        self.next_load_id += 1;
        id
    }

    /// Collect a snapshot of all currently loaded GPS points — used by log-file
    /// loaders so they can associate log timestamps with the existing GPS track.
    fn snapshot_nav_points(&self) -> Vec<NavPoint> {
        let s = self.shared.borrow();
        s.loaded_files
            .iter()
            .flat_map(|f| f.trips.iter())
            .flat_map(|t| t.points.iter())
            .cloned()
            .collect()
    }

    /// Spawn a background thread that loads a `.nvd` file from disk.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_load_nvd_path(&mut self, path: PathBuf) {
        let id = self.alloc_load_id();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();
        self.loading_jobs.push(loader::LoadingJob {
            id,
            filename: filename.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });

        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name(format!("load-{filename}"))
            .spawn(move || {
                // `report` uses separate clones so `tx`/`ctx` remain available
                // for the final `Completed` send below.
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let report = move |frac: f32, stage: &'static str| {
                    r_tx.send(loader::LoadMessage::Progress {
                        id,
                        fraction: frac,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };

                let outcome = gt_io::load_file_with_progress(&path, report)
                    .map(|file| {
                        tx.send(loader::LoadMessage::Progress {
                            id,
                            fraction: 0.95,
                            stage: STAGE_PLOTTING,
                        })
                        .ok();
                        ctx.request_repaint();
                        let series = gt_plot::prepare_file_series(0, &file);
                        loader::LoadOutcome::NvdFile { file, series }
                    })
                    .map_err(|e| e.to_string());

                tx.send(loader::LoadMessage::Completed { id, outcome }).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn nvd-path loader thread");
    }

    /// Spawn a background thread that parses `.nvd` bytes (e.g. from drag-and-drop).
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_load_nvd_bytes(&mut self, bytes: Arc<[u8]>, filename: String) {
        let id = self.alloc_load_id();
        self.loading_jobs.push(loader::LoadingJob {
            id,
            filename: filename.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });

        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name(format!("load-{filename}"))
            .spawn(move || {
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let report = move |frac: f32, stage: &'static str| {
                    r_tx.send(loader::LoadMessage::Progress {
                        id,
                        fraction: frac,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };

                let outcome = gt_io::load_bytes_with_progress(&bytes, filename, report)
                    .map(|file| {
                        tx.send(loader::LoadMessage::Progress {
                            id,
                            fraction: 0.95,
                            stage: STAGE_PLOTTING,
                        })
                        .ok();
                        ctx.request_repaint();
                        let series = gt_plot::prepare_file_series(0, &file);
                        loader::LoadOutcome::NvdFile { file, series }
                    })
                    .map_err(|e| e.to_string());

                tx.send(loader::LoadMessage::Completed { id, outcome }).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn nvd-bytes loader thread");
    }

    /// Spawn a background thread that reads and parses a log file from disk.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_load_log_path(&mut self, path: PathBuf) {
        let id = self.alloc_load_id();
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();
        self.loading_jobs.push(loader::LoadingJob {
            id,
            filename: filename.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });

        let nav_points = self.snapshot_nav_points();
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name(format!("load-log-{filename}"))
            .spawn(move || {
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let report = move |frac: f32, stage: &'static str| {
                    r_tx.send(loader::LoadMessage::Progress {
                        id,
                        fraction: frac,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };

                report(0.20, STAGE_READING);
                let content = match fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        tx.send(loader::LoadMessage::Completed {
                            id,
                            outcome: Err(format!("Failed to read {filename}: {e}")),
                        })
                        .ok();
                        ctx.request_repaint();
                        return;
                    }
                };

                finish_log_load(id, &filename, &content, &nav_points, &tx, &ctx, report);
            })
            .expect("failed to spawn log-path loader thread");
    }

    /// Spawn a background thread that parses log text from memory (e.g. drag-and-drop).
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn spawn_load_log_text(&mut self, text: String, filename: String) {
        let id = self.alloc_load_id();
        self.loading_jobs.push(loader::LoadingJob {
            id,
            filename: filename.clone(),
            progress: 0.0,
            stage: STAGE_STARTING,
            started_at: std::time::Instant::now(),
        });

        let nav_points = self.snapshot_nav_points();
        let tx = self.load_tx.clone();
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name(format!("load-log-{filename}"))
            .spawn(move || {
                let r_tx = tx.clone();
                let r_ctx = ctx.clone();
                let report = move |frac: f32, stage: &'static str| {
                    r_tx.send(loader::LoadMessage::Progress {
                        id,
                        fraction: frac,
                        stage,
                    })
                    .ok();
                    r_ctx.request_repaint();
                };
                finish_log_load(id, &filename, &text, &nav_points, &tx, &ctx, report);
            })
            .expect("failed to spawn log-text loader thread");
    }

    /// Drain all pending messages from background load threads.  Called once per frame.
    fn drain_load_channel(&mut self) {
        while let Ok(msg) = self.load_rx.try_recv() {
            match msg {
                loader::LoadMessage::Progress {
                    id,
                    fraction,
                    stage,
                } => {
                    if let Some(job) = self.loading_jobs.iter_mut().find(|j| j.id == id) {
                        job.progress = fraction;
                        job.stage = stage;
                    }
                }
                loader::LoadMessage::Completed { id, outcome } => {
                    // Capture elapsed time before removing the job so it can be
                    // shown in the fade-out overlay.
                    let elapsed_secs = self
                        .loading_jobs
                        .iter()
                        .find(|j| j.id == id)
                        .map_or(0.0, |j| j.started_at.elapsed().as_secs_f32());
                    let filename = self
                        .loading_jobs
                        .iter()
                        .find(|j| j.id == id)
                        .map(|j| j.filename.clone())
                        .unwrap_or_default();
                    self.loading_jobs.retain(|j| j.id != id);
                    match outcome {
                        Ok(loader::LoadOutcome::NvdFile { file, series }) => {
                            let orphans: Vec<String> = file
                                .orphaned_event_markers
                                .iter()
                                .map(|m| m.variant_path.clone())
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
                            self.finishing_jobs.push(loader::FinishedJob {
                                filename,
                                elapsed_secs,
                                completed_at: std::time::Instant::now(),
                            });
                        }
                        Ok(loader::LoadOutcome::LogFile {
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
                            self.finishing_jobs.push(loader::FinishedJob {
                                filename,
                                elapsed_secs,
                                completed_at: std::time::Instant::now(),
                            });
                        }
                        Err(e) => {
                            log::error!("Background load failed: {e}");
                            self.load_error = Some(e);
                        }
                    }
                }
            }
        }
    }

    /// Spawn a background thread that shows the OS file-picker dialog.
    ///
    /// `rfd::FileDialog::pick_file()` blocks its calling thread.  Running it
    /// on the render thread freezes the egui loop; on Wayland the compositor
    /// then stops delivering events, making the window appear unresponsive.
    /// The dialog runs on a dedicated thread instead; the chosen path arrives
    /// via `file_dialog_rx` and is consumed by `drain_file_dialog` each frame.
    #[expect(
        clippy::expect_used,
        reason = "thread spawn can only fail under extreme system resource exhaustion"
    )]
    fn open_file_dialog(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.file_dialog_rx = Some(rx);
        let ctx = self.ctx.clone();
        thread::Builder::new()
            .name("file-dialog".to_owned())
            .spawn(move || {
                let path = rfd::FileDialog::new()
                    .add_filter("GeoTrace Data", &["nvd"])
                    .add_filter("Log Files", &["log", "txt"])
                    .pick_file();
                tx.send(path).ok();
                ctx.request_repaint();
            })
            .expect("failed to spawn file-dialog thread");
    }

    /// Consume a pending file-picker result and dispatch the path to the
    /// appropriate loader.  Called once per frame from `ui()`.
    fn drain_file_dialog(&mut self) {
        let Some(rx) = &self.file_dialog_rx else {
            return;
        };
        let Ok(path_opt) = rx.try_recv() else {
            return;
        };
        self.file_dialog_rx = None;
        let Some(path) = path_opt else {
            return;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "nvd" {
            self.spawn_load_nvd_path(path);
        } else {
            self.spawn_load_log_path(path);
        }
    }

    fn handle_dropped_bytes(&mut self, bytes: Arc<[u8]>, name: &str) {
        const HDF5_MAGIC: &[u8] = b"\x89HDF\r\n\x1a\n";
        if bytes.starts_with(HDF5_MAGIC) {
            let filename = if name.is_empty() {
                "dropped.nvd".to_owned()
            } else {
                name.to_owned()
            };
            self.spawn_load_nvd_bytes(bytes, filename);
        } else if let Ok(text) = str::from_utf8(&bytes) {
            let filename = if name.is_empty() { "dropped.log" } else { name };
            self.spawn_load_log_text(text.to_owned(), filename.to_owned());
        } else {
            self.load_error = Some("Unrecognised file format".to_owned());
        }
    }

    /// Returns `true` when the plot tile is currently visible.
    fn plot_is_visible(&self) -> bool {
        self.tiles_tree.tiles.is_visible(self.plot_tile_id)
    }

    /// Toggle the plot tile's visibility.
    ///
    /// The tile is always kept in the tree so the GC never removes it — we
    /// just flip its visibility flag, which causes the Linear container to
    /// collapse it to zero size without removing it from the children list.
    fn toggle_plot(&mut self) {
        self.tiles_tree.tiles.toggle_visibility(self.plot_tile_id);
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

    /// Apply loaded settings on startup.
    fn apply_startup_settings(&mut self, s: &crate::settings::Settings) {
        if !s.map.mapbox_token.is_empty() {
            self.map.set_mapbox_token(s.map.mapbox_token.clone());
        }
        self.map.set_layer(map_layer_from_setting(s.map.layer));
        self.ctx.set_theme(theme_pref_from_setting(s.ui.theme));

        {
            let mut shared = self.shared.borrow_mut();
            shared.sync_plot_to_map = s.map.sync_to_map;
            shared.plot_state.show_grid = s.plot.show_grid;
            let vis = &mut shared.plot_state.metric_vis;
            vis.sats_seen = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::SatsSeen)
                .copied()
                .unwrap_or(true);
            vis.sats_fix = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::SatsFix)
                .copied()
                .unwrap_or(true);
            vis.gps_seen = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::GpsSeen)
                .copied()
                .unwrap_or(true);
            vis.gps_fix = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::GpsFix)
                .copied()
                .unwrap_or(true);
            vis.glonass_seen = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::GlonassSeen)
                .copied()
                .unwrap_or(true);
            vis.glonass_fix = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::GlonassFix)
                .copied()
                .unwrap_or(true);
            vis.galileo_seen = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::GalileoSeen)
                .copied()
                .unwrap_or(true);
            vis.galileo_fix = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::GalileoFix)
                .copied()
                .unwrap_or(true);
            vis.beidou_seen = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::BeidouSeen)
                .copied()
                .unwrap_or(true);
            vis.beidou_fix = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::BeidouFix)
                .copied()
                .unwrap_or(true);
            vis.velocity = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::Velocity)
                .copied()
                .unwrap_or(true);
            vis.eph = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::Eph)
                .copied()
                .unwrap_or(true);
            vis.heading_deg = s
                .plot
                .metric
                .get(&crate::settings::MetricKind::HeadingDeg)
                .copied()
                .unwrap_or(true);
        }

        self.tiles_tree
            .tiles
            .set_visible(self.plot_tile_id, s.plot.panel_visible);
        self.set_split_ratio(s.plot.split_ratio);
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
            split_ratio_bits: self.get_split_ratio().to_bits(),
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
            layer: map_layer_to_setting(self.map.layer()),
            mapbox_token: self.map.mapbox_token().to_owned(),
            sync_to_map: s.sync_plot_to_map,
            theme,
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
                sync_to_map: s.sync_plot_to_map,
            },
            ui: crate::settings::UiSettings { theme },
        }
    }

    fn flush_settings(&self) {
        let Some(path) = crate::settings::settings_path() else {
            log::warn!("Config directory unavailable — settings not saved");
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
        let header = "# GeoTrace configuration — generated automatically.\n\
                      # WARNING: do not commit this file to a public repository if mapbox_token is set.\n\n";
        let full_text = format!("{header}{text}");
        let tmp = path.with_extension("toml.tmp");
        if let Err(e) = std::fs::write(&tmp, full_text.as_bytes()) {
            log::warn!("Failed to write config to {tmp:?}: {e:#}");
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            log::warn!("Failed to rename config file: {e:#}");
        }
    }
}

/// Behavior implementation that renders each pane of the central tiles tree.
struct MainBehavior<'a> {
    map: &'a mut NavMap,
    state: &'a mut SharedAppState,
    map_hover_time: Option<chrono::DateTime<chrono::Utc>>,
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
                        MapContextAction::ShowOnlyTrip {
                            file_index,
                            trip_index,
                        } => {
                            s.tree
                                .show_only_trip(FileIdx(file_index), TripIdx(trip_index));
                        }
                        MapContextAction::ShowOnlyFile { file_index } => {
                            s.tree.show_only_file(FileIdx(file_index));
                        }
                    }
                }
            }
            MainPane::Plot => {
                let s = &mut *self.state;
                // Compute the time range of TPV points visible in the current
                // map viewport so the plot can pan to follow the map.
                let map_sync_x_range = if s.sync_plot_to_map {
                    self.map.viewport_geo_bounds().and_then(|b| {
                        tpv_time_range_in_bounds(&s.loaded_files, s.tree.visibility(), b)
                    })
                } else {
                    None
                };
                gt_plot::show_trip_plot(
                    ui,
                    &s.loaded_files,
                    s.tree.visibility(),
                    &s.filter,
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
        // Do not auto-prune single-child or empty containers — this keeps the
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

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Drain background load results first so newly loaded data is
        // visible in the same frame that it arrives.
        self.drain_load_channel();
        self.drain_file_dialog();

        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = &file.path {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if ext == "nvd" {
                    self.spawn_load_nvd_path(path.clone());
                } else {
                    self.spawn_load_log_path(path.clone());
                }
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
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        ui.close();
                        self.open_file_dialog();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.add_space(16.0);
                {
                    let label = format!("{} Plot", egui_phosphor::regular::CHART_LINE_UP);
                    let plot_visible = self.plot_is_visible();
                    if ui.selectable_label(plot_visible, label).clicked() {
                        self.toggle_plot();
                    }
                }
                {
                    let mut s = self.shared.borrow_mut();
                    let label = format!("{} Sync", egui_phosphor::regular::LINK);
                    ui.selectable_label(s.sync_plot_to_map, label)
                        .on_hover_text("Sync plot time range to map viewport")
                        .clicked()
                        .then(|| s.sync_plot_to_map = !s.sync_plot_to_map);
                }
                ui.add_space(16.0);
                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        let detached = self.shared.borrow().tree.detached;
        if !detached {
            egui::Panel::left("trip_data_panel")
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
                        },
                    );
                });
        } else {
            // Render the panel as a floating egui Window inside the same OS window
            // as the map. A separate OS viewport caused Wayland compositors to
            // suspend event delivery when the child was minimised or occluded,
            // freezing both windows. The floating-window approach is fully
            // platform-independent.
            let mut is_open = true;
            egui::Window::new("Trip data")
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
                        },
                    );
                });
            if !is_open {
                self.shared.borrow_mut().tree.detached = false;
            }
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mut s = self.shared.borrow_mut();
            let map_hover_time = extract_map_hover_time(&s.loaded_files, &s.highlight);

            // Render the tiles tree (map on top, optional plot on bottom).
            // Borrow tiles_tree and map explicitly so the borrow checker can see
            // they are disjoint from s (which comes from self.shared).
            {
                let map = &mut self.map;
                let tiles_tree = &mut self.tiles_tree;
                let mut behavior = MainBehavior {
                    map,
                    state: &mut s,
                    map_hover_time,
                };
                tiles_tree.ui(&mut behavior, ui);
            }

            // Forward plot hover → map highlight (must happen after the tree renders
            // so that show_trip_plot has already written the current hovered_time).
            // The pre-computed `plot_hover_point` lets TpvRenderer look up the
            // closest point in O(1) instead of re-scanning all trip points.
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
            }
        });

        if self.map.layer() == MapLayer::Satellite && !self.map.has_mapbox_token() {
            show_mapbox_token_dialog(ui, &mut self.map, &mut self.mapbox_token_input);
        }

        // Loading progress overlay — floats in the bottom-right corner.
        // Shows in-flight jobs with a live elapsed timer, and recently completed
        // jobs that fade out over ~3 seconds so the user can see how long it took.
        let any_finishing = !self.finishing_jobs.is_empty();
        // Expire jobs that have fully faded (> 3 s since completion).
        self.finishing_jobs
            .retain(|j| j.completed_at.elapsed().as_secs_f32() < 3.0);

        if !self.loading_jobs.is_empty() || any_finishing {
            // Keep repainting while jobs are active or fading.
            ui.ctx().request_repaint();

            egui::Window::new("##loading_progress")
                .title_bar(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(260.0);

                    for job in &self.loading_jobs {
                        let elapsed = job.started_at.elapsed().as_secs_f32();
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(egui::RichText::new(&job.filename).strong().small());
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

                    for job in &self.finishing_jobs {
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
                            ui.label(
                                egui::RichText::new(&job.filename)
                                    .color(color)
                                    .strong()
                                    .small(),
                            );
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
                        egui::Color32::from_rgb(220, 70, 50),
                        format!("{} {error}", egui_phosphor::regular::WARNING),
                    );
                    dismiss = ui.small_button(egui_phosphor::regular::X).clicked();
                });
            }
        });
        if dismiss {
            self.load_error = None;
        }

        {
            let mut refmut = self.shared.borrow_mut();
            let s = &mut *refmut;
            if show_delete_confirmation(ui, &mut s.tree, &mut s.loaded_files) {
                s.plot_state.rebuild_all(&s.loaded_files);
            }
        }

        show_unassociated_popup(ui, &mut self.unassociated_log_lines);
        show_orphaned_event_markers_popup(ui, &mut self.orphaned_event_markers);

        // Detect settings changes and trigger a debounced write-through.
        let snapshot = self.collect_snapshot();
        if snapshot != self.prev_snapshot {
            self.prev_snapshot = snapshot;
            self.config_dirty = true;
            self.config_last_changed = Some(std::time::Instant::now());
        }
        const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);
        if self.config_dirty
            && self
                .config_last_changed
                .is_some_and(|t| t.elapsed() >= DEBOUNCE)
        {
            self.flush_settings();
            self.config_dirty = false;
        }
    }
}

/// Find the Unix-second time range of TPV points that lie within the given map
/// geographic bounds, considering only files/trips currently enabled in `visibility`.
///
/// Returns `None` when no visible TPV points fall in the viewport.
fn tpv_time_range_in_bounds(
    files: &[LoadedFile],
    visibility: &TripDataVisibility,
    bounds: gt_map::GeoBounds,
) -> Option<(f64, f64)> {
    use uom::si::angle::degree;
    let mut t_min = f64::INFINITY;
    let mut t_max = f64::NEG_INFINITY;

    for (fi, file) in files.iter().enumerate() {
        let Some(fv) = visibility.files.get(fi) else {
            continue;
        };
        if !fv.enabled {
            continue;
        }
        for (ti, trip) in file.trips.iter().enumerate() {
            let Some(tv) = fv.trips.get(ti) else {
                continue;
            };
            if !tv.enabled {
                continue;
            }
            for point in &trip.points {
                let lat = point.tpv.lat().get::<degree>();
                let lon = point.tpv.lon().get::<degree>();
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
    files
        .get(point_ref.file_index.0)
        .and_then(|f| f.trips.get(point_ref.trip_index.0))
        .and_then(|t| t.points.get(point_ref.point_index.0))
        .map(|p| p.tpv.time().utc())
}

/// Shared tail of log-file loading: parse `content`, build a `LoadedFile`, and
/// send the `Completed` message.  Called from both the path-based and bytes-based
/// log loader threads after file content has been obtained.
fn finish_log_load(
    id: u64,
    filename: &str,
    content: &str,
    nav_points: &[NavPoint],
    tx: &mpsc::Sender<loader::LoadMessage>,
    ctx: &egui::Context,
    report: impl Fn(f32, &'static str),
) {
    report(0.55, STAGE_PARSING);
    let result = gt_log_marker::load_log(content, nav_points, chrono::Utc::now());

    if result.markers.is_empty() && result.unassociated.is_empty() {
        tx.send(loader::LoadMessage::Completed {
            id,
            outcome: Err("Unrecognised file format".to_owned()),
        })
        .ok();
        ctx.request_repaint();
        return;
    }

    report(0.90, STAGE_PROCESSING);
    let loaded = loader::build_log_loaded_file(filename, result.markers);
    report(0.95, STAGE_PLOTTING);
    let series = loaded.as_ref().map(|f| gt_plot::prepare_file_series(0, f));

    tx.send(loader::LoadMessage::Completed {
        id,
        outcome: Ok(loader::LoadOutcome::LogFile {
            loaded,
            series,
            unassociated: result.unassociated,
        }),
    })
    .ok();
    ctx.request_repaint();
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

impl Default for AppSnapshot {
    fn default() -> Self {
        Self {
            show_grid: true,
            panel_visible: true,
            split_ratio_bits: 0.6_f32.to_bits(),
            metric_sats_seen: true,
            metric_sats_fix: true,
            metric_gps_seen: true,
            metric_gps_fix: true,
            metric_glonass_seen: true,
            metric_glonass_fix: true,
            metric_galileo_seen: true,
            metric_galileo_fix: true,
            metric_beidou_seen: true,
            metric_beidou_fix: true,
            metric_velocity: true,
            metric_eph: true,
            metric_heading_deg: true,
            layer: crate::settings::MapLayerSetting::Osm,
            mapbox_token: String::new(),
            sync_to_map: true,
            theme: crate::settings::ThemeSetting::System,
        }
    }
}

#[cfg(test)]
#[path = "app/ui_tests.rs"]
mod tests;
