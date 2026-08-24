use egui::{Button, Checkbox, Grid, Label, RichText, ScrollArea, Window};
use egui_phosphor::regular::WARNING as ICON_WARNING;
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use gt_jam::text::{ATTRIBUTION, PUBLISHER_URL, UPSTREAM_URL};
use gt_map::{MapLayer, NavMap};
use gt_pending_writes::WriteAccess;
use gt_side_panel::{NodeKey, RecordingDetails, TreeState};
use gt_store::{DatabaseRef, EnvironmentArchive};
use gt_types::{LoadWarning, TrackRef};
use gt_ui_theme::warning_amber;
use strum::IntoEnumIterator as _;

use gt_loaded_files::{LoadedFiles, LoadedFilesView, RecordingNames};

use crate::app::environment_storage::{CoveredDayCounts, PruneRequest, PruneScope, PrunedDays};
use crate::app::mapbox_token;
use crate::app::mapbox_token::{MapboxTokenCommit, MapboxTokenField};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;

const PERMANENT_DELETE_LABEL: &str = "Also delete permanently from history";

/// The tracks of one stored recording that a remove acts on.
pub struct RecordingTrackRemoval {
    pub db_ref: DatabaseRef,
    /// Original (segmentation) track indices, matching the recording's stored
    /// track table - not the live view positions, which shift as tracks are
    /// removed.
    pub track_indices: Vec<usize>,
}

/// Actions the app applies in the frame after the user confirms the remove
/// dialog.
pub struct RemoveOutcome {
    /// Per stored recording, the tracks being removed. Empty when nothing
    /// removed was backed by history.
    pub affected: Vec<RecordingTrackRemoval>,
    /// `true` to permanently delete the affected tracks from the originals,
    /// `false` to hide them.
    pub permanent: bool,
}

/// Show the delete-confirmation dialog.
///
/// Returns `Some` in the one frame when items were actually removed, so the
/// caller can rebuild caches that depend on file indices and apply the chosen
/// history operation (hide or permanent delete) to `affected`.
/// The button row of a modal dialog: buttons grouped bottom-right with the
/// affirmative (or destructive action) rightmost - `add_contents` adds them
/// in right-to-left order. The horizontal wrapper keeps the layout from
/// claiming the window's full height.
pub(super) fn dialog_button_row(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
            add_contents,
        );
    });
}

