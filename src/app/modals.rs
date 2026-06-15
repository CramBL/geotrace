use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use gt_map::{MapLayer, NavMap};
use gt_side_panel::{NodeKey, TreeState};
use gt_types::{LoadWarning, LoadedFile, TrackRef};
use gt_ui_theme::WARNING_AMBER;

/// Show the delete-confirmation dialog.
///
/// Returns `true` in the one frame when items were actually deleted so the
/// caller can rebuild any caches that depend on file indices.
pub fn show_delete_confirmation(
    ui: &egui::Ui,
    tree: &mut TreeState,
    loaded_files: &mut Vec<LoadedFile>,
) -> bool {
    let Some(confirm) = &tree.delete_confirm else {
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

    let item_label = if count == 1 { "item" } else { "items" };
    egui::Window::new(format!("Remove {count} {item_label}?"))
        .collapsible(false)
        .resizable(true)
        .min_width(420.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(500.0)
                .show(ui, |ui| {
                    let items: Vec<_> = tree
                        .delete_confirm
                        .as_ref()
                        .map(|c| c.items.clone())
                        .unwrap_or_default();
                    for key in &items {
                        match key {
                            NodeKey::File(fi) => {
                                if let Some(file) = fi.get(loaded_files) {
                                    ui.label(&file.metadata.filename);
                                }
                            }
                            NodeKey::Track(TrackRef { fi, index: ti }) => {
                                if let Some(file) = fi.get(loaded_files)
                                    && let Some(track) = ti.get(&file.tracks)
                                {
                                    let dist = gt_fmt::format_distance(track.metadata.distance_km);
                                    let dur = gt_fmt::format_human_terse_duration(
                                        track.metadata.duration,
                                    );
                                    ui.label(format!(
                                        "  {} / #{}  {dist}  {dur}",
                                        file.metadata.filename, track.metadata.index
                                    ));
                                }
                            }
                        }
                    }
                });
            ui.separator();
            ui.label(
                egui::RichText::new("This only removes them from the current view.")
                    .weak()
                    .small(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    do_cancel = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Remove").clicked() {
                        do_delete = true;
                    }
                });
            });
        });

    if do_cancel {
        tree.delete_confirm = None;
        return false;
    } else if do_delete {
        let items = tree
            .delete_confirm
            .take()
            .map(|c| c.items)
            .unwrap_or_default();
        execute_delete(&items, loaded_files, tree);
        return true;
    }
    false
}

pub fn execute_delete(keys: &[NodeKey], loaded_files: &mut Vec<LoadedFile>, tree: &mut TreeState) {
    let mut file_indices_to_remove: BTreeSet<usize> = BTreeSet::new();
    let mut trips_to_remove: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();

    for key in keys {
        match key {
            NodeKey::File(fi) => {
                file_indices_to_remove.insert(fi.as_usize());
            }
            NodeKey::Track(TrackRef { fi, index: ti }) => {
                trips_to_remove
                    .entry(fi.as_usize())
                    .or_default()
                    .insert(ti.as_usize());
            }
        }
    }

    for (fi, trip_set) in &trips_to_remove {
        if file_indices_to_remove.contains(fi) {
            continue;
        }
        if let Some(file) = loaded_files.get_mut(*fi) {
            for ti in (0..file.tracks.len()).rev() {
                if trip_set.contains(&ti) {
                    file.tracks.remove(ti);
                }
            }
            if file.tracks.is_empty() {
                file_indices_to_remove.insert(*fi);
            }
        }
    }

    for fi in (0..loaded_files.len()).rev() {
        if file_indices_to_remove.contains(&fi) {
            loaded_files.remove(fi);
        }
    }

    tree.reset_for_files(loaded_files);
}

pub fn show_unassociated_popup(ui: &egui::Ui, lines: &mut Option<Vec<(DateTime<Utc>, String)>>) {
    let Some(unassociated) = lines else {
        return;
    };
    let count = unassociated.len();
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut dismiss = escape_pressed;
    egui::Window::new(format!("{count} log entries could not be associated"))
        .collapsible(false)
        .resizable(true)
        .min_width(480.0)
        .show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    let ten_min = chrono::Duration::minutes(10);
                    let mut prev_ts: Option<DateTime<Utc>> = None;
                    for (ts, line) in unassociated.iter() {
                        if let Some(prev) = prev_ts
                            && ts.signed_duration_since(prev) > ten_min
                        {
                            ui.separator();
                        }
                        ui.monospace(format!("{}  {}", ts.format("%Y-%m-%d %H:%M:%S"), line));
                        prev_ts = Some(*ts);
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

pub fn show_orphaned_event_markers_popup(
    ui: &egui::Ui,
    markers: &mut Option<Vec<(DateTime<Utc>, String)>>,
) {
    let Some(orphans) = markers else {
        return;
    };
    let count = orphans.len();
    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut dismiss = enter_pressed || escape_pressed;
    egui::Window::new(format!("{count} event markers outside track range"))
        .collapsible(false)
        .resizable(true)
        .min_width(480.0)
        .show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    let ten_min = chrono::Duration::minutes(10);
                    let mut prev_ts: Option<DateTime<Utc>> = None;
                    for (ts, path) in orphans.iter() {
                        if let Some(prev) = prev_ts
                            && ts.signed_duration_since(prev) > ten_min
                        {
                            ui.separator();
                        }
                        ui.monospace(format!("{}  {}", ts.format("%Y-%m-%d %H:%M:%S"), path));
                        prev_ts = Some(*ts);
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

pub fn show_load_warnings_dialog(ui: &egui::Ui, popup: &mut Option<(String, Vec<LoadWarning>)>) {
    let Some((filename, warnings)) = popup else {
        return;
    };
    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut dismiss = enter_pressed || escape_pressed;

    egui::Window::new("Data quality warnings")
        .collapsible(false)
        .resizable(true)
        .min_width(540.0)
        .show(ui.ctx(), |ui| {
            ui.label(egui::RichText::new(filename.as_str()).strong());
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(400.0)
                .show(ui, |ui| {
                    egui::Grid::new("load_warnings_grid")
                        .num_columns(4)
                        .striped(true)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            for w in warnings.iter() {
                                ui.label(
                                    egui::RichText::new(egui_phosphor::regular::WARNING)
                                        .color(WARNING_AMBER),
                                );
                                ui.label(egui::RichText::new(w.count.to_string()).strong());
                                ui.label(&w.issue);
                                ui.add(egui::Label::new(&w.description).wrap());
                                ui.end_row();
                            }
                        });
                });
            ui.separator();
            if ui.button("Dismiss").clicked() {
                dismiss = true;
            }
        });

    if dismiss {
        *popup = None;
    }
}

pub fn show_mapbox_token_dialog(ui: &egui::Ui, map: &mut NavMap, token_input: &mut String) {
    // ESC dismisses the dialog - same effect as the cancel button.
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
            if ui.button("Cancel - use OpenStreetMap").clicked() {
                map.set_layer(MapLayer::OpenStreetMap);
                token_input.clear();
            }
        });

    // X button in the title bar was clicked - treat as cancel.
    if !open {
        map.set_layer(MapLayer::OpenStreetMap);
        token_input.clear();
    }
}
