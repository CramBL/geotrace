mod filter_panel;
mod loader;
mod modals;
mod side_panel;
mod trip_data_panel;

const STAGE_STARTING: &str = "Starting…";
const STAGE_READING: &str = "Reading…";
const STAGE_PARSING: &str = "Parsing…";
const STAGE_PROCESSING: &str = "Processing…";

use std::{
    cell::RefCell,
    env, fs,
    path::PathBuf,
    rc::Rc,
    str,
    sync::{Arc, mpsc},
    thread,
};

use nav_map::{MapLayer, NavMap};
use nav_types::{GlobalFilter, LoadedFile, MapHighlight, NavPoint, TripDataVisibility};
use trip_data_panel::TripDataPanelState;

use modals::{show_delete_confirmation, show_mapbox_token_dialog, show_unassociated_popup};
use side_panel::{PanelContext, show_side_panel};

struct SharedAppState {
    loaded_files: Vec<LoadedFile>,
    visibility: TripDataVisibility,
    highlight: MapHighlight,
    filter: GlobalFilter,
    filter_state: filter_panel::FilterPanelState,
    panel: TripDataPanelState,
    map_center_request: Option<(f64, f64)>,
    /// Requested screen position for the next sticky info popup, set by panel
    /// item clicks and consumed by `NavMap::draw` as the popup's default position.
    popup_pos_request: Option<egui::Pos2>,
    /// When `true`, `NavMap::draw` zooms the map to fit all currently visible data.
    zoom_to_visible_request: bool,
}

pub struct App {
    map: NavMap,
    shared: Rc<RefCell<SharedAppState>>,
    load_error: Option<String>,
    unassociated_log_lines: Option<Vec<String>>,
    mapbox_token_input: String,

    /// Egui context — cloned into background threads for `request_repaint`.
    ctx: egui::Context,
    /// Sending half of the load-result channel; cloned for each spawned thread.
    load_tx: mpsc::Sender<loader::LoadMessage>,
    /// Receiving half — drained once per frame in `ui()`.
    load_rx: mpsc::Receiver<loader::LoadMessage>,
    /// Jobs that are currently in-flight (used for the progress overlay).
    loading_jobs: Vec<loader::LoadingJob>,
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
        nav_map::register_marker_icons(&cc.egui_ctx);

        let stored_token = cc
            .storage
            .and_then(|s| s.get_string("mapbox_token"))
            .unwrap_or_default();
        let mapbox_token = env::var("MAPBOX_TOKEN")
            .or_else(|_| env::var("MAPBOX_ACCESS_TOKEN"))
            .unwrap_or(stored_token);

        let map_layer = cc
            .storage
            .and_then(|s| s.get_string("map_layer"))
            .map(|s| {
                if s == "satellite" {
                    MapLayer::Satellite
                } else {
                    MapLayer::OpenStreetMap
                }
            })
            .unwrap_or_default();

        let mut map = NavMap::new(cc.egui_ctx.clone());
        if !mapbox_token.is_empty() {
            map.set_mapbox_token(mapbox_token);
        }
        map.set_layer(map_layer);

        let (load_tx, load_rx) = mpsc::channel::<loader::LoadMessage>();

        let mut app = Self {
            map,
            shared: Rc::new(RefCell::new(SharedAppState {
                loaded_files: Vec::new(),
                visibility: TripDataVisibility { files: Vec::new() },
                highlight: MapHighlight::default(),
                filter: GlobalFilter::default(),
                filter_state: filter_panel::FilterPanelState::default(),
                panel: TripDataPanelState::new(),
                map_center_request: None,
                popup_pos_request: None,
                zoom_to_visible_request: false,
            })),
            load_error: None,
            unassociated_log_lines: None,
            mapbox_token_input: String::new(),
            ctx: cc.egui_ctx.clone(),
            load_tx,
            load_rx,
            loading_jobs: Vec::new(),
            next_load_id: 0,
            file_dialog_rx: None,
        };

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

                let outcome = nav_io::load_file_with_progress(&path, report)
                    .map(loader::LoadOutcome::NvdFile)
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

                let outcome = nav_io::load_bytes_with_progress(&bytes, filename, report)
                    .map(loader::LoadOutcome::NvdFile)
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

