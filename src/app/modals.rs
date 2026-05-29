use std::collections::{BTreeMap, BTreeSet};

use gt_map::{MapLayer, NavMap};
use gt_types::{LoadedFile, TripDataVisibility};

use super::trip_data_panel::{SelectionKey, TripDataPanelState, TripRef};

/// Show the delete-confirmation dialog.
///
/// Returns `true` in the one frame when items were actually deleted so the
/// caller can rebuild any caches that depend on file indices.
pub fn show_delete_confirmation(
    ui: &egui::Ui,
    panel: &mut TripDataPanelState,
    loaded_files: &mut Vec<LoadedFile>,
    visibility: &mut TripDataVisibility,
) -> bool {
    let Some(confirm) = &panel.delete_confirm else {
        return false;
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
        .resizable(true)
        .min_width(360.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(500.0)
                .show(ui, |ui| {
                    let items: Vec<_> = panel
                        .delete_confirm
                        .as_ref()
                        .map(|c| c.items.clone())
                        .unwrap_or_default();
                    for key in &items {
                        match key {
                            SelectionKey::File(fi) => {
                                if let Some(file) = loaded_files.get(fi.0) {
                                    ui.label(&file.metadata.filename);
                                }
                            }
                            SelectionKey::Trip(TripRef { file: fi, trip: ti }) => {
                                if let Some(file) = loaded_files.get(fi.0)
                                    && let Some(trip) = file.trips.get(ti.0)
                                {
                                    let dist = gt_fmt::format_distance(trip.metadata.distance_km);
                                    let dur =
                                        gt_fmt::format_human_terse_duration(trip.metadata.duration);
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
        return false;
    } else if do_delete {
        let items = panel
            .delete_confirm
            .take()
            .map(|c| c.items)
            .unwrap_or_default();
        execute_delete(&items, loaded_files, visibility, panel);
        return true;
    }
    false
}

pub fn execute_delete(
    keys: &[SelectionKey],
    loaded_files: &mut Vec<LoadedFile>,
    visibility: &mut TripDataVisibility,
    panel: &mut TripDataPanelState,
) {
    let mut file_indices_to_remove: BTreeSet<usize> = BTreeSet::new();
    let mut trips_to_remove: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();

    for key in keys {
        match key {
            SelectionKey::File(fi) => {
                file_indices_to_remove.insert(fi.0);
            }
            SelectionKey::Trip(TripRef { file: fi, trip: ti }) => {
                trips_to_remove.entry(fi.0).or_default().insert(ti.0);
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

pub fn show_orphaned_event_markers_popup(ui: &egui::Ui, markers: &mut Option<Vec<String>>) {
    let Some(orphans) = markers else {
        return;
    };
    let mut dismiss = false;
    egui::Window::new("Event markers outside trip range")
        .collapsible(false)
        .resizable(true)
        .show(ui.ctx(), |ui| {
            ui.label("These event markers could not be assigned to any trip:");
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for path in orphans.iter() {
                        ui.monospace(path);
                    }
                });
            if ui.button("Dismiss").clicked() {
                dismiss = true;
            }
        });
    if dismiss {
        *markers = None;
    }
}

pub fn show_mapbox_token_dialog(ui: &egui::Ui, map: &mut NavMap, token_input: &mut String) {
    // ESC dismisses the dialog — same effect as the cancel button.
    let esc_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if esc_pressed {
        map.set_layer(MapLayer::OpenStreetMap);
        token_input.clear();
        return;
    }

    let mut open = true;
    egui::Window::new("Mapbox API Token Required")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label("Satellite view requires a Mapbox API token");
            ui.label("Get one free at mapbox.com");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Token");
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

    // X button in the title bar was clicked — treat as cancel.
    if !open {
        map.set_layer(MapLayer::OpenStreetMap);
        token_input.clear();
    }
}
