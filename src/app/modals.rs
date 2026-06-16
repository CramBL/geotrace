use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use gt_map::{MapLayer, NavMap};
use gt_side_panel::{NodeKey, TreeState};
use gt_types::{LoadWarning, LoadedFile, TrackRef};
use gt_ui_theme::WARNING_AMBER;

/// What the remove-confirmation dialog asks the app to do, in the one frame
/// after the user confirms.
pub struct RemoveOutcome {
    /// Recordings to act on in history - one per fully-removed file that exists
    /// in the history database. Empty when nothing removed was stored.
    pub affected: Vec<gt_types::DatabaseRef>,
    /// `true` to permanently delete the affected recordings; `false` to hide them.
    pub permanent: bool,
}

/// Show the delete-confirmation dialog.
///
/// Returns `Some` in the one frame when items were actually removed, so the
/// caller can rebuild caches that depend on file indices and apply the chosen
/// history operation (hide or permanent delete) to `affected`.
pub fn show_delete_confirmation(
    ui: &egui::Ui,
    tree: &mut TreeState,
    loaded_files: &mut Vec<LoadedFile>,
) -> Option<RemoveOutcome> {
    let Some(confirm) = &tree.delete_confirm else {
        return None;
    };
    let count = confirm.items.len();
    let mut permanent = confirm.delete_permanently;
    // How many of the to-be-removed files are stored in history; this drives the
    // wording and whether the "delete permanently" option is even relevant.
    let history_affected = affected_recordings(&confirm.items, loaded_files).len();

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
            if history_affected == 0 {
                ui.label(
                    egui::RichText::new("This only removes them from the current view.")
                        .weak()
                        .small(),
                );
            } else {
                ui.checkbox(&mut permanent, "Also delete permanently from history");
                let rec_label = gt_fmt::pluralize(history_affected, "recording", "recordings");
                let detail = if permanent {
                    format!(
                        "Removes them from the view and permanently deletes {history_affected} {rec_label} from history."
                    )
                } else {
                    format!(
                        "Removes them from the view and hides {history_affected} {rec_label} in history."
                    )
                };
                ui.label(egui::RichText::new(detail).weak().small());
            }
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
        return None;
    }
    if do_delete {
        let items = tree
            .delete_confirm
            .take()
            .map(|c| c.items)
            .unwrap_or_default();
        let affected = execute_delete(&items, loaded_files, tree);
        return Some(RemoveOutcome {
            affected,
            permanent,
        });
    }
    // Keep the checkbox state across frames while the dialog stays open.
    if let Some(c) = tree.delete_confirm.as_mut() {
        c.delete_permanently = permanent;
    }
    None
}

/// Indices of files that removing `keys` would empty entirely - either selected
/// directly, or because every one of their tracks is in the removal set.
fn files_fully_removed(keys: &[NodeKey], loaded_files: &[LoadedFile]) -> BTreeSet<usize> {
    let mut files: BTreeSet<usize> = BTreeSet::new();
    let mut tracks: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for key in keys {
        match key {
            NodeKey::File(fi) => {
                files.insert(fi.as_usize());
            }
            NodeKey::Track(TrackRef { fi, index: ti }) => {
                tracks
                    .entry(fi.as_usize())
                    .or_default()
                    .insert(ti.as_usize());
            }
        }
    }
    for (fi, track_set) in &tracks {
        if files.contains(fi) {
            continue;
        }
        if let Some(file) = loaded_files.get(*fi)
            && !file.tracks.is_empty()
            && (0..file.tracks.len()).all(|ti| track_set.contains(&ti))
        {
            files.insert(*fi);
        }
    }
    files
}

/// History references for the files that removing `keys` would empty entirely.
fn affected_recordings(
    keys: &[NodeKey],
    loaded_files: &[LoadedFile],
) -> Vec<gt_types::DatabaseRef> {
    files_fully_removed(keys, loaded_files)
        .iter()
        .filter_map(|fi| loaded_files.get(*fi).and_then(|f| f.db_ref.clone()))
        .collect()
}