pub fn show_delete_confirmation(
    ui: &egui::Ui,
    tree: &mut TreeState,
    loaded_files: &mut LoadedFiles,
    recording_names: &RecordingNames,
    write_access: WriteAccess,
) -> Option<RemoveOutcome> {
    let Some(confirm) = &tree.delete_confirm else {
        return None;
    };
    let count = confirm.items.len();
    let mut permanent = confirm.delete_permanently;
    // Tracks backed by history that this remove touches. Drives the wording and
    // whether the "delete permanently" option is even relevant.
    let removals = track_removals(&confirm.items, loaded_files.view());
    let affected_recordings = removals.len();
    let affected_tracks: usize = removals.iter().map(|r| r.track_indices.len()).sum();

    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let mut do_delete = enter_pressed;
    let mut do_cancel = escape_pressed;

    let item_label = if count == 1 { "item" } else { "items" };
    Window::new(format!("Remove {count} {item_label}?"))
        .collapsible(false)
        .resizable(true)
        .min_width(420.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ScrollArea::vertical()
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
                                    let name = recording_names
                                        .get(*fi)
                                        .unwrap_or(file.metadata.filename.as_str());
                                    ui.add(Label::new(name).truncate());
                                }
                            }
                            NodeKey::Track(TrackRef { fi, index: ti }) => {
                                if let Some(file) = fi.get(loaded_files)
                                    && let Some(track) = ti.get(&file.tracks)
                                {
                                    let name = recording_names
                                        .get(*fi)
                                        .unwrap_or(file.metadata.filename.as_str());
                                    let dist = gt_fmt::format_distance(track.metadata.distance_km);
                                    let dur = gt_fmt::format_human_terse_duration(
                                        track.metadata.duration,
                                    );
                                    let label = format!(
                                        "  {name} / #{}  {dist}  {dur}",
                                        track.metadata.index
                                    );
                                    ui.add(Label::new(label.as_str()).truncate());
                                }
                            }
                        }
                    }
                });
            ui.separator();
            if affected_tracks == 0 {
                ui.label(
                    RichText::new("This only removes them from the current view.")
                        .weak()
                        .small(),
                );
            } else if !write_access.allows_writing() {
                permanent = false;
                ui.add_enabled(
                    false,
                    Checkbox::new(&mut permanent, PERMANENT_DELETE_LABEL),
                )
                .on_disabled_hover_text(READ_ONLY_RECORDING_HISTORY_HOVER);
                ui.label(
                    RichText::new(
                        "This only removes them from the current view: the session is read-only \
                         and leaves every recording in history as it is.",
                    )
                    .weak()
                    .small(),
                );
            } else {
                ui.checkbox(&mut permanent, PERMANENT_DELETE_LABEL);
                let track_label = gt_fmt::pluralize(affected_tracks, "track", "tracks");
                let rec_label = gt_fmt::pluralize(affected_recordings, "recording", "recordings");
                let detail = if permanent {
                    format!(
                        "Removes them from the view and permanently deletes {affected_tracks} {track_label} from {affected_recordings} {rec_label} in history."
                    )
                } else {
                    format!(
                        "Removes them from the view and hides {affected_tracks} {track_label} in {affected_recordings} {rec_label} in history."
                    )
                };
                ui.label(RichText::new(detail).weak().small());
            }
            ui.add_space(4.0);
            dialog_button_row(ui, |ui| {
                if ui.button("Remove").clicked() {
                    do_delete = true;
                }
                if ui.button("Cancel").clicked() {
                    do_cancel = true;
                }
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
fn files_fully_removed(keys: &[NodeKey], loaded_files: LoadedFilesView<'_>) -> BTreeSet<usize> {
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
        if let Some(entry) = loaded_files.get(*fi) {
            let file = entry.file();
            if !file.tracks.is_empty() && (0..file.tracks.len()).all(|ti| track_set.contains(&ti)) {
                files.insert(*fi);
            }
        }
    }
    files
}

/// For every removed track that belongs to a stored recording, the original
/// (segmentation) track indices to act on in history, grouped by recording.
///
/// A removed file contributes all of its tracks. Track indices are taken from
/// each track's stored `metadata.index` rather than its live view position, so
/// they line up with the recording's persisted track table.
fn track_removals(
    keys: &[NodeKey],
    loaded_files: LoadedFilesView<'_>,
) -> Vec<RecordingTrackRemoval> {
    // File index -> set of removed view positions.
    let mut by_file: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for key in keys {
        match key {
            NodeKey::File(fi) => {
                if let Some(entry) = loaded_files.entry_for(*fi) {
                    by_file
                        .entry(fi.as_usize())
                        .or_default()
                        .extend(0..entry.file().tracks.len());
                }
            }
            NodeKey::Track(TrackRef { fi, index: ti }) => {
                by_file
                    .entry(fi.as_usize())
                    .or_default()
                    .insert(ti.as_usize());
            }
        }
    }

    let mut removals = Vec::new();
    for (fi, positions) in by_file {
        let Some(entry) = loaded_files.get(fi) else {
            continue;
        };
        let file = entry.file();
        let Some(db_ref) = entry.history().db_ref().cloned() else {
            continue;
        };
        let track_indices: Vec<usize> = positions
            .iter()
            .filter_map(|ti| file.tracks.get(*ti))
            // `metadata.index` is the 1-based display index. The stored track
            // table is 0-based, so shift down by one.
            .map(|t| t.metadata.index.saturating_sub(1))
            .collect();
        if !track_indices.is_empty() {
            removals.push(RecordingTrackRemoval {
                db_ref,
                track_indices,
            });
        }
    }
    removals
}

/// Remove `keys` from the view and return, per stored recording, the tracks that
/// were removed (so the caller can hide or permanently delete them). Computed
/// before the view is mutated, while the track indices are still intact.
pub fn execute_delete(
    keys: &[NodeKey],
    loaded_files: &mut LoadedFiles,
    tree: &mut TreeState,
) -> Vec<RecordingTrackRemoval> {
    let fully_removed = files_fully_removed(keys, loaded_files.view());
    let removals = track_removals(keys, loaded_files.view());

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
            loaded_files.remove_file(fi);
        }
    }

    tree.reset_for_files(loaded_files.files());
    removals
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
    Window::new(format!("{count} event markers outside track range"))
        .collapsible(false)
        .resizable(true)
        .min_width(480.0)
        .show(ui.ctx(), |ui| {
            ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                let ten_min = chrono::Duration::minutes(10);
                let mut prev_ts: Option<DateTime<Utc>> = None;
                for (ts, path) in orphans.iter() {
                    if let Some(prev) = prev_ts
                        && ts.signed_duration_since(prev) > ten_min
                    {
                        ui.separator();
                    }
                    let line = format!("{}  {}", ts.format("%Y-%m-%d %H:%M:%S"), path);
                    ui.add(Label::new(RichText::new(&line).monospace()).truncate());
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

    Window::new("Data quality warnings")
        .collapsible(false)
        .resizable(true)
        .min_width(540.0)
        .show(ui.ctx(), |ui| {
            ui.add(Label::new(RichText::new(filename.as_str()).strong()).truncate());
            ui.separator();
            ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                Grid::new("load_warnings_grid")
                    .num_columns(4)
                    .striped(true)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for w in warnings.iter() {
                            ui.label(
                                RichText::new(ICON_WARNING)
                                    .color(warning_amber(ui.visuals().dark_mode)),
                            );
                            ui.label(RichText::new(w.count.to_string()).strong());
                            ui.label(&w.issue);
                            ui.add(Label::new(&w.description).wrap());
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

/// Resizable dialog listing a recording's metadata (title, device, identity,
/// notes). Opened from a file row's note icon. Sized generously so long
/// identities and note paths read in full.
pub fn show_recording_details_dialog(ui: &egui::Ui, request: &mut Option<RecordingDetails>) {
    let Some(details) = request else {
        return;
    };
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    // The titlebar close button drives `open`, Escape also dismisses. A
    // read-only viewer needs no explicit footer button.
    let mut open = true;
    Window::new("Recording details")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .min_width(420.0)
        .default_width(480.0)
        .show(ui.ctx(), |ui| {
            ui.add(
                Label::new(RichText::new(details.metadata.filename.as_str()).strong()).truncate(),
            );
            ui.separator();
            ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                gt_side_panel::widgets::metadata_detail_rows(
                    ui,
                    &gt_side_panel::widgets::MetadataView::from_file_metadata(
                        &details.metadata,
                        details.identity.as_deref(),
                    ),
                );
            });
        });

    if !open || escape_pressed {
        *request = None;
    }
}

/// Show the About dialog: version and the data/service attributions.
///
/// Map tiles and snap-to-road matching both build on OpenStreetMap data
/// (ODbL), the default matching server is run by FOSSGIS e.V., and the
/// interference overlay is gpsjam.org's data over adsbexchange.com's
/// reports - the credits live here, always reachable from the menu bar.
pub fn show_about_dialog(ui: &egui::Ui, open: &mut bool, version: &str) {
    if !*open {
        return;
    }
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let mut keep_open = true;
    Window::new("About GeoTrace")
        .open(&mut keep_open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            // Selectable: the version is what a bug report quotes.
            ui.add(
                Label::new(RichText::new(format!("GeoTrace {version}")).strong()).selectable(true),
            );
            ui.label("GPS/GNSS navigation data visualizer");
            ui.separator();
            ui.label("Map tiles and road-network matching build on OpenStreetMap data");
            ui.hyperlink_to(
                "© OpenStreetMap contributors",
                "https://www.openstreetmap.org/copyright",
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("The default snap to road server is hosted by");
                ui.hyperlink_to("FOSSGIS e.V.", "https://www.fossgis.de/");
            });
            ui.add_space(4.0);
            ui.label(ATTRIBUTION);
            ui.horizontal(|ui| {
                ui.hyperlink_to("gpsjam.org", PUBLISHER_URL);
                ui.hyperlink_to("adsbexchange.com", UPSTREAM_URL);
            });
        });

    if !keep_open || escape_pressed {
        *open = false;
    }
}

/// The user's decision in the snap upload-consent dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapConsentChoice {
    /// Uploads to the configured server's host are acknowledged. `auto_snap`
    /// carries the mode choice when the dialog asked for one (`None` = the
    /// choice was already made earlier and was not asked again).
    Accepted { auto_snap: Option<bool> },
    /// No acknowledgment. The manual snap action stays available and
    /// re-prompts. Auto mode turns off - a declined consent must never
    /// leave automatic uploads armed.
    Declined,
}

/// Show the one-time snap upload-consent dialog naming the configured server
/// and what is uploaded. Returns `None` while the dialog stays open.
///
/// Recorded location data leaves the machine, so nothing is ever sent before
/// this dialog has been accepted for the configured server's host (see
/// `SnapSettings::consent_granted`). With `ask_auto` the dialog also asks
/// whether tracks should snap automatically from now on - both agree
/// buttons acknowledge uploads, they differ only in that choice. Escape,
/// Cancel, and the close button all decline. The acknowledgment is not
/// persisted on decline, so the next manual trigger re-prompts.
pub fn show_snap_consent_dialog(
    ui: &egui::Ui,
    server_url: &str,
    ask_auto: bool,
) -> Option<SnapConsentChoice> {
    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    // Enter agrees with the default mode: automatic when the choice is
    // being asked (the design default), unchanged otherwise.
    let mut choice = None;
    if enter_pressed {
        choice = Some(SnapConsentChoice::Accepted {
            auto_snap: ask_auto.then_some(true),
        });
    }
    if escape_pressed {
        choice = Some(SnapConsentChoice::Declined);
    }

    let mut open = true;
    egui::Window::new("Snap to road")
        .collapsible(false)
        .resizable(false)
        .min_width(420.0)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(
                "Snap to road matches a recorded track against the OpenStreetMap road network.",
            );
            ui.add_space(4.0);
            ui.label("The track's recorded positions and timestamps are uploaded to");
            ui.monospace(server_url);
            // The service description only applies to the public FOSSGIS
            // infrastructure. A self-hosted server has its own terms.
            if gt_snap::server_host(server_url) == gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL)
            {
                ui.hyperlink_to(
                    "Read more about the routing service",
                    gt_snap::SERVICE_INFO_URL,
                );
            }
            ui.add_space(4.0);
            ui.label(
                "Nothing is uploaded until you agree. The acknowledgment is remembered for this \
                 server and asked again when the server changes.",
            );
            if ask_auto {
                ui.add_space(4.0);
                ui.label(
                    "Snapping can run automatically: every track you load and show on the map \
                     is uploaded and matched without a click. Manual only uploads a track when \
                     you trigger it. Changeable anytime in the settings.",
                );
            }
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                if ask_auto {
                    if ui.button("Agree - snap automatically").clicked() {
                        choice = Some(SnapConsentChoice::Accepted {
                            auto_snap: Some(true),
                        });
                    }
                    if ui.button("Agree - manual only").clicked() {
                        choice = Some(SnapConsentChoice::Accepted {
                            auto_snap: Some(false),
                        });
                    }
                } else if ui.button("Agree").clicked() {
                    choice = Some(SnapConsentChoice::Accepted { auto_snap: None });
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(SnapConsentChoice::Declined);
                }
            });
        });
    if !open {
        choice = Some(SnapConsentChoice::Declined);
    }
    choice
}

