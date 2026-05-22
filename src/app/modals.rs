use nav_map::{MapLayer, NavMap};
use nav_types::{
    Coord, CustomMarker, FileMetadata, LoadedFile, LoadedTrip, Rect, TripDataVisibility,
    TripMetadata,
};
use uom::si::angle::degree;

use super::trip_data_panel::{SelectionKey, TripDataPanelState};

pub fn show_delete_confirmation(
    ui: &egui::Ui,
    panel: &mut TripDataPanelState,
    loaded_files: &mut Vec<LoadedFile>,
    visibility: &mut TripDataVisibility,
) {
    let Some(confirm) = &panel.delete_confirm else {
        return;
    };
    let count = confirm.items.len();
    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let mut do_delete = enter_pressed;
    let mut do_cancel = escape_pressed;

    egui::Window::new(format!("Delete {count} item(s)?"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    let items: Vec<_> = panel
                        .delete_confirm
                        .as_ref()
                        .map(|c| c.items.clone())
                        .unwrap_or_default();
                    for key in &items {
                        match key {
                            SelectionKey::File(fi) => {
                                if let Some(file) = loaded_files.get(*fi) {
                                    ui.label(&file.metadata.filename);
                                }
                            }
                            SelectionKey::Trip(fi, ti) => {
                                if let Some(file) = loaded_files.get(*fi)
                                    && let Some(trip) = file.trips.get(*ti)
                                {
                                    let dist = nav_fmt::format_distance(trip.metadata.distance_km);
                                    let dur = nav_fmt::format_human_terse_duration(
                                        trip.metadata.duration,
                                    );
                                    ui.label(format!(
                                        "  {} / T{}  {dist}  {dur}",
                                        file.metadata.filename, trip.metadata.index
                                    ));
                                }
                            }
                        }
                    }
                });
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    do_cancel = true;
                }
                if ui.button("Delete").clicked() {
                    do_delete = true;
                }
            });
        });

    if do_cancel {
        panel.delete_confirm = None;
    } else if do_delete {
        let items = panel
            .delete_confirm
            .take()
            .map(|c| c.items)
            .unwrap_or_default();
        execute_delete(&items, loaded_files, visibility, panel);
    }
}

pub fn execute_delete(
    keys: &[SelectionKey],
    loaded_files: &mut Vec<LoadedFile>,
    visibility: &mut TripDataVisibility,
    panel: &mut TripDataPanelState,
) {
    let mut file_indices_to_remove: std::collections::BTreeSet<usize> =
        std::collections::BTreeSet::new();
    let mut trips_to_remove: std::collections::BTreeMap<usize, std::collections::BTreeSet<usize>> =
        std::collections::BTreeMap::new();

    for key in keys {
        match key {
            SelectionKey::File(fi) => {
                file_indices_to_remove.insert(*fi);
            }
            SelectionKey::Trip(fi, ti) => {
                trips_to_remove.entry(*fi).or_default().insert(*ti);
            }
        }
    }

    for (fi, trip_set) in &trips_to_remove {
        if file_indices_to_remove.contains(fi) {
            continue;
        }
        if let Some(file) = loaded_files.get_mut(*fi) {
            let mut ti = file.trips.len();
            while ti > 0 {
                ti -= 1;
                if trip_set.contains(&ti) {
                    file.trips.remove(ti);
                }
            }
            if file.trips.is_empty() {
                file_indices_to_remove.insert(*fi);
            }
        }
    }

    let mut fi = loaded_files.len();
    while fi > 0 {
        fi -= 1;
        if file_indices_to_remove.contains(&fi) {
            loaded_files.remove(fi);
        }
    }

    *visibility = TripDataVisibility::from_loaded(loaded_files);
    panel.selection.clear();
    panel.delete_confirm = None;
}

pub fn show_unassociated_popup(ui: &egui::Ui, lines: &mut Option<Vec<String>>) {
    let Some(unassociated) = lines else {
        return;
    };
    let mut dismiss = false;
    egui::Window::new("Log entries could not be associated")
        .collapsible(false)
        .resizable(true)
        .show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for line in unassociated.iter() {
                        ui.monospace(line);
                    }
                });
            if ui.button("Dismiss").clicked() {
                dismiss = true;
            }
        });
    if dismiss {
        *lines = None;
    }
}

pub fn show_mapbox_token_dialog(ui: &egui::Ui, map: &mut NavMap, token_input: &mut String) {
    egui::Window::new("Mapbox API Token Required")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label("Satellite view requires a Mapbox API token.");
            ui.label("Get one free at mapbox.com.");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Token:");
                let response = ui.text_edit_singleline(token_input);
                let submitted =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (submitted || ui.button("Apply").clicked()) && !token_input.is_empty() {
                    map.set_mapbox_token(std::mem::take(token_input));
                }
            });
            if ui.button("Cancel — use OpenStreetMap").clicked() {
                map.set_layer(MapLayer::OpenStreetMap);
                token_input.clear();
            }
        });
}

pub fn make_log_loaded_file(filename: &str, markers: Vec<CustomMarker>) -> Option<LoadedFile> {
    let first = markers.first()?;

    let mut min_lat = first.lat.get::<degree>();
    let mut max_lat = min_lat;
    let mut min_lon = first.lon.get::<degree>();
    let mut max_lon = min_lon;
    let mut min_time = first.time;
    let mut max_time = first.time;

    for m in &markers {
        let lat = m.lat.get::<degree>();
        let lon = m.lon.get::<degree>();
        if lat < min_lat {
            min_lat = lat;
        }
        if lat > max_lat {
            max_lat = lat;
        }
        if lon < min_lon {
            min_lon = lon;
        }
        if lon > max_lon {
            max_lon = lon;
        }
        if m.time < min_time {
            min_time = m.time;
        }
        if m.time > max_time {
            max_time = m.time;
        }
    }

    let count = markers.len();
    let duration = max_time - min_time;
    let filename = if filename.is_empty() {
        "log".to_owned()
    } else {
        filename.to_owned()
    };

    let trip = LoadedTrip {
        metadata: TripMetadata {
            index: 0,
            distance_km: 0.0,
            duration,
            time_range: (min_time, max_time),
            bounding_box: Rect::new(
                Coord {
                    x: min_lon,
                    y: min_lat,
                },
                Coord {
                    x: max_lon,
                    y: max_lat,
                },
            ),
            point_set_diameter_m: 0.0,
            has_custom_markers: true,
            tpv_count: 0,
            satellite_report_count: 0,
            custom_marker_count: count,
            generated_marker_count: 0,
        },
        points: Vec::new(),
        custom_markers: markers,
        generated_markers: Vec::new(),
    };

    Some(LoadedFile {
        metadata: FileMetadata {
            filename,
            total_distance_km: 0.0,
            total_duration: duration,
            time_range: (min_time, max_time),
        },
        trips: vec![trip],
    })
}