    /// Drain all pending messages from background load threads. Called once per frame.
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
                    self.loading_jobs.retain(|j| j.id != id);
                    match outcome {
                        Ok(loader::LoadOutcome::NvdFile(loaded)) => {
                            let mut s = self.shared.borrow_mut();
                            s.loaded_files.push(loaded);
                            s.visibility = TripDataVisibility::from_loaded(&s.loaded_files);
                            self.load_error = None;
                        }
                        Ok(loader::LoadOutcome::LogFile {
                            loaded,
                            unassociated,
                        }) => {
                            if let Some(loaded) = loaded {
                                let mut s = self.shared.borrow_mut();
                                s.loaded_files.push(loaded);
                                s.visibility = TripDataVisibility::from_loaded(&s.loaded_files);
                            }
                            if !unassociated.is_empty() {
                                self.unassociated_log_lines = Some(unassociated);
                            }
                            self.load_error = None;
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
                    .add_filter("NaView Data", &["nvd"])
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
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let token = self.map.mapbox_token();
        if !token.is_empty() {
            storage.set_string("mapbox_token", token.to_owned());
        }
        let layer_str = match self.map.layer() {
            MapLayer::OpenStreetMap => "osm",
            MapLayer::Satellite => "satellite",
        };
        storage.set_string("map_layer", layer_str.to_owned());
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
            if delete_pressed && !s.panel.selection.is_empty() && s.panel.delete_confirm.is_none() {
                let items = s.panel.selection.iter().cloned().collect();
                s.panel.delete_confirm = Some(trip_data_panel::DeleteConfirmState { items });
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
                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        let detached = self.shared.borrow().panel.detached;
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
                            visibility: &mut s.visibility,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            panel: &mut s.panel,
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
            egui::Window::new("Trip Data")
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
                            visibility: &mut s.visibility,
                            highlight: &mut s.highlight,
                            filter: &mut s.filter,
                            filter_state: &mut s.filter_state,
                            panel: &mut s.panel,
                            map_center_request: &mut s.map_center_request,
                            popup_pos_request: &mut s.popup_pos_request,
                            zoom_to_visible_request: &mut s.zoom_to_visible_request,
                        },
                    );
                });
            if !is_open {
                self.shared.borrow_mut().panel.detached = false;
            }
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mut refmut = self.shared.borrow_mut();
            let center_req = refmut.map_center_request.take();
            let popup_pos = refmut.popup_pos_request.take();
            let zoom_to_visible = std::mem::replace(&mut refmut.zoom_to_visible_request, false);
            let s = &mut *refmut;
            self.map.draw(
                ui,
                &s.loaded_files,
                &s.visibility,
                &mut s.highlight,
                &s.filter,
                center_req,
                zoom_to_visible,
                popup_pos,
            );
        });

        if self.map.layer() == MapLayer::Satellite && !self.map.has_mapbox_token() {
            show_mapbox_token_dialog(ui, &mut self.map, &mut self.mapbox_token_input);
        }

        // Loading progress overlay — floats in the bottom-right corner and shows
        // one progress bar per in-flight background load job.
        if !self.loading_jobs.is_empty() {
            egui::Window::new("##loading_progress")
                .title_bar(false)
                .resizable(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -8.0])
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(260.0);
                    for job in &self.loading_jobs {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(egui::RichText::new(&job.filename).strong().small());
                        });
                        ui.add(
                            egui::ProgressBar::new(job.progress)
                                .animate(true)
                                .desired_width(240.0)
                                .text(job.stage),
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
            show_delete_confirmation(ui, &mut s.panel, &mut s.loaded_files, &mut s.visibility);
        }

        show_unassociated_popup(ui, &mut self.unassociated_log_lines);
    }
}

/// Shared tail of log-file loading: parse `content`, build a `LoadedFile`, and
/// send the `Completed` message. Called from both the path-based and bytes-based
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
    let result = nav_log_marker::load_log(content, nav_points, chrono::Utc::now());

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

    tx.send(loader::LoadMessage::Completed {
        id,
        outcome: Ok(loader::LoadOutcome::LogFile {
            loaded,
            unassociated: result.unassociated,
        }),
    })
    .ok();
    ctx.request_repaint();
}

#[cfg(test)]
#[path = "app/ui_tests.rs"]
mod tests;