/// The user's decision in the replace-cached-run confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapReplaceChoice {
    /// Run against the server again, replacing the stored result.
    SnapAgain,
    /// Keeps the stored result: nothing is uploaded.
    Cancel,
}

/// Ask before a "Snap again as" choice replaces the result this track
/// already has for `costing_name`. Returns `None` while the dialog stays
/// open.
///
/// Escape, Cancel, and the close button all keep the stored result: a
/// dismissed dialog never uploads anything.
pub fn show_snap_replace_dialog(ui: &egui::Ui, costing_name: &str) -> Option<SnapReplaceChoice> {
    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let mut choice = None;
    if enter_pressed {
        choice = Some(SnapReplaceChoice::SnapAgain);
    }
    if escape_pressed {
        choice = Some(SnapReplaceChoice::Cancel);
    }

    let mut open = true;
    egui::Window::new("Snap to road again?")
        .collapsible(false)
        .resizable(false)
        .min_width(380.0)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(format!(
                "This track already has snap to road data for {costing_name}."
            ));
            ui.add_space(4.0);
            ui.label(
                "Snapping again uploads the track once more and replaces that result with the \
                 new one.",
            );
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                if ui.button("Snap again").clicked() {
                    choice = Some(SnapReplaceChoice::SnapAgain);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(SnapReplaceChoice::Cancel);
                }
            });
        });
    if !open {
        choice = Some(SnapReplaceChoice::Cancel);
    }
    choice
}

