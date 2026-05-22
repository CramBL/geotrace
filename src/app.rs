mod filter_panel;
mod modals;
mod side_panel;
mod trip_data_panel;

use std::{cell::RefCell, rc::Rc};

use nav_map::{MapLayer, NavMap};
use nav_types::{GlobalFilter, LoadedFile, MapHighlight, TripDataVisibility};
use trip_data_panel::TripDataPanelState;

use modals::{
    make_log_loaded_file, show_delete_confirmation, show_mapbox_token_dialog,
    show_unassociated_popup,
};
use side_panel::{PanelContext, show_side_panel};

struct SharedAppState {
    loaded_files: Vec<LoadedFile>,
    visibility: TripDataVisibility,
    highlight: MapHighlight,
    filter: GlobalFilter,
    filter_state: filter_panel::FilterPanelState,
    panel: TripDataPanelState,
    map_center_request: Option<(f64, f64)>,
}

pub struct App {
    map: NavMap,
    shared: Rc<RefCell<SharedAppState>>,
    load_error: Option<String>,
    unassociated_log_lines: Option<Vec<String>>,
    mapbox_token_input: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_files(cc, &[])
    }

    pub fn new_with_files(cc: &eframe::CreationContext<'_>, paths: &[std::path::PathBuf]) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let stored_token = cc
            .storage
            .and_then(|s| s.get_string("mapbox_token"))
            .unwrap_or_default();
        let mapbox_token = std::env::var("MAPBOX_TOKEN")
            .or_else(|_| std::env::var("MAPBOX_ACCESS_TOKEN"))
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
            })),
            load_error: None,
            unassociated_log_lines: None,
            mapbox_token_input: String::new(),
        };

        for path in paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "nvd" {
                app.load_file(path);
            } else {
                app.try_load_log_file(path);
            }
        }

        app
    }

    fn load_file(&mut self, path: &std::path::Path) {
        match nav_io::load_file(path) {
            Ok(loaded) => {
                let mut s = self.shared.borrow_mut();
                s.loaded_files.push(loaded);
                s.visibility = TripDataVisibility::from_loaded(&s.loaded_files);
                self.load_error = None;
            }
            Err(e) => {
                log::error!("Failed to load {path:?}: {e}");
                self.load_error = Some(e.to_string());
            }
        }
    }

    fn try_load_log_file(&mut self, path: &std::path::Path) {
        let content = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                self.load_error = Some(format!("Failed to read file: {e}"));
                return;
            }
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("log");
        self.try_load_log_content(&content, name);
    }

    fn try_load_log_content(&mut self, content: &str, filename: &str) {
        let nav_points: Vec<nav_types::NavPoint> = {
            let s = self.shared.borrow();
            s.loaded_files
                .iter()
                .flat_map(|f| f.trips.iter())
                .flat_map(|t| t.points.iter())
                .cloned()
                .collect()
        };

        let result = nav_log_marker::load_log(content, &nav_points, chrono::Utc::now());

        if result.markers.is_empty() && result.unassociated.is_empty() {
            self.load_error = Some("Unrecognised file format".to_owned());
            return;
        }

        self.load_error = None;

        if let Some(loaded_file) = make_log_loaded_file(filename, result.markers) {
            let mut s = self.shared.borrow_mut();
            s.loaded_files.push(loaded_file);
            s.visibility = TripDataVisibility::from_loaded(&s.loaded_files);
        }

        if !result.unassociated.is_empty() {
            self.unassociated_log_lines = Some(result.unassociated);
        }
    }

    fn handle_dropped_bytes(&mut self, bytes: &[u8], name: &str) {
        const HDF5_MAGIC: &[u8] = b"\x89HDF\r\n\x1a\n";
        if bytes.starts_with(HDF5_MAGIC) {
            let filename = if name.is_empty() {
                "dropped.nvd".to_owned()
            } else {
                name.to_owned()
            };
            match nav_io::load_bytes(bytes, filename) {
                Ok(loaded) => {
                    let mut s = self.shared.borrow_mut();
                    s.loaded_files.push(loaded);
                    s.visibility = TripDataVisibility::from_loaded(&s.loaded_files);
                    self.load_error = None;
                }
                Err(e) => {
                    log::error!("Failed to load dropped file: {e}");
                    self.load_error = Some(e.to_string());
                }
            }
        } else if let Ok(text) = std::str::from_utf8(bytes) {
            let filename = if name.is_empty() { "dropped.log" } else { name };
            self.try_load_log_content(text, filename);
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
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = &file.path {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if ext == "nvd" {
                    self.load_file(path);
                } else {
                    self.try_load_log_file(path);
                }
            } else if let Some(bytes) = &file.bytes {
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
                    if ui.button("Open\u{2026}").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("NaView Data", &["nvd"])
                            .add_filter("Log Files", &["log", "txt"])
                            .pick_file()
                        {
                            let ext = path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            if ext == "nvd" {
                                self.load_file(&path);
                            } else {
                                self.try_load_log_file(&path);
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Map", |ui| {
                    let layer = self.map.layer();
                    if ui
                        .selectable_label(layer == MapLayer::OpenStreetMap, "OpenStreetMap")
                        .clicked()
                    {
                        self.map.set_layer(MapLayer::OpenStreetMap);
                        ui.close();
                    }
                    if ui
                        .selectable_label(layer == MapLayer::Satellite, "Satellite (Mapbox)")
                        .clicked()
                    {
                        self.map.set_layer(MapLayer::Satellite);
                        ui.close();
                    }
                });
                ui.add_space(16.0);
                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        let detached = self.shared.borrow().panel.detached;
        let viewport_id = self.shared.borrow().panel.viewport_id;
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
                        },
                    );
                });
        } else {
            let shared = Rc::clone(&self.shared);
            ui.ctx().show_viewport_immediate(
                viewport_id,
                egui::ViewportBuilder::default()
                    .with_title("Trip Data")
                    .with_inner_size([320.0, 600.0]),
                move |ui, _class| {
                    ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        let mut refmut = shared.borrow_mut();
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
                            },
                        );
                    });
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        shared.borrow_mut().panel.detached = false;
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    }
                },
            );
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let mut refmut = self.shared.borrow_mut();
            let center_req = refmut.map_center_request.take();
            let s = &mut *refmut;
            self.map.draw(
                ui,
                &s.loaded_files,
                &s.visibility,
                &mut s.highlight,
                &s.filter,
                center_req,
            );
        });

        if self.map.layer() == MapLayer::Satellite && !self.map.has_mapbox_token() {
            show_mapbox_token_dialog(ui, &mut self.map, &mut self.mapbox_token_input);
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

#[cfg(test)]
#[path = "app/ui_tests.rs"]
mod tests;