/// Remove `keys` from the view and return the history references of the files
/// that were removed entirely (so the caller can hide or delete them).
pub fn execute_delete(
    keys: &[NodeKey],
    loaded_files: &mut Vec<LoadedFile>,
    tree: &mut TreeState,
) -> Vec<gt_types::DatabaseRef> {
    let fully_removed = files_fully_removed(keys, loaded_files);
    // Derived from the set we already computed, rather than recomputing it.
    let affected: Vec<gt_types::DatabaseRef> = fully_removed
        .iter()
        .filter_map(|fi| loaded_files.get(*fi).and_then(|f| f.db_ref.clone()))
        .collect();

    // Drop individual tracks from files that survive (are not removed wholesale).
    let mut tracks_to_remove: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for key in keys {
        if let NodeKey::Track(TrackRef { fi, index: ti }) = key {
            tracks_to_remove
                .entry(fi.as_usize())
                .or_default()
                .insert(ti.as_usize());
        }
    }
    for (fi, track_set) in &tracks_to_remove {
        if fully_removed.contains(fi) {
            continue;
        }
        if let Some(file) = loaded_files.get_mut(*fi) {
            for ti in (0..file.tracks.len()).rev() {
                if track_set.contains(&ti) {
                    file.tracks.remove(ti);
                }
            }
        }
    }

    for fi in (0..loaded_files.len()).rev() {
        if fully_removed.contains(&fi) {
            loaded_files.remove(fi);
        }
    }

    tree.reset_for_files(loaded_files);
    affected
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use gt_types::{
        DatabaseRef, FileIdx, FileMetadata, FileSource, LoadedFile, LoadedTrack, TrackIdx,
        TrackLod, TrackMetadata,
    };

    use super::{NodeKey, TrackRef, affected_recordings, files_fully_removed};

    fn make_file(track_count: usize, has_db_ref: bool, idx: usize) -> LoadedFile {
        LoadedFile {
            metadata: FileMetadata::default(),
            tracks: (0..track_count)
                .map(|_| LoadedTrack {
                    metadata: TrackMetadata::default(),
                    points: Vec::new(),
                    lod: TrackLod::default(),
                    custom_markers: Vec::new(),
                    generated_markers: Vec::new(),
                    event_markers: Vec::new(),
                })
                .collect(),
            event_marker_styles: HashMap::new(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdPath(PathBuf::new()),
            load_warnings: Vec::new(),
            identity: String::new(),
            db_ref: has_db_ref.then(|| DatabaseRef {
                identity: "id".to_owned(),
                group_name: format!("rec{idx}"),
            }),
            recording_meta: None,
        }
    }

    fn track_key(fi: usize, ti: usize) -> NodeKey {
        NodeKey::Track(TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti)))
    }

    fn file_key(fi: usize) -> NodeKey {
        NodeKey::File(FileIdx::new(fi))
    }

    #[test]
    fn fully_removed_and_affected_cover_the_key_cases() {
        struct Case {
            name: &'static str,
            /// One `(track_count, has_db_ref)` per file in the fixture.
            files: Vec<(usize, bool)>,
            keys: Vec<NodeKey>,
            /// File indices expected to be removed wholesale (ascending).
            expect_removed: Vec<usize>,
            /// Number of removed files that carry a history `db_ref`.
            expect_affected: usize,
        }

        let cases = [
            Case {
                name: "no keys removes nothing",
                files: vec![(2, true)],
                keys: vec![],
                expect_removed: vec![],
                expect_affected: 0,
            },
            Case {
                name: "file key removes the whole file",
                files: vec![(2, true)],
                keys: vec![file_key(0)],
                expect_removed: vec![0],
                expect_affected: 1,
            },
            Case {
                name: "all tracks selected promotes to full removal",
                files: vec![(2, true)],
                keys: vec![track_key(0, 0), track_key(0, 1)],
                expect_removed: vec![0],
                expect_affected: 1,
            },
            Case {
                name: "partial track selection does not remove the file",
                files: vec![(3, true)],
                keys: vec![track_key(0, 0), track_key(0, 1)],
                expect_removed: vec![],
                expect_affected: 0,
            },
            Case {
                name: "removed file without db_ref is not in affected",
                files: vec![(1, false)],
                keys: vec![file_key(0)],
                expect_removed: vec![0],
                expect_affected: 0,
            },
            Case {
                name: "removes one file and leaves the other",
                files: vec![(1, true), (2, true)],
                keys: vec![file_key(1)],
                expect_removed: vec![1],
                expect_affected: 1,
            },
        ];

        for case in cases {
            let files: Vec<LoadedFile> = case
                .files
                .iter()
                .enumerate()
                .map(|(i, &(tracks, has_ref))| make_file(tracks, has_ref, i))
                .collect();

            let removed: Vec<usize> = files_fully_removed(&case.keys, &files)
                .into_iter()
                .collect();
            assert_eq!(
                removed, case.expect_removed,
                "removed set for '{}'",
                case.name
            );

            let affected = affected_recordings(&case.keys, &files);
            assert_eq!(
                affected.len(),
                case.expect_affected,
                "affected count for '{}'",
                case.name
            );
        }
    }
}