/// How many tracks one scope of a recording covers, and how many of them
/// already have a run for the chosen costing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapScopeCount {
    pub tracks: usize,
    pub already_snapped: usize,
}

/// The two scopes a recording-level "Snap again as" choice can run over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapScopeCounts {
    pub selected: SnapScopeCount,
    pub all: SnapScopeCount,
}

/// Which of a recording's tracks a "Snap again as" choice covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapScope {
    /// Only the recording's tracks selected in the panel.
    SelectedTracks,
    /// Every track of the recording.
    AllTracks,
}

/// The user's decision in the scope dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapScopeChoice {
    Snap(SnapScope),
    /// Nothing is uploaded.
    Cancel,
}

/// Ask which of a recording's tracks a "Snap again as" choice covers, and
/// how many of them already have data for `costing_name`. Returns `None`
/// while the dialog stays open.
///
/// Escape, Cancel, and the close button all drop the choice: nothing is
/// uploaded.
pub fn show_snap_scope_dialog(
    ui: &egui::Ui,
    costing_name: &str,
    counts: SnapScopeCounts,
) -> Option<SnapScopeChoice> {
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let mut choice = escape_pressed.then_some(SnapScopeChoice::Cancel);

    let mut open = true;
    Window::new(format!("Snap to road as {costing_name}"))
        .collapsible(false)
        .resizable(false)
        .min_width(380.0)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            Grid::new("snap_scope_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    let mut row = |label: &str, count: SnapScopeCount| {
                        ui.label(RichText::new(label).weak());
                        ui.label(scope_summary(count, costing_name));
                        ui.end_row();
                    };
                    row("Selected", counts.selected);
                    row("All", counts.all);
                });
            if counts.all.already_snapped > 0 {
                ui.add_space(4.0);
                ui.label("Snapping again uploads those tracks once more and replaces their data.");
            }
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                if ui.button("Snap all tracks").clicked() {
                    choice = Some(SnapScopeChoice::Snap(SnapScope::AllTracks));
                }
                let selected = ui.add_enabled(
                    counts.selected.tracks > 0,
                    Button::new("Snap selected tracks"),
                );
                if selected.clicked() {
                    choice = Some(SnapScopeChoice::Snap(SnapScope::SelectedTracks));
                }
                selected.on_disabled_hover_text("Select tracks of this recording first");
                if ui.button("Cancel").clicked() {
                    choice = Some(SnapScopeChoice::Cancel);
                }
            });
        });
    if !open {
        choice = Some(SnapScopeChoice::Cancel);
    }
    choice
}

/// One scope row of [`show_snap_scope_dialog`]: how many tracks it covers
/// and how many of those already carry data for the chosen costing.
fn scope_summary(count: SnapScopeCount, costing_name: &str) -> String {
    let tracks = format!(
        "{} {}",
        count.tracks,
        gt_fmt::pluralize(count.tracks, "track", "tracks")
    );
    if count.already_snapped == 0 {
        return tracks;
    }
    format!(
        "{tracks}, {} already snapped as {costing_name}",
        count.already_snapped
    )
}

/// The user's decision in the one-time auto-snap prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapAutoChoice {
    /// Loaded tracks snap automatically from now on.
    Automatic,
    /// Snapping stays click-triggered.
    ManualOnly,
}

/// One-time prompt for users who acknowledged uploads before auto mode
/// existed: asks whether loaded tracks should snap automatically from now
/// on. Returns `None` while the dialog stays open.
///
/// Escape and the close button choose manual only - dismissing a dialog
/// must never silently expand what gets uploaded.
pub fn show_snap_auto_prompt(ui: &egui::Ui, server_url: &str) -> Option<SnapAutoChoice> {
    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let mut choice = None;
    if enter_pressed {
        choice = Some(SnapAutoChoice::Automatic);
    }
    if escape_pressed {
        choice = Some(SnapAutoChoice::ManualOnly);
    }

    let mut open = true;
    egui::Window::new("Snap to road automatically?")
        .collapsible(false)
        .resizable(false)
        .min_width(420.0)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label(
                "Snap to road can run automatically: every track you load and show on the map \
                 is uploaded and matched without a click.",
            );
            ui.add_space(4.0);
            ui.label("Your earlier acknowledgment still applies; uploads go to");
            ui.monospace(server_url);
            ui.add_space(4.0);
            ui.label("Changeable anytime in the settings.");
            ui.add_space(8.0);
            dialog_button_row(ui, |ui| {
                if ui.button("Snap automatically").clicked() {
                    choice = Some(SnapAutoChoice::Automatic);
                }
                if ui.button("Manual only").clicked() {
                    choice = Some(SnapAutoChoice::ManualOnly);
                }
            });
        });
    if !open {
        choice = Some(SnapAutoChoice::ManualOnly);
    }
    choice
}

pub fn show_mapbox_token_dialog(
    ui: &egui::Ui,
    map: &mut NavMap,
    token_field: &mut MapboxTokenField,
) {
    // ESC dismisses the dialog - same effect as the cancel button.
    let esc_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if esc_pressed {
        map.set_layer(MapLayer::OpenStreetMap);
        return;
    }

    let mut open = true;
    Window::new("Mapbox API Token Required")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.label("Satellite view requires a Mapbox API token");
            ui.label("Get one free at mapbox.com");
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(mapbox_token::TOKEN_LABEL);
                token_field.show(ui, map, MapboxTokenCommit::OnEnter);
                if ui.button("Apply").clicked() {
                    token_field.commit(map);
                }
            });
            if ui.button("Cancel - use OpenStreetMap").clicked() {
                map.set_layer(MapLayer::OpenStreetMap);
            }
        });

    // X button in the title bar was clicked - treat as cancel.
    if !open {
        map.set_layer(MapLayer::OpenStreetMap);
    }
}

/// The environment-data delete waiting for the user to confirm, and what it
/// would take.
pub struct EnvironmentPrunePrompt<'a> {
    pub request: PruneRequest,
    /// How many days each archive holds inside the delete's range.
    pub covered: CoveredDayCounts,
    /// Loaded recordings spanning a day the delete removes, named as the rest
    /// of the app names them.
    pub loaded_recordings: &'a [String],
}

pub enum EnvironmentPruneChoice {
    Delete,
    Cancel,
}

/// Confirm an environment-data delete, naming what goes and which loaded
/// recordings are downloaded again straight after.
///
/// Returns the choice in the frame the user makes it, and [`None`] while the
/// dialog is still open.
pub fn show_environment_prune_confirmation(
    ui: &egui::Ui,
    prompt: &EnvironmentPrunePrompt<'_>,
) -> Option<EnvironmentPruneChoice> {
    let mut choice = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        .then_some(EnvironmentPruneChoice::Cancel);

    Window::new("Delete archived days?")
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            ui.set_max_width(460.0);
            ui.label(prune_scope_line(prompt.request));
            ui.add_space(4.0);
            Grid::new("environment_prune_grid")
                .num_columns(2)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for archive in EnvironmentArchive::iter()
                        .filter(|archive| prompt.request.scope.covers(*archive))
                    {
                        let days = prompt.covered.of(archive);
                        ui.label(archive.label());
                        ui.label(format!("{days} {}", gt_fmt::pluralize(days, "day", "days")));
                        ui.end_row();
                    }
                });

            if !prompt.loaded_recordings.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "These loaded recordings span days in that range. Those days are \
                         downloaded again as soon as the delete finishes.",
                    )
                    .weak()
                    .small(),
                );
                ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for name in prompt.loaded_recordings {
                        ui.add(Label::new(name.as_str()).truncate());
                    }
                });
            }

            ui.add_space(6.0);
            dialog_button_row(ui, |ui| {
                if ui
                    .button(RichText::new("Delete").color(warning_amber(ui.visuals().dark_mode)))
                    .on_hover_text(
                        "This cannot be undone. The days are downloaded again as they are needed.",
                    )
                    .clicked()
                {
                    choice = Some(EnvironmentPruneChoice::Delete);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(EnvironmentPruneChoice::Cancel);
                }
            });
        });

    choice
}

/// What the delete acts on, as the dialog opens with.
fn prune_scope_line(request: PruneRequest) -> String {
    match (request.scope, request.days) {
        (PruneScope::Every, PrunedDays::Before(cutoff)) => {
            format!("Every archive loses the days it holds before {cutoff}")
        }
        (PruneScope::Every, PrunedDays::All) => "Every archive loses every day it holds".to_owned(),
        (PruneScope::One(archive), PrunedDays::Before(cutoff)) => format!(
            "The {} archive loses the days it holds before {cutoff}",
            archive.label()
        ),
        (PruneScope::One(archive), PrunedDays::All) => {
            format!("The {} archive loses every day it holds", archive.label())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceQuitChoice {
    Quit,
    Cancel,
}

/// Confirm ending the process with the running writes unfinished, listing what
/// each one costs.
///
/// Returns the choice in the frame the user makes it, and [`None`] while the
/// dialog is still open.
pub fn show_force_quit_confirmation(
    ui: &egui::Ui,
    interruption_costs: &[String],
) -> Option<ForceQuitChoice> {
    let mut choice = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        .then_some(ForceQuitChoice::Cancel);

    Window::new("Force quit?")
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ui.ctx(), |ui| {
            // Fits inside the window that shutdown sizes itself down to.
            ui.set_max_width(360.0);
            ui.label("GeoTrace ends now, with the work it is still doing unfinished");
            ui.add_space(4.0);
            for cost in interruption_costs {
                ui.add(Label::new(cost.as_str()).wrap());
            }
            ui.add_space(6.0);
            dialog_button_row(ui, |ui| {
                if ui
                    .button(
                        RichText::new("Force quit").color(warning_amber(ui.visuals().dark_mode)),
                    )
                    .on_hover_text("This cannot be undone")
                    .clicked()
                {
                    choice = Some(ForceQuitChoice::Quit);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(ForceQuitChoice::Cancel);
                }
            });
        });

    choice
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gt_types::{
        FileIdx, FileSource, LoadedFile, LoadedTrack, TrackIdx, TrackLod, TrackMetadata,
    };

    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_map::TileAccess;
    use gt_pending_writes::{WriteAccess, WriteKind};
    use gt_test_utils::{HarnessInteraction as _, TestHarness};
    use rustc_hash::FxHashMap;

    use super::{
        CoveredDayCounts, EnvironmentArchive, EnvironmentPruneChoice, EnvironmentPrunePrompt,
        ForceQuitChoice, MapLayer, MapboxTokenField, NavMap, NodeKey, PERMANENT_DELETE_LABEL,
        PruneRequest, PruneScope, PrunedDays, TrackRef, files_fully_removed, prune_scope_line,
        show_delete_confirmation, show_environment_prune_confirmation,
        show_force_quit_confirmation, show_mapbox_token_dialog, track_removals,
    };
    use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
    use gt_side_panel::{DeleteConfirmState, TreeState};

    fn day(offset: i64) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap_or_default()
            + chrono::TimeDelta::days(offset)
    }

    fn prune_request(scope: PruneScope) -> PruneRequest {
        PruneRequest {
            scope,
            days: PrunedDays::Before(day(0)),
        }
    }

    fn covered_days() -> CoveredDayCounts {
        CoveredDayCounts {
            interference: 2,
            geomagnetic_indices: 3,
            tec_maps: 29,
            solar_flares: 1,
        }
    }

    /// Renders the confirmation and reports the choice it took.
    fn prune_dialog<'a>(
        prompt: &'a EnvironmentPrunePrompt<'a>,
        choice: &'a std::cell::RefCell<Option<EnvironmentPruneChoice>>,
    ) -> TestHarness<'a, ()> {
        let mut harness = TestHarness::builder()
            .size(egui::vec2(520.0, 320.0))
            .ui(|ui| {
                if let Some(made) = show_environment_prune_confirmation(ui, prompt) {
                    *choice.borrow_mut() = Some(made);
                }
            });
        harness.run();
        harness
    }

    /// The dialog names the loaded recordings whose days go: the fetch
    /// schedulers download those days again straight after.
    #[test]
    fn the_prune_dialog_names_the_loaded_recordings_it_affects() {
        let recordings = ["Morning ride".to_owned(), "Ferry crossing".to_owned()];
        let prompt = EnvironmentPrunePrompt {
            request: prune_request(PruneScope::Every),
            covered: covered_days(),
            loaded_recordings: &recordings,
        };
        let choice = std::cell::RefCell::new(None);
        let harness = prune_dialog(&prompt, &choice);

        for name in &recordings {
            assert!(
                harness.inner.query_by_label_contains(name).is_some(),
                "{name} is not named in the dialog"
            );
        }
        assert!(
            harness
                .inner
                .query_by_label_contains("downloaded again")
                .is_some(),
            "the dialog does not say the days come back"
        );
    }

    #[test]
    fn confirming_the_prune_dialog_reports_the_delete() {
        let prompt = EnvironmentPrunePrompt {
            request: prune_request(PruneScope::Every),
            covered: covered_days(),
            loaded_recordings: &[],
        };
        let choice = std::cell::RefCell::new(None);
        let mut harness = prune_dialog(&prompt, &choice);
        harness.inner.get_by_label("Delete").click();
        harness.run();
        drop(harness);

        assert!(matches!(
            choice.into_inner(),
            Some(EnvironmentPruneChoice::Delete)
        ));
    }

    #[test]
    fn cancelling_the_prune_dialog_deletes_nothing() {
        let prompt = EnvironmentPrunePrompt {
            request: prune_request(PruneScope::Every),
            covered: covered_days(),
            loaded_recordings: &[],
        };
        let choice = std::cell::RefCell::new(None);
        let mut harness = prune_dialog(&prompt, &choice);
        harness.inner.get_by_label("Cancel").click();
        harness.run();
        drop(harness);

        assert!(matches!(
            choice.into_inner(),
            Some(EnvironmentPruneChoice::Cancel)
        ));
    }

    /// Renders the force-quit confirmation and reports the choice it took.
    fn force_quit_dialog<'a>(
        interruption_costs: &'a [String],
        choice: &'a std::cell::RefCell<Option<ForceQuitChoice>>,
    ) -> TestHarness<'a, ()> {
        let mut harness = TestHarness::builder()
            .size(egui::vec2(520.0, 320.0))
            .ui(|ui| {
                if let Some(made) = show_force_quit_confirmation(ui, interruption_costs) {
                    *choice.borrow_mut() = Some(made);
                }
            });
        harness.run();
        harness
    }

    #[test]
    fn confirming_the_force_quit_dialog_reports_the_quit() {
        let costs = [WriteKind::Settings.interruption_cost()];
        let choice = std::cell::RefCell::new(None);
        let mut harness = force_quit_dialog(&costs, &choice);
        harness.inner.get_by_label("Force quit").click();
        harness.run();
        drop(harness);

        assert!(matches!(choice.into_inner(), Some(ForceQuitChoice::Quit)));
    }

    #[test]
    fn cancelling_the_force_quit_dialog_quits_nothing() {
        let costs = [WriteKind::Settings.interruption_cost()];
        let choice = std::cell::RefCell::new(None);
        let mut harness = force_quit_dialog(&costs, &choice);
        harness.inner.get_by_label("Cancel").click();
        harness.run();
        drop(harness);

        assert!(matches!(choice.into_inner(), Some(ForceQuitChoice::Cancel)));
    }

    #[test]
    fn escape_cancels_the_force_quit_dialog() {
        let costs = [WriteKind::Settings.interruption_cost()];
        let choice = std::cell::RefCell::new(None);
        let mut harness = force_quit_dialog(&costs, &choice);
        harness.inner.key_press(egui::Key::Escape);
        harness.run();
        drop(harness);

        assert!(matches!(choice.into_inner(), Some(ForceQuitChoice::Cancel)));
    }

    #[rstest::rstest]
    #[case::every_archive_before_a_day(
        PruneScope::Every,
        PrunedDays::Before(day(0)),
        "Every archive loses the days it holds before 2026-07-05"
    )]
    #[case::every_archive_entirely(
        PruneScope::Every,
        PrunedDays::All,
        "Every archive loses every day it holds"
    )]
    #[case::one_archive_before_a_day(
        PruneScope::One(EnvironmentArchive::IonosphericTec),
        PrunedDays::Before(day(0)),
        "The Ionospheric TEC archive loses the days it holds before 2026-07-05"
    )]
    #[case::one_archive_entirely(
        PruneScope::One(EnvironmentArchive::SolarFlares),
        PrunedDays::All,
        "The Solar flares archive loses every day it holds"
    )]
    fn the_dialog_opens_with_what_the_delete_takes(
        #[case] scope: PruneScope,
        #[case] days: PrunedDays,
        #[case] expected: &str,
    ) {
        assert_eq!(prune_scope_line(PruneRequest { scope, days }), expected);
    }

    /// The dialog as the user meets it: what goes, and which loaded recordings
    /// are downloaded again.
    #[test]
    fn snapshot_environment_prune_dialog() {
        let recordings = ["Morning ride".to_owned(), "Ferry crossing".to_owned()];
        let prompt = EnvironmentPrunePrompt {
            request: prune_request(PruneScope::Every),
            covered: covered_days(),
            loaded_recordings: &recordings,
        };
        let choice = std::cell::RefCell::new(None);
        let mut harness = prune_dialog(&prompt, &choice);
        harness.snapshot("environment_prune_dialog");
    }

    struct TokenDialogState {
        map: NavMap,
        field: MapboxTokenField,
    }

    fn token_dialog() -> TestHarness<'static, TokenDialogState> {
        let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
        map.set_layer(MapLayer::Satellite);
        let mut harness = TestHarness::builder().ui_state(
            |ui, state: &mut TokenDialogState| {
                show_mapbox_token_dialog(ui, &mut state.map, &mut state.field);
            },
            TokenDialogState {
                map,
                field: MapboxTokenField::default(),
            },
        );
        harness.run();
        harness
    }

    /// Cancelling leaves the map on OpenStreetMap with no token: the typed text
    /// was never applied.
    #[test]
    fn cancelling_the_token_dialog_keeps_the_typed_text_off_the_map() {
        let mut harness = token_dialog();
        harness.inner.type_into_text_input("typed");

        harness
            .inner
            .get_by_label("Cancel - use OpenStreetMap")
            .click();
        harness.run();

        assert!(!harness.state().map.has_mapbox_token());
        assert_eq!(harness.state().map.layer(), MapLayer::OpenStreetMap);
    }

    /// Escape dismisses the dialog the way Cancel does, without applying what
    /// was typed.
    #[test]
    fn escape_keeps_the_typed_text_off_the_map() {
        let mut harness = token_dialog();
        harness.inner.type_into_text_input("typed");

        harness.inner.key_press(egui::Key::Escape);
        harness.run();

        assert!(!harness.state().map.has_mapbox_token());
        assert_eq!(harness.state().map.layer(), MapLayer::OpenStreetMap);
    }

    #[test]
    fn applying_the_token_dialog_hands_the_typed_text_to_the_map() {
        let mut harness = token_dialog();
        harness.inner.type_into_text_input("typed");

        harness.inner.get_by_label("Apply").click();
        harness.run();

        assert_eq!(harness.state().map.mapbox_token(), "typed");
    }

    fn make_file(track_count: usize) -> LoadedFile {
        LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: (0..track_count)
                .map(|ti| LoadedTrack {
                    metadata: TrackMetadata {
                        // 1-based display index, as `build_loaded_file` assigns.
                        index: ti + 1,
                        ..gt_test_utils::empty_track_metadata()
                    },
                    points: Vec::new(),
                    lod: TrackLod::default(),
                    sat_label_anchors: Vec::new(),
                    custom_markers: Vec::new(),
                    generated_markers: Vec::new(),
                    event_markers: Vec::new(),
                    channels: Vec::new(),
                })
                .collect(),
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdPath(PathBuf::new()),
            load_warnings: Vec::new(),
        }
    }

    fn make_loaded_files(files: &[(usize, bool)]) -> LoadedFiles {
        let mut loaded = LoadedFiles::new();
        for (idx, &(track_count, has_db_ref)) in files.iter().enumerate() {
            let history = if has_db_ref {
                FileHistory::recording(
                    "id".to_owned(),
                    gt_store::RecordingMeta {
                        start_us: idx as i64,
                        end_us: idx as i64,
                        nav_point_count: 0,
                        sat_report_count: 0,
                        marker_count: 0,
                        event_marker_count: 0,
                        gtd_size_bytes: 0,
                    },
                    Some(gt_store::DatabaseRef {
                        identity: "id".to_owned(),
                        group_name: format!("rec{idx}"),
                    }),
                )
            } else {
                FileHistory::None
            };
            loaded.push(make_file(track_count), history);
        }
        loaded
    }

    /// The remove confirmation over one stored recording, with its first
    /// track selected for removal.
    struct DeleteConfirmationState {
        tree: TreeState,
        loaded_files: LoadedFiles,
        write_access: WriteAccess,
    }

    fn delete_confirmation_ui(ui: &mut egui::Ui, state: &mut DeleteConfirmationState) {
        let names = RecordingNames::resolve(state.loaded_files.view(), "{filename}");
        show_delete_confirmation(
            ui,
            &mut state.tree,
            &mut state.loaded_files,
            &names,
            state.write_access,
        );
    }

    /// Removing tracks of a stored recording writes to the recording history,
    /// which a read-only session does not: the permanent-delete tickbox is
    /// grayed, and the dialog names the view as all the remove changes.
    #[test]
    fn the_remove_confirmation_touches_no_recording_in_a_read_only_session() {
        let mut tree = TreeState::default();
        tree.delete_confirm = Some(DeleteConfirmState {
            items: vec![track_key(0, 0)],
            delete_permanently: true,
        });
        let mut harness = TestHarness::builder().ui_state(
            delete_confirmation_ui,
            DeleteConfirmationState {
                tree,
                loaded_files: make_loaded_files(&[(2, true)]),
                write_access: WriteAccess::ReadOnly,
            },
        );
        harness.inner.run_steps(3);

        assert!(
            harness
                .inner
                .get_by_label_contains(PERMANENT_DELETE_LABEL)
                .accesskit_node()
                .is_disabled()
        );
        harness
            .inner
            .get_by_label_contains("the session is read-only");
    }

    fn track_key(fi: usize, ti: usize) -> NodeKey {
        NodeKey::Track(TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti)))
    }

    fn file_key(fi: usize) -> NodeKey {
        NodeKey::File(FileIdx::new(fi))
    }

    #[test]
    fn fully_removed_and_track_removals_cover_the_key_cases() {
        struct Case {
            name: &'static str,
            /// One `(track_count, has_db_ref)` per file in the fixture.
            files: Vec<(usize, bool)>,
            keys: Vec<NodeKey>,
            /// File indices expected to be removed wholesale (ascending).
            expect_removed: Vec<usize>,
            /// Number of recordings the remove touches in history.
            expect_recordings: usize,
            /// Track indices removed per affected recording (ascending).
            expect_track_indices: Vec<Vec<usize>>,
        }

        let cases = [
            Case {
                name: "no keys removes nothing",
                files: vec![(2, true)],
                keys: vec![],
                expect_removed: vec![],
                expect_recordings: 0,
                expect_track_indices: vec![],
            },
            Case {
                name: "file key removes every track of the file",
                files: vec![(2, true)],
                keys: vec![file_key(0)],
                expect_removed: vec![0],
                expect_recordings: 1,
                expect_track_indices: vec![vec![0, 1]],
            },
            Case {
                name: "all tracks selected promotes to full removal",
                files: vec![(2, true)],
                keys: vec![track_key(0, 0), track_key(0, 1)],
                expect_removed: vec![0],
                expect_recordings: 1,
                expect_track_indices: vec![vec![0, 1]],
            },
            Case {
                name: "partial track selection hides just those tracks",
                files: vec![(3, true)],
                keys: vec![track_key(0, 0), track_key(0, 1)],
                expect_removed: vec![],
                expect_recordings: 1,
                expect_track_indices: vec![vec![0, 1]],
            },
            Case {
                name: "removed file without db_ref touches no recording",
                files: vec![(1, false)],
                keys: vec![file_key(0)],
                expect_removed: vec![0],
                expect_recordings: 0,
                expect_track_indices: vec![],
            },
            Case {
                name: "removes one file and leaves the other",
                files: vec![(1, true), (2, true)],
                keys: vec![file_key(1)],
                expect_removed: vec![1],
                expect_recordings: 1,
                expect_track_indices: vec![vec![0, 1]],
            },
        ];

        for case in cases {
            let files = make_loaded_files(&case.files);

            let removed: Vec<usize> = files_fully_removed(&case.keys, files.view())
                .into_iter()
                .collect();
            assert_eq!(
                removed, case.expect_removed,
                "removed set for '{}'",
                case.name
            );

            let removals = track_removals(&case.keys, files.view());
            assert_eq!(
                removals.len(),
                case.expect_recordings,
                "affected recording count for '{}'",
                case.name
            );
            let indices: Vec<Vec<usize>> =
                removals.iter().map(|r| r.track_indices.clone()).collect();
            assert_eq!(
                indices, case.expect_track_indices,
                "removed track indices for '{}'",
                case.name
            );
        }
    }
}
