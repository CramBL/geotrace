use egui::{Button, Grid, Label, RichText, ScrollArea, Window};
use egui_phosphor::regular::WARNING as ICON_WARNING;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use gt_fmt::UTC_SECOND_FORMAT;
use gt_jam::text::{ATTRIBUTION, PUBLISHER_URL, UPSTREAM_URL};
use gt_log_view::{LoadedLogs, RecordingKey};
use gt_map::{MapLayer, NavMap};
use gt_side_panel::{NodeKey, RecordingDetails, TreeState};
use gt_store::{DatabaseRef, EnvironmentArchive};
use gt_types::{LoadWarning, TrackRef};
use gt_ui_theme::warning_amber;
use strum::IntoEnumIterator as _;

use gt_loaded_files::{LoadedFiles, LoadedFilesView, RecordingNames};

use crate::app::anchored_dialog::{
    AnchoredDialog, AnchoredDialogKind, DialogRegions, HeldBodyLines,
};
use crate::app::environment_storage::{CoveredDayCounts, PruneRequest, PruneScope, PrunedDays};
use crate::app::mapbox_token;
use crate::app::mapbox_token::{MapboxTokenCommit, MapboxTokenField};

/// Label of the tickbox that escalates the shelve to a permanent delete.
const PERMANENT_DELETE_LABEL: &str = "Delete permanently from history";

pub(in crate::app) const SHELVE_BUTTON_LABEL: &str = "Shelve";

pub(in crate::app) const DELETE_PERMANENTLY_BUTTON_LABEL: &str = "Delete permanently";

/// The tracks of one stored recording that the confirmed action applies to.
pub struct RecordingTrackRemoval {
    pub db_ref: DatabaseRef,
    /// The rows of the recording's stored track table - not the live view
    /// positions, which shift as tracks are removed.
    pub track_rows: Vec<usize>,
}

/// What the confirmation does in the recording history to the tracks that it
/// takes out of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredTrackAction {
    Shelve,
    DeletePermanently,
}

impl StoredTrackAction {
    fn from_permanent_delete_tickbox(ticked: bool) -> Self {
        if ticked {
            Self::DeletePermanently
        } else {
            Self::Shelve
        }
    }
}

/// Actions the app applies in the frame after the user confirms the shelve
/// dialog.
pub struct ShelveOutcome {
    /// Per stored recording, the tracks that the action applies to.
    pub affected: Vec<RecordingTrackRemoval>,
    pub action: StoredTrackAction,
    /// The [`RecordingKey`] of every recording that the confirmation took out
    /// of the session.
    pub removed_recordings: Vec<RecordingKey>,
}

/// Gap between a dialog's body and the action row below it.
const DIALOG_ACTIONS_GAP: f32 = 6.0;

/// What a dialog scrolls: everything the user reads before acting.
pub(super) struct DialogBody<F>(F);

impl<F: FnOnce(&mut egui::Ui)> DialogBody<F> {
    pub(super) fn new(body: F) -> Self {
        Self(body)
    }
}

/// The row a dialog keeps on screen below its body: the buttons at its right
/// end, and one optional control at its left end.
///
/// Every dialog places its actions through this row.
pub(super) struct DialogActionRow<Leading, Buttons> {
    leading: Option<Leading>,
    buttons: Buttons,
}

impl<Buttons: FnOnce(&mut egui::Ui) -> R, R> DialogActionRow<fn(&mut egui::Ui), Buttons> {
    /// The buttons alone, grouped at the right end with the affirmative (or
    /// destructive action) rightmost: `buttons` adds them in right-to-left
    /// order. The row wraps onto further rows when the window is narrower
    /// than the buttons.
    pub(super) fn buttons(buttons: Buttons) -> Self {
        Self {
            leading: None,
            buttons,
        }
    }
}

impl<Leading, Buttons> DialogActionRow<Leading, Buttons> {
    /// Puts `leading` at the left end of the row, in the theme's weak text
    /// color and under a rule across the dialog's width.
    ///
    /// The weak color and the rule separate it from a tickbox the body ends
    /// with: the body's tickbox applies to the one action the row confirms,
    /// the leading control to every dialog of its class.
    pub(super) fn with_leading_control<F: FnOnce(&mut egui::Ui)>(
        self,
        leading: F,
    ) -> DialogActionRow<F, Buttons> {
        DialogActionRow {
            leading: Some(leading),
            buttons: self.buttons,
        }
    }
}

impl<Leading: FnOnce(&mut egui::Ui), Buttons: FnOnce(&mut egui::Ui) -> R, R>
    DialogActionRow<Leading, Buttons>
{
    fn ui(self, ui: &mut egui::Ui) -> R {
        let Self { leading, buttons } = self;
        if leading.is_some() {
            ui.separator();
        }
        // The horizontal wrapper keeps the layout from claiming the window's
        // full height.
        ui.horizontal(|ui| {
            if let Some(leading) = leading {
                ui.scope(|ui| {
                    let weak_text_color = ui.visuals().weak_text_color();
                    ui.visuals_mut().override_text_color = Some(weak_text_color);
                    leading(ui);
                });
            }
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
                buttons,
            )
            .inner
        })
        .inner
    }
}

/// A dialog's contents: `body` scrolls inside the room the screen leaves above
/// the action row, which stays out of that scroll area so it is always on
/// screen.
///
/// egui clips a window at the screen edge and scrolls nothing on its own, so
/// without this a long list or an unbroken identity puts the actions out of the
/// user's reach. The row's height is measured as it is drawn and reserved on
/// the next frame, which covers a row that wrapped onto a second line.
pub(super) fn dialog_body_above_the_action_row<R>(
    ui: &mut egui::Ui,
    body: DialogBody<impl FnOnce(&mut egui::Ui)>,
    actions: DialogActionRow<impl FnOnce(&mut egui::Ui), impl FnOnce(&mut egui::Ui) -> R>,
) -> R {
    dialog_body_above_the_action_row_taking(
        ui,
        DialogBodyHeight::UpToWhatTheWindowLeaves,
        body,
        actions,
    )
}

/// What a dialog's body does with the height above its actions.
#[derive(Clone, Copy)]
pub(super) enum DialogBodyHeight {
    /// As much as its content needs, and it scrolls past what the window
    /// leaves above the actions.
    UpToWhatTheWindowLeaves,

    /// As much as its content needs, in no scroll area, which is how a dialog
    /// measures its own height on the frame it opens.
    WhatItsContentNeeds,

    /// The whole height above the actions, which keeps the actions at the
    /// bottom edge of a window whose height is held.
    TheHeldHeight,
}

pub(super) fn dialog_body_above_the_action_row_taking<R>(
    ui: &mut egui::Ui,
    height: DialogBodyHeight,
    DialogBody(body): DialogBody<impl FnOnce(&mut egui::Ui)>,
    actions: DialogActionRow<impl FnOnce(&mut egui::Ui), impl FnOnce(&mut egui::Ui) -> R>,
) -> R {
    let measured_height_id = ui.id().with("dialog_actions_height");
    let reserved = ui
        .data(|data| data.get_temp::<f32>(measured_height_id))
        .unwrap_or_else(|| ui.spacing().interact_size.y + DIALOG_ACTIONS_GAP)
        + ui.spacing().item_spacing.y;
    match height {
        DialogBodyHeight::WhatItsContentNeeds => {
            ui.scope(body);
        }
        DialogBodyHeight::UpToWhatTheWindowLeaves | DialogBodyHeight::TheHeldHeight => {
            ScrollArea::both()
                .id_salt("dialog_body")
                .auto_shrink(!matches!(height, DialogBodyHeight::TheHeldHeight))
                // A body shorter than 64 points would open the dialog with
                // the difference as a gap above the action row: egui keeps 64
                // points for a scroll area to scroll in.
                .min_scrolled_height(0.0)
                .max_height((ui.available_height() - reserved).max(0.0))
                .show(ui, body);
        }
    }
    let laid_out = ui.scope(|ui| {
        ui.add_space(DIALOG_ACTIONS_GAP);
        actions.ui(ui)
    });
    ui.data_mut(|data| data.insert_temp(measured_height_id, laid_out.response.rect.height()));
    laid_out.inner
}

/// Returns `escape_choice` on the frame the user presses Escape, taking the
/// key so nothing else reads it.
fn consume_escape_press<T>(ctx: &egui::Context, escape_choice: T) -> Option<T> {
    ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        .then_some(escape_choice)
}

/// A confirmation drawn by [`AnchoredDialog`], with `body` above the action
/// row. The window keeps the position and the height it opened at, and `body`
/// gets the regions for its content.
///
/// Escape returns `escape_choice`: pass the choice that discards nothing.
pub(super) fn anchored_confirmation_dialog<T>(
    ctx: &egui::Context,
    kind: AnchoredDialogKind,
    title: impl Into<String>,
    escape_choice: T,
    body: impl FnOnce(&mut egui::Ui, DialogRegions),
    buttons: impl FnOnce(&mut egui::Ui) -> Option<T>,
) -> Option<T> {
    let mut choice = consume_escape_press(ctx, escape_choice);

    let dialog = AnchoredDialog::new(kind, title);
    let regions = dialog.regions();
    let clicked = dialog.show(
        ctx,
        DialogBody::new(|ui| body(ui, regions)),
        DialogActionRow::buttons(buttons),
    );
    if let Some(clicked) = clicked.flatten() {
        choice = Some(clicked);
    }
    choice
}

pub(super) fn destructive_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.button(RichText::new(label).color(warning_amber(ui.visuals().dark_mode)))
        .on_hover_text("This cannot be undone")
}

/// The region listing the items that the confirmation takes out of the view.
const SHELVED_ITEMS_REGION: &str = "shelved_items";

/// Lines the [`SHELVED_ITEMS_REGION`] holds at most, however many items the
/// confirmation takes: the rest of the rows scroll inside it. Twelve is the
/// room ten item rows take, each a line of body text with the spacing under it.
const SHELVED_ITEMS_MOST_LINES: u8 = 12;

/// The region counting the logs attached to the recordings the confirmation
/// takes out. A restored attachment that arrives while the dialog is open joins
/// that count.
const ATTACHED_LOGS_REGION: &str = "shelve_attached_logs";

/// Lines the [`ATTACHED_LOGS_REGION`] holds from the frame the dialog opens.
/// The longer of its two wordings takes one line at this width.
const ATTACHED_LOGS_LINES: u8 = 1;

/// Show the shelve confirmation, which the side panel's "Shelve filtered data"
/// button raises over the tracks that the filter excludes.
///
/// Returns `Some` in the one frame the user confirms it, so the caller can
/// rebuild the caches that depend on file indices and apply the chosen action
/// to `affected`.
pub fn show_shelve_confirmation(
    ui: &egui::Ui,
    tree: &mut TreeState,
    loaded_files: &mut LoadedFiles,
    recording_names: &RecordingNames,
    logs: &LoadedLogs,
) -> Option<ShelveOutcome> {
    let Some(confirm) = &tree.shelve_confirm else {
        return None;
    };
    let count = confirm.items.len();
    // The body's tickbox writes it and the action row's button reads it, from
    // two closures that the dialog holds at once.
    let permanent = Cell::new(confirm.delete_permanently);
    let removals = track_removals(&confirm.items, loaded_files.view());
    let affected_recordings = removals.len();
    let affected_tracks: usize = removals.iter().map(|r| r.track_rows.len()).sum();
    let removed_recordings = removed_recording_keys(&confirm.items, loaded_files.view());
    let attached_logs = logs
        .anchored_to(&removed_recordings)
        .filter(|(_, log)| log.attachment().is_some())
        .count();

    let enter_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let escape_pressed = ui
        .ctx()
        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));

    let mut do_confirm = enter_pressed;
    let mut do_cancel = escape_pressed;

    let item_label = gt_fmt::pluralize(count, "item", "items");
    let dialog = AnchoredDialog::new(
        AnchoredDialogKind::ShelveItems,
        format!("Shelve {count} {item_label}?"),
    );
    let regions = dialog.regions();
    dialog.show(
        ui.ctx(),
        DialogBody::new(|ui| {
            regions.frozen_at_open(
                ui,
                SHELVED_ITEMS_REGION,
                HeldBodyLines::what_the_content_took().and_at_most(SHELVED_ITEMS_MOST_LINES),
                |ui| {
                    let items: Vec<_> = tree
                        .shelve_confirm
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
                                    let dist = track.geometry.measured().map_or_else(
                                        || gt_ui_theme::EM_DASH.to_owned(),
                                        |geometry| gt_fmt::format_distance(geometry.distance_km),
                                    );
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
                },
            );
            ui.separator();
            let mut ticked = permanent.get();
            ui.checkbox(&mut ticked, PERMANENT_DELETE_LABEL);
            permanent.set(ticked);
            let track_label = gt_fmt::pluralize(affected_tracks, "track", "tracks");
            let rec_label = gt_fmt::pluralize(affected_recordings, "recording", "recordings");
            let detail = if ticked {
                format!(
                    "Permanently deletes {affected_tracks} {track_label} from \
                     {affected_recordings} {rec_label} in history and takes them out of the view."
                )
            } else {
                format!(
                    "Shelves {affected_tracks} {track_label} in {affected_recordings} {rec_label} \
                     in history and takes them out of the view."
                )
            };
            ui.label(RichText::new(detail).weak().small());
            regions.frozen_at_open(
                ui,
                ATTACHED_LOGS_REGION,
                HeldBodyLines::at_least(ATTACHED_LOGS_LINES),
                |ui| {
                    if attached_logs == 0 {
                        return;
                    }
                    let log_label = gt_fmt::pluralize(attached_logs, "log", "logs");
                    let line = if permanent.get() {
                        format!("Deletes {attached_logs} attached {log_label} with them.")
                    } else {
                        format!(
                            "Unloads {attached_logs} attached {log_label}. They come back when \
                             the recording is opened again."
                        )
                    };
                    ui.label(RichText::new(line).weak().small());
                },
            );
        }),
        DialogActionRow::buttons(|ui| {
            let confirm = if permanent.get() {
                destructive_button(ui, DELETE_PERMANENTLY_BUTTON_LABEL)
            } else {
                ui.button(SHELVE_BUTTON_LABEL)
            };
            if confirm.clicked() {
                do_confirm = true;
            }
            if ui.button("Cancel").clicked() {
                do_cancel = true;
            }
        }),
    );

    if do_cancel {
        tree.shelve_confirm = None;
        return None;
    }
    let permanent = permanent.get();
    if do_confirm {
        let items = tree
            .shelve_confirm
            .take()
            .map(|c| c.items)
            .unwrap_or_default();
        let affected = remove_items_from_view(&items, loaded_files, tree);
        let action = StoredTrackAction::from_permanent_delete_tickbox(permanent);
        return Some(ShelveOutcome {
            affected,
            action,
            removed_recordings,
        });
    }
    // Keep the checkbox state across frames while the dialog stays open.
    if let Some(c) = tree.shelve_confirm.as_mut() {
        c.delete_permanently = permanent;
    }
    None
}

/// The [`RecordingKey`] of every recording that taking `keys` out of the view
/// removes from the session.
pub fn removed_recording_keys(
    keys: &[NodeKey],
    loaded_files: LoadedFilesView<'_>,
) -> Vec<RecordingKey> {
    files_fully_removed(keys, loaded_files)
        .iter()
        .filter_map(|fi| loaded_files.get(*fi))
        .map(RecordingKey::of_loaded_recording)
        .collect()
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

/// For every removed track that belongs to a stored recording, the stored track
/// table rows to act on in history, grouped by recording.
///
/// A removed file contributes all of its tracks. Each row comes from the
/// track's `metadata.index`, which numbers a track by the stored row that it
/// sits in.
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
        let track_rows: Vec<usize> = positions
            .iter()
            .filter_map(|ti| file.tracks.get(*ti))
            // `metadata.index` is 1-based and the stored table is 0-based.
            .map(|t| t.metadata.index.saturating_sub(1))
            .collect();
        if !track_rows.is_empty() {
            removals.push(RecordingTrackRemoval { db_ref, track_rows });
        }
    }
    removals
}

/// Remove `keys` from the view and return, per stored recording, the tracks
/// that were removed. The rows are read before the view is mutated, while the
/// track indices still address the loaded tracks.
pub fn remove_items_from_view(
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

    tree.sync_from_loaded_files(loaded_files.view());
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
            dialog_body_above_the_action_row(
                ui,
                DialogBody::new(|ui| {
                    let ten_min = chrono::Duration::minutes(10);
                    let mut prev_ts: Option<DateTime<Utc>> = None;
                    for (ts, path) in orphans.iter() {
                        if let Some(prev) = prev_ts
                            && ts.signed_duration_since(prev) > ten_min
                        {
                            ui.separator();
                        }
                        let line = format!("{}  {}", ts.format(UTC_SECOND_FORMAT), path);
                        ui.add(Label::new(RichText::new(&line).monospace()).truncate());
                        prev_ts = Some(*ts);
                    }
                }),
                DialogActionRow::buttons(|ui| {
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                }),
            );
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
            dialog_body_above_the_action_row(
                ui,
                DialogBody::new(|ui| {
                    ui.add(Label::new(RichText::new(filename.as_str()).strong()).truncate());
                    ui.separator();
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
                }),
                DialogActionRow::buttons(|ui| {
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                }),
            );
        });

    if dismiss {
        *popup = None;
    }
}

/// Resizable dialog listing a recording's time range and recorded time, then
/// its metadata (title, device, identity, notes). Opened from a file row's note
/// icon. Sized generously so long identities and note paths read in full.
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
            ScrollArea::both().show(ui, |ui| {
                ui.add(
                    Label::new(RichText::new(details.metadata.filename.as_str()).strong())
                        .truncate(),
                );
                ui.separator();
                gt_side_panel::widgets::recording_time_detail_rows(ui, &details.metadata);
                let metadata_view = gt_side_panel::widgets::MetadataView::from_file_metadata(
                    &details.metadata,
                    details.identity.as_deref(),
                );
                // The separator introduces the metadata grid, so it shows only
                // where that grid has rows of its own.
                if gt_side_panel::widgets::has_metadata_details(&metadata_view) {
                    ui.separator();
                    gt_side_panel::widgets::metadata_detail_rows(ui, &metadata_view);
                }
            });
        });

    if !open || escape_pressed {
        *request = None;
    }
}

/// The region holding the version and the attributions.
const ABOUT_BODY_REGION: &str = "about_body";

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
    let dialog = AnchoredDialog::new(AnchoredDialogKind::AboutGeoTrace, "About GeoTrace")
        .with_close_button(&mut keep_open);
    let regions = dialog.regions();
    dialog.show(
        ui.ctx(),
        DialogBody::new(|ui| {
            regions.frozen_at_open(
                ui,
                ABOUT_BODY_REGION,
                HeldBodyLines::what_the_content_took(),
                |ui| {
                    // Selectable: the version is what a bug report quotes.
                    ui.add(
                        Label::new(RichText::new(format!("GeoTrace {version}")).strong())
                            .selectable(true),
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
                },
            );
        }),
        // The close button in the title bar and Escape dismiss the dialog,
        // which has nothing to act on.
        DialogActionRow::buttons(|_ui| {}),
    );

    if !keep_open || escape_pressed {
        *open = false;
    }
}

/// The region holding what the dialog states about the upload. The service
/// link shows only for the default host: pointing the setting at another
/// server while the dialog is open takes that line away.
const SNAP_CONSENT_BODY_REGION: &str = "snap_consent_body";

/// The user's decision in the snap upload-consent dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapConsentChoice {
    /// Uploads to the configured server's host are acknowledged. `auto_snap`
    /// holds the mode choice when the dialog asked the user for one (`None` =
    /// the choice was already made earlier and the dialog left it out).
    Accepted { auto_snap: Option<bool> },
    /// No acknowledgment. The manual snap action stays available and
    /// re-prompts. Auto mode turns off - a declined consent must never
    /// leave automatic uploads armed.
    Declined,
}

/// Show the one-time snap upload-consent dialog stating the configured server
/// and what is uploaded. Returns `None` while the dialog stays open.
///
/// Recorded location data leaves the machine, so nothing is ever sent before
/// this dialog has been accepted for the configured server's host (see
/// `SnapSettings::consent_granted`). With `ask_auto` the dialog also asks the
/// user whether tracks should snap automatically from now on - both agree
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

    // Enter agrees with the default mode: automatic when the dialog asks the
    // user for the choice (the design default), unchanged otherwise.
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
    let dialog = AnchoredDialog::new(AnchoredDialogKind::SnapToRoadConsent, "Snap to road")
        .with_close_button(&mut open);
    let regions = dialog.regions();
    dialog.show(
        ui.ctx(),
        DialogBody::new(|ui| {
            regions.frozen_at_open(
                ui,
                SNAP_CONSENT_BODY_REGION,
                HeldBodyLines::what_the_content_took(),
                |ui| {
                    ui.label(
                        "Snap to road matches a recorded track against the OpenStreetMap road \
                         network.",
                    );
                    ui.add_space(4.0);
                    ui.label("The track's recorded positions and timestamps are uploaded to");
                    ui.monospace(server_url);
                    // The service description only applies to the public FOSSGIS
                    // infrastructure. A self-hosted server has its own terms.
                    if gt_snap::server_host(server_url)
                        == gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL)
                    {
                        ui.hyperlink_to(
                            "Read more about the routing service",
                            gt_snap::SERVICE_INFO_URL,
                        );
                    }
                    ui.add_space(4.0);
                    ui.label(
                        "Nothing is uploaded until you agree. The acknowledgment is remembered \
                         for this server and asked again when the server changes.",
                    );
                    if ask_auto {
                        ui.add_space(4.0);
                        ui.label(
                            "Snapping can run automatically: every track you load and show on \
                             the map is uploaded and matched without a click. Manual only \
                             uploads a track when you trigger it. Changeable anytime in the \
                             settings.",
                        );
                    }
                },
            );
        }),
        DialogActionRow::buttons(|ui| {
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
        }),
    );
    if !open {
        choice = Some(SnapConsentChoice::Declined);
    }
    choice
}

/// The region stating what snapping again does to the stored result.
const SNAP_REPLACE_BODY_REGION: &str = "snap_replace_body";

/// The user's decision in the replace-cached-run confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapReplaceChoice {
    /// Run against the server again, replacing the stored result.
    SnapAgain,
    /// Keeps the stored result: nothing is uploaded.
    Cancel,
}

/// Ask the user before a "Snap again as" choice replaces the result this
/// track already has for `costing_name`. Returns `None` while the dialog
/// stays open.
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
    let dialog = AnchoredDialog::new(AnchoredDialogKind::SnapToRoadAgain, "Snap to road again?")
        .with_close_button(&mut open);
    let regions = dialog.regions();
    dialog.show(
        ui.ctx(),
        DialogBody::new(|ui| {
            regions.frozen_at_open(
                ui,
                SNAP_REPLACE_BODY_REGION,
                HeldBodyLines::what_the_content_took(),
                |ui| {
                    ui.label(format!(
                        "This track already has snap to road data for {costing_name}."
                    ));
                    ui.add_space(4.0);
                    ui.label(
                        "Snapping again uploads the track once more and replaces that result \
                         with the new one.",
                    );
                },
            );
        }),
        DialogActionRow::buttons(|ui| {
            if ui.button("Snap again").clicked() {
                choice = Some(SnapReplaceChoice::SnapAgain);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(SnapReplaceChoice::Cancel);
            }
        }),
    );
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

/// The region stating that snapping again replaces the tracks' existing data.
/// A snap result that arrives while the dialog is open fills it.
const REPLACED_DATA_REGION: &str = "snap_scope_replaced_data";

/// Lines the [`REPLACED_DATA_REGION`] holds from the frame the dialog opens,
/// which is the two lines that statement wraps onto at this width.
const REPLACED_DATA_LINES: u8 = 2;

/// Ask the user which of a recording's tracks a "Snap again as" choice
/// covers, and how many of them already have data for `costing_name`.
/// Returns `None` while the dialog stays open.
///
/// Escape, Cancel, and the close button all drop the choice: nothing is
/// uploaded.
pub fn show_snap_scope_dialog(
    ui: &egui::Ui,
    costing_name: &str,
    counts: SnapScopeCounts,
) -> Option<SnapScopeChoice> {
    let mut choice = consume_escape_press(ui.ctx(), SnapScopeChoice::Cancel);

    let mut open = true;
    let dialog = AnchoredDialog::new(
        AnchoredDialogKind::SnapToRoadScope,
        format!("Snap to road as {costing_name}"),
    )
    .with_close_button(&mut open);
    let regions = dialog.regions();
    dialog.show(
        ui.ctx(),
        DialogBody::new(|ui| {
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
            ui.add_space(4.0);
            regions.frozen_at_open(
                ui,
                REPLACED_DATA_REGION,
                HeldBodyLines::at_least(REPLACED_DATA_LINES),
                |ui| {
                    if counts.all.already_snapped == 0 {
                        return;
                    }
                    ui.add(
                        Label::new(
                            "Snapping again uploads those tracks once more and replaces their \
                             data.",
                        )
                        .wrap(),
                    );
                },
            );
        }),
        DialogActionRow::buttons(|ui| {
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
        }),
    );
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

/// The region holding what the prompt states about automatic snapping and the
/// server it uploads to.
const SNAP_AUTO_PROMPT_BODY_REGION: &str = "snap_auto_prompt_body";

/// The user's decision in the one-time auto-snap prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapAutoChoice {
    /// Loaded tracks snap automatically from now on.
    Automatic,
    /// Snapping stays click-triggered.
    ManualOnly,
}

/// One-time prompt for users who acknowledged uploads before auto mode
/// existed: asks the user whether loaded tracks should snap automatically
/// from now on. Returns `None` while the dialog stays open.
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
    let dialog = AnchoredDialog::new(
        AnchoredDialogKind::SnapToRoadAutomatically,
        "Snap to road automatically?",
    )
    .with_close_button(&mut open);
    let regions = dialog.regions();
    dialog.show(
        ui.ctx(),
        DialogBody::new(|ui| {
            regions.frozen_at_open(
                ui,
                SNAP_AUTO_PROMPT_BODY_REGION,
                HeldBodyLines::what_the_content_took(),
                |ui| {
                    ui.label(
                        "Snap to road can run automatically: every track you load and show on \
                         the map is uploaded and matched without a click.",
                    );
                    ui.add_space(4.0);
                    ui.label("Your earlier acknowledgment still applies; uploads go to");
                    ui.monospace(server_url);
                    ui.add_space(4.0);
                    ui.label("Changeable anytime in the settings.");
                },
            );
        }),
        DialogActionRow::buttons(|ui| {
            if ui.button("Snap automatically").clicked() {
                choice = Some(SnapAutoChoice::Automatic);
            }
            if ui.button("Manual only").clicked() {
                choice = Some(SnapAutoChoice::ManualOnly);
            }
        }),
    );
    if !open {
        choice = Some(SnapAutoChoice::ManualOnly);
    }
    choice
}

/// The region holding the token field and the two lines above it.
const MAPBOX_TOKEN_BODY_REGION: &str = "mapbox_token_body";

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
    let dialog = AnchoredDialog::new(AnchoredDialogKind::MapboxToken, "Mapbox API Token Required")
        .with_close_button(&mut open);
    let regions = dialog.regions();
    let cancelled = dialog.show(
        ui.ctx(),
        DialogBody::new(|ui| {
            regions.frozen_at_open(
                ui,
                MAPBOX_TOKEN_BODY_REGION,
                HeldBodyLines::what_the_content_took(),
                |ui| {
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
                },
            );
        }),
        DialogActionRow::buttons(|ui| ui.button("Cancel - use OpenStreetMap").clicked()),
    );

    // The X button in the title bar is a cancel too.
    if !open || cancelled == Some(true) {
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

const DELETE_ARCHIVED_DAYS_TITLE: &str = "Delete archived days?";

/// The region listing the loaded recordings. A recording that finishes loading
/// while the dialog is open joins that list.
const LOADED_RECORDINGS_REGION: &str = "environment_prune_loaded_recordings";

/// Lines the [`LOADED_RECORDINGS_REGION`] holds from the frame the dialog
/// opens. Three is the fewest that hold the note above the names and the
/// first name under it.
const LOADED_RECORDINGS_LINES: u8 = 3;

/// Lines the [`LOADED_RECORDINGS_REGION`] holds at most, however many
/// recordings are loaded: the rest of the names scroll inside it.
const LOADED_RECORDINGS_MOST_LINES: u8 = 9;

/// Confirm an environment-data delete, stating what goes and which loaded
/// recordings are downloaded again straight after.
///
/// Returns the choice in the frame the user makes it, and [`None`] while the
/// dialog is still open.
pub fn show_environment_prune_confirmation(
    ui: &egui::Ui,
    prompt: &EnvironmentPrunePrompt<'_>,
) -> Option<EnvironmentPruneChoice> {
    anchored_confirmation_dialog(
        ui,
        AnchoredDialogKind::DeleteArchivedDays,
        DELETE_ARCHIVED_DAYS_TITLE,
        EnvironmentPruneChoice::Cancel,
        |ui, regions| {
            ui.label(prune_scope_line(prompt.request));
            ui.add_space(4.0);
            Grid::new("environment_prune_grid")
                .num_columns(2)
                .spacing([12.0, 2.0])
                .show(ui, |ui| {
                    for archive in EnvironmentArchive::iter()
                        .filter(|archive| prompt.request.scope.covers(*archive))
                    {
                        let days = prompt.covered[archive];
                        ui.label(archive.label());
                        ui.label(format!("{days} {}", gt_fmt::pluralize(days, "day", "days")));
                        ui.end_row();
                    }
                });

            regions.frozen_at_open(
                ui,
                LOADED_RECORDINGS_REGION,
                HeldBodyLines::at_least(LOADED_RECORDINGS_LINES)
                    .and_at_most(LOADED_RECORDINGS_MOST_LINES),
                |ui| {
                    if prompt.loaded_recordings.is_empty() {
                        return;
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "These loaded recordings span days in that range. Those days are \
                             downloaded again as soon as the delete finishes.",
                        )
                        .weak()
                        .small(),
                    );
                    for name in prompt.loaded_recordings {
                        ui.add(Label::new(name.as_str()).truncate());
                    }
                },
            );
        },
        |ui| {
            let mut choice = None;
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
            choice
        },
    )
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
    /// Close the confirmation and leave the process running.
    Dismiss,
}

/// Whether the pointer rests over a dialog's own window this frame.
///
/// A caller that closes a dialog on its own reads this first: closing one out
/// from under the pointer sends the press that follows to whatever the dialog
/// was drawn over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerOverTheDialog {
    Resting,
    Away,
}

impl PointerOverTheDialog {
    /// Where the pointer is against the layer `ui` draws in, which for a
    /// dialog's body is the dialog's own window.
    pub(super) fn of(ui: &egui::Ui) -> Self {
        let over_this_layer = ui
            .ctx()
            .pointer_latest_pos()
            .and_then(|pos| ui.ctx().layer_id_at(pos))
            .is_some_and(|layer| layer == ui.layer_id());
        if over_this_layer {
            Self::Resting
        } else {
            Self::Away
        }
    }
}

/// What the force-quit confirmation shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForceQuitPromptContents {
    /// What interrupting each running write costs, one line each.
    InterruptionCosts(Vec<String>),
    /// Every write the confirmation listed has finished.
    WritesFinished(TimeUntilTheClose),
}

/// How long a dialog reporting that its action has nothing left to do has
/// before it closes itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeUntilTheClose(pub Duration);

impl TimeUntilTheClose {
    /// The label counts the whole seconds left, rounded up, so it reads `1s`
    /// through the whole last second of the count.
    pub(super) fn close_button_label(self) -> String {
        let seconds = self.0.as_secs() + u64::from(self.0.subsec_nanos() > 0);
        format!("Close ({seconds}s)")
    }
}

/// A dialog that reports its action has nothing left to do runs this count
/// before it closes itself, which is long enough to read the one sentence it
/// reports.
pub(super) const COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES: Duration = Duration::from_secs(4);

/// What a dialog reporting that its action has nothing left to do has left
/// before it closes itself, and the frame that time was last taken off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CountdownToTheClose {
    time_left: Duration,
    last_advanced_at: Instant,
}

impl CountdownToTheClose {
    /// The count moves without new input. A dialog counting down repaints on
    /// this interval.
    const REPAINT_INTERVAL: Duration = Duration::from_millis(100);

    pub(super) fn started_at(now: Instant) -> Self {
        Self {
            time_left: COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES,
            last_advanced_at: now,
        }
    }

    pub(super) fn time_until_the_close(self) -> TimeUntilTheClose {
        TimeUntilTheClose(self.time_left)
    }

    /// Takes the time since the last frame off the count, and holds the count
    /// where it is while the pointer rests over the dialog.
    pub(super) fn advance_to(&mut self, now: Instant, pointer: PointerOverTheDialog) {
        let since_the_last_frame = now.saturating_duration_since(self.last_advanced_at);
        self.last_advanced_at = now;
        if pointer == PointerOverTheDialog::Away {
            self.time_left = self.time_left.saturating_sub(since_the_last_frame);
        }
    }

    pub(super) fn has_run_out(self) -> bool {
        self.time_left.is_zero()
    }

    /// A dialog counting down calls this on every frame it is up.
    pub(super) fn request_the_repaint_the_count_needs(self, ctx: &egui::Context) {
        ctx.request_repaint_after(Self::REPAINT_INTERVAL);
    }
}

/// The region listing what each running write costs. The dialog drops the line
/// for a write that finishes while it is open.
const INTERRUPTION_COSTS_REGION: &str = "force_quit_interruption_costs";

/// egui derives the confirmation's window id from its title, and one window
/// draws both of [`ForceQuitPromptContents`]: the title stays put while the
/// body and the actions change.
const FORCE_QUIT_TITLE: &str = "Force quit?";

const FORCE_QUIT_LABEL: &str = "Force quit";

const CLOSE_BUTTON_HOVER: &str = "Closes GeoTrace now. GeoTrace closes on its own when the count \
                                  reaches zero. The count holds while the pointer is over this \
                                  window.";

/// What the force-quit confirmation reports for the frame it drew.
pub struct ForceQuitPromptResponse {
    /// The choice in the frame the user makes it, and [`None`] on every other
    /// frame the confirmation is up.
    pub choice: Option<ForceQuitChoice>,
    pub pointer: PointerOverTheDialog,
}

/// Confirm ending the process with the running writes unfinished, listing what
/// each one costs, or report that every one of them finished.
pub fn show_force_quit_confirmation(
    ui: &egui::Ui,
    contents: &ForceQuitPromptContents,
) -> ForceQuitPromptResponse {
    let mut pointer = PointerOverTheDialog::Away;
    let choice = anchored_confirmation_dialog(
        ui,
        AnchoredDialogKind::ForceQuit,
        FORCE_QUIT_TITLE,
        ForceQuitChoice::Dismiss,
        |ui, regions| {
            pointer = PointerOverTheDialog::of(ui);
            match contents {
                ForceQuitPromptContents::InterruptionCosts(_) => {
                    ui.label("GeoTrace ends now, with the work it is still doing unfinished");
                }
                ForceQuitPromptContents::WritesFinished(_) => {
                    ui.label("The work finished: there is nothing left to interrupt");
                }
            }
            ui.add_space(4.0);
            regions.frozen_at_open(
                ui,
                INTERRUPTION_COSTS_REGION,
                HeldBodyLines::what_the_content_took(),
                |ui| {
                    let ForceQuitPromptContents::InterruptionCosts(costs) = contents else {
                        return;
                    };
                    for cost in costs {
                        ui.add(Label::new(cost.as_str()).wrap());
                    }
                },
            );
        },
        |ui| {
            let mut choice = None;
            let dismiss = match contents {
                ForceQuitPromptContents::InterruptionCosts(_) => {
                    if destructive_button(ui, FORCE_QUIT_LABEL).clicked() {
                        choice = Some(ForceQuitChoice::Quit);
                    }
                    ui.button("Cancel")
                }
                ForceQuitPromptContents::WritesFinished(time_until_the_close) => {
                    ui.add_enabled(false, Button::new(FORCE_QUIT_LABEL))
                        .on_disabled_hover_text("The work has finished");
                    ui.button(time_until_the_close.close_button_label())
                        .on_hover_text(CLOSE_BUTTON_HOVER)
                }
            };
            if dismiss.clicked() {
                choice = Some(ForceQuitChoice::Dismiss);
            }
            choice
        },
    );
    ForceQuitPromptResponse { choice, pointer }
}

#[cfg(test)]
mod tests {
    //! A harness that changes what it renders between frames holds that input
    //! in a `RefCell` and reads it every frame.

    use std::cell::RefCell;
    use std::path::PathBuf;

    use chrono::DateTime;
    use gt_types::{
        FileIdx, FileSource, GeneratedMarkerKindTag, LoadedFile, LoadedTrack, TimeRange,
        TotalDistance, TrackIdx, TrackMetadata,
    };
    use uom::si::f64::Length;
    use uom::si::length::kilometer;

    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_map::TileAccess;
    use gt_pending_writes::WriteKind;
    use gt_store::{DatabaseRef, TrackState};
    use gt_test_utils::window_fit::{
        CRAMPED_VIEWPORT, NARROW_VIEWPORT, OVERSIZED_ROW_COUNT, SHORT_VIEWPORT,
    };
    use gt_test_utils::{
        AuditedWindow, ControlLabel, HarnessInteraction as _, TestHarness, WindowFitAssertions as _,
    };
    use rustc_hash::FxHashMap;

    use super::{
        CoveredDayCounts, DELETE_ARCHIVED_DAYS_TITLE, DELETE_PERMANENTLY_BUTTON_LABEL, Duration,
        EnvironmentArchive, EnvironmentPruneChoice, EnvironmentPrunePrompt, FORCE_QUIT_LABEL,
        ForceQuitChoice, ForceQuitPromptContents, LoadedLogs, MapLayer, MapboxTokenField, NavMap,
        NodeKey, PruneRequest, PruneScope, PrunedDays, RecordingDetails, SHELVE_BUTTON_LABEL,
        SHELVED_ITEMS_MOST_LINES, ShelveOutcome, SnapScopeChoice, SnapScopeCount, SnapScopeCounts,
        StoredTrackAction, TimeUntilTheClose, TrackRef, files_fully_removed, prune_scope_line,
        remove_items_from_view, show_about_dialog, show_environment_prune_confirmation,
        show_force_quit_confirmation, show_load_warnings_dialog, show_mapbox_token_dialog,
        show_orphaned_event_markers_popup, show_recording_details_dialog, show_shelve_confirmation,
        show_snap_auto_prompt, show_snap_consent_dialog, show_snap_replace_dialog,
        show_snap_scope_dialog, track_removals,
    };
    use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
    use gt_side_panel::tree::CheckState;
    use gt_side_panel::{ShelveConfirmState, TreeState};

    use crate::app::history_db::{DbOp, HistoryWorker, Response};
    use crate::app::history_test_support::{
        next_response, only_recording, seed_recording_cut_at, worker_on,
    };

    fn day(offset: i64) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap_or_default()
            + chrono::TimeDelta::days(offset)
    }

    pub(super) fn prune_request(scope: PruneScope) -> PruneRequest {
        PruneRequest {
            scope,
            days: PrunedDays::Before(day(0)),
        }
    }

    pub(super) fn covered_days() -> CoveredDayCounts {
        let mut covered = CoveredDayCounts::default();
        covered[EnvironmentArchive::AircraftInterference] = 2;
        covered[EnvironmentArchive::GeomagneticIndices] = 3;
        covered[EnvironmentArchive::IonosphericTec] = 29;
        covered[EnvironmentArchive::SolarFlares] = 1;
        covered
    }

    /// Every dialog opened by these tests is drawn at its full width, with no
    /// control clipped by the screen. This is wider and taller than all of
    /// them.
    pub(super) const DIALOG_VIEWPORT: egui::Vec2 = egui::vec2(640.0, 480.0);

    /// Renders the confirmation and reports the choice it took.
    pub(super) fn prune_dialog<'a>(
        scope: PruneScope,
        loaded_recordings: &'a RefCell<Vec<String>>,
        choice: &'a RefCell<Option<EnvironmentPruneChoice>>,
    ) -> TestHarness<'a, ()> {
        let request = prune_request(scope);
        let covered = covered_days();
        let mut harness = TestHarness::builder().size(DIALOG_VIEWPORT).ui(move |ui| {
            let listed = loaded_recordings.borrow();
            let prompt = EnvironmentPrunePrompt {
                request,
                covered,
                loaded_recordings: &listed,
            };
            if let Some(made) = show_environment_prune_confirmation(ui, &prompt) {
                *choice.borrow_mut() = Some(made);
            }
        });
        harness.inner.run_steps(4);
        harness
    }

    /// Renders the snap scope dialog and reports the choice it took.
    pub(super) fn snap_scope_dialog<'a>(
        counts: &'a RefCell<SnapScopeCounts>,
        choice: &'a RefCell<Option<SnapScopeChoice>>,
    ) -> TestHarness<'a, ()> {
        let mut harness = TestHarness::builder().size(DIALOG_VIEWPORT).ui(move |ui| {
            if let Some(made) = show_snap_scope_dialog(ui, SNAP_COSTING, *counts.borrow()) {
                *choice.borrow_mut() = Some(made);
            }
        });
        harness.inner.run_steps(4);
        harness
    }

    /// The costing the snap scope dialog is opened for.
    pub(super) const SNAP_COSTING: &str = "car";

    /// Four tracks, none of them snapped as [`SNAP_COSTING`] yet, one of them
    /// selected in the panel.
    pub(super) fn nothing_snapped_yet() -> SnapScopeCounts {
        SnapScopeCounts {
            selected: SnapScopeCount {
                tracks: 1,
                already_snapped: 0,
            },
            all: SnapScopeCount {
                tracks: 4,
                already_snapped: 0,
            },
        }
    }

    /// The dialog lists the loaded recordings whose days go: the fetch
    /// schedulers download those days again straight after.
    #[test]
    fn the_prune_dialog_names_the_loaded_recordings_it_affects() {
        let recordings = RefCell::new(vec!["Morning ride".to_owned(), "Ferry crossing".to_owned()]);
        let choice = RefCell::new(None);
        let harness = prune_dialog(PruneScope::Every, &recordings, &choice);

        for name in recordings.borrow().iter() {
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
        let no_recording_loaded = RefCell::new(Vec::new());
        let choice = RefCell::new(None);
        let mut harness = prune_dialog(PruneScope::Every, &no_recording_loaded, &choice);
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
        let no_recording_loaded = RefCell::new(Vec::new());
        let choice = RefCell::new(None);
        let mut harness = prune_dialog(PruneScope::Every, &no_recording_loaded, &choice);
        harness.inner.get_by_label("Cancel").click();
        harness.run();
        drop(harness);

        assert!(matches!(
            choice.into_inner(),
            Some(EnvironmentPruneChoice::Cancel)
        ));
    }

    #[test]
    fn escape_cancels_the_prune_dialog() {
        let no_recording_loaded = RefCell::new(Vec::new());
        let choice = RefCell::new(None);
        let mut harness = prune_dialog(PruneScope::Every, &no_recording_loaded, &choice);
        harness.inner.key_press(egui::Key::Escape);
        harness.run();
        drop(harness);

        assert!(matches!(
            choice.into_inner(),
            Some(EnvironmentPruneChoice::Cancel)
        ));
    }

    /// Renders the force-quit confirmation and reports the choice it took.
    fn force_quit_dialog<'a>(
        interruption_costs: &'a RefCell<Vec<String>>,
        choice: &'a RefCell<Option<ForceQuitChoice>>,
    ) -> TestHarness<'a, ()> {
        force_quit_dialog_over(interruption_costs, choice, |_| {})
    }

    /// [`force_quit_dialog`] with `shutdown_window` drawn behind the
    /// confirmation: a press that misses the confirmation reaches it.
    pub(super) fn force_quit_dialog_over<'a>(
        interruption_costs: &'a RefCell<Vec<String>>,
        choice: &'a RefCell<Option<ForceQuitChoice>>,
        mut shutdown_window: impl FnMut(&mut egui::Ui) + 'a,
    ) -> TestHarness<'a, ()> {
        let mut harness = TestHarness::builder().size(DIALOG_VIEWPORT).ui(move |ui| {
            shutdown_window(ui);
            let contents =
                ForceQuitPromptContents::InterruptionCosts(interruption_costs.borrow().clone());
            if let Some(made) = show_force_quit_confirmation(ui, &contents).choice {
                *choice.borrow_mut() = Some(made);
            }
        });
        harness.run();
        harness
    }

    const TIME_UNTIL_THE_CLOSE: TimeUntilTheClose = TimeUntilTheClose(Duration::from_secs(4));

    fn force_quit_dialog_once_the_writes_finish(
        choice: &RefCell<Option<ForceQuitChoice>>,
    ) -> TestHarness<'_, ()> {
        let mut harness = TestHarness::builder().size(DIALOG_VIEWPORT).ui(move |ui| {
            let response = show_force_quit_confirmation(
                ui,
                &ForceQuitPromptContents::WritesFinished(TIME_UNTIL_THE_CLOSE),
            );
            if let Some(made) = response.choice {
                *choice.borrow_mut() = Some(made);
            }
        });
        harness.run();
        harness
    }

    #[rstest::rstest]
    #[case::the_whole_count(Duration::from_secs(4), "Close (4s)")]
    #[case::part_way_through_a_second(Duration::from_millis(2500), "Close (3s)")]
    #[case::the_last_moment_of_the_count(Duration::from_millis(1), "Close (1s)")]
    fn the_close_button_counts_the_seconds_left(
        #[case] time_until_the_close: Duration,
        #[case] expected: &str,
    ) {
        assert_eq!(
            TimeUntilTheClose(time_until_the_close).close_button_label(),
            expected
        );
    }

    #[test]
    fn confirming_the_force_quit_dialog_reports_the_quit() {
        let costs = RefCell::new(vec![WriteKind::Settings.interruption_cost()]);
        let choice = RefCell::new(None);
        let mut harness = force_quit_dialog(&costs, &choice);
        harness.inner.get_by_label(FORCE_QUIT_LABEL).click();
        harness.run();
        drop(harness);

        assert!(matches!(choice.into_inner(), Some(ForceQuitChoice::Quit)));
    }

    #[test]
    fn cancelling_the_force_quit_dialog_quits_nothing() {
        let costs = RefCell::new(vec![WriteKind::Settings.interruption_cost()]);
        let choice = RefCell::new(None);
        let mut harness = force_quit_dialog(&costs, &choice);
        harness.inner.get_by_label("Cancel").click();
        harness.run();
        drop(harness);

        assert!(matches!(
            choice.into_inner(),
            Some(ForceQuitChoice::Dismiss)
        ));
    }

    #[test]
    fn escape_cancels_the_force_quit_dialog() {
        let costs = RefCell::new(vec![WriteKind::Settings.interruption_cost()]);
        let choice = RefCell::new(None);
        let mut harness = force_quit_dialog(&costs, &choice);
        harness.inner.key_press(egui::Key::Escape);
        harness.run();
        drop(harness);

        assert!(matches!(
            choice.into_inner(),
            Some(ForceQuitChoice::Dismiss)
        ));
    }

    #[test]
    fn the_force_quit_dialog_grays_its_quit_out_once_the_writes_finish() {
        let choice = RefCell::new(None);
        let harness = force_quit_dialog_once_the_writes_finish(&choice);

        assert!(
            harness
                .inner
                .get_by_label(FORCE_QUIT_LABEL)
                .accesskit_node()
                .is_disabled()
        );
    }

    #[test]
    fn closing_the_force_quit_dialog_once_the_writes_finish_quits_nothing() {
        let choice = RefCell::new(None);
        let mut harness = force_quit_dialog_once_the_writes_finish(&choice);
        harness
            .inner
            .get_by_label(&TIME_UNTIL_THE_CLOSE.close_button_label())
            .click();
        harness.run();
        drop(harness);

        assert!(matches!(
            choice.into_inner(),
            Some(ForceQuitChoice::Dismiss)
        ));
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
        let recordings = RefCell::new(vec!["Morning ride".to_owned(), "Ferry crossing".to_owned()]);
        let choice = RefCell::new(None);
        let mut harness = prune_dialog(PruneScope::Every, &recordings, &choice);
        harness.snapshot("environment_prune_dialog");
    }

    #[test]
    fn snapshot_the_prune_dialog_while_no_recording_is_named() {
        let no_recording_loaded = RefCell::new(Vec::new());
        let choice = RefCell::new(None);
        let mut harness = prune_dialog(PruneScope::Every, &no_recording_loaded, &choice);
        harness.snapshot("environment_prune_dialog_no_recording_named");
    }

    /// The confirmation as the user meets it during shutdown, listing what
    /// each running write costs.
    #[test]
    fn snapshot_the_force_quit_confirmation() {
        let costs = RefCell::new(vec![
            WriteKind::Settings.interruption_cost(),
            WriteKind::RecordingDatabase.interruption_cost(),
            WriteKind::DatabaseOpen.interruption_cost(),
            WriteKind::TakeOverRecord.interruption_cost(),
        ]);
        let choice = RefCell::new(None);
        let mut harness = force_quit_dialog(&costs, &choice);
        harness.snapshot("force_quit_confirmation");
    }

    #[test]
    fn snapshot_the_force_quit_confirmation_once_the_writes_finish() {
        let choice = RefCell::new(None);
        let mut harness = force_quit_dialog_once_the_writes_finish(&choice);
        harness.snapshot("force_quit_confirmation_writes_finished");
    }

    #[test]
    fn snapshot_the_snap_scope_dialog_while_nothing_is_snapped() {
        let counts = RefCell::new(nothing_snapped_yet());
        let choice = RefCell::new(None);
        let mut harness = snap_scope_dialog(&counts, &choice);
        harness.snapshot("snap_to_road_scope_dialog_nothing_snapped");
    }

    /// The confirmation the user meets when a track already has a run for the
    /// costing they chose.
    #[test]
    fn snapshot_the_snap_replace_confirmation() {
        let mut harness = TestHarness::builder().size(DIALOG_VIEWPORT).ui(|ui| {
            show_snap_replace_dialog(ui, SNAP_COSTING);
        });
        harness.inner.run_steps(4);
        harness.snapshot("snap_replace_confirmation");
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

    /// The dialog the satellite layer shows without a token.
    #[test]
    fn snapshot_the_mapbox_token_dialog() {
        let mut harness = token_dialog();
        harness.inner.run_steps(4);
        harness.snapshot("mapbox_token_dialog");
    }

    /// The dialog states the time range and the recorded time apart for a
    /// recording that idled between its tracks.
    #[test]
    fn the_recording_details_dialog_states_the_time_range_and_the_recorded_time() {
        let mut metadata = gt_test_utils::empty_file_metadata();
        metadata.filename = "paused.gtd".to_owned();
        metadata.time_range = Some(TimeRange::new(
            chrono::DateTime::UNIX_EPOCH + chrono::TimeDelta::hours(12),
            chrono::DateTime::UNIX_EPOCH + chrono::TimeDelta::minutes(18 * 60 + 30),
        ));
        metadata.total_duration = chrono::TimeDelta::minutes(88);
        let mut harness = TestHarness::builder()
            .size(egui::vec2(520.0, 320.0))
            .ui_state(
                |ui, request: &mut Option<RecordingDetails>| {
                    show_recording_details_dialog(ui, request);
                },
                Some(RecordingDetails {
                    metadata,
                    identity: None,
                }),
            );
        harness.run();

        assert!(
            harness
                .inner
                .query_by_label("1970-01-01 12:00:00 – 18:30:00")
                .is_some(),
            "the dialog does not state the range the recording covers"
        );
        assert!(
            harness.inner.query_by_label("1h28m").is_some(),
            "the dialog does not state the recorded time its tracks hold"
        );
    }

    fn make_file(filename: String, track_count: usize) -> LoadedFile {
        let metadata = gt_types::FileMetadata {
            filename,
            ..gt_test_utils::empty_file_metadata()
        };
        LoadedFile {
            metadata,
            tracks: (0..track_count)
                .map(|ti| LoadedTrack {
                    metadata: TrackMetadata {
                        // 1-based display index, as `build_loaded_file` assigns.
                        index: ti + 1,
                        ..gt_test_utils::empty_track_metadata()
                    },
                    ..gt_test_utils::loaded_track_with_points(Vec::new())
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
                        time_range: gt_store::NavPointTimeRange::covering(&[idx as i64]),
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
            loaded.push(make_file(format!("ride-{idx}.gtd"), track_count), history);
        }
        loaded
    }

    /// The shelve confirmation over the tracks of one stored recording, and
    /// what it reported on the frame the user confirmed it.
    pub(super) struct ShelveConfirmationState {
        tree: TreeState,
        loaded_files: LoadedFiles,
        outcome: Option<ShelveOutcome>,
    }

    fn shelve_confirmation_ui(ui: &mut egui::Ui, state: &mut ShelveConfirmationState) {
        let names = RecordingNames::resolve(state.loaded_files.view(), "{filename}");
        let outcome = show_shelve_confirmation(
            ui,
            &mut state.tree,
            &mut state.loaded_files,
            &names,
            &LoadedLogs::default(),
        );
        if outcome.is_some() {
            state.outcome = outcome;
        }
    }

    /// Whether the confirmation opens with [`PERMANENT_DELETE_LABEL`] ticked.
    #[derive(Clone, Copy)]
    pub(super) struct PermanentDeleteTicked(pub(super) bool);

    /// The confirmation over `count` tracks of one stored recording, at
    /// `viewport`.
    pub(super) fn shelve_confirmation_at(
        viewport: egui::Vec2,
        count: usize,
        PermanentDeleteTicked(delete_permanently): PermanentDeleteTicked,
    ) -> TestHarness<'static, ShelveConfirmationState> {
        let mut tree = TreeState::default();
        tree.shelve_confirm = Some(ShelveConfirmState {
            items: (0..count).map(|ti| track_key(0, ti)).collect(),
            delete_permanently,
        });
        let mut harness = TestHarness::builder().size(viewport).ui_state(
            shelve_confirmation_ui,
            ShelveConfirmationState {
                tree,
                loaded_files: make_loaded_files(&[(count + 1, true)]),
                outcome: None,
            },
        );
        harness.inner.run_steps(4);
        harness
    }

    fn shelve_confirmation(count: usize) -> TestHarness<'static, ShelveConfirmationState> {
        shelve_confirmation_at(DIALOG_VIEWPORT, count, PermanentDeleteTicked(true))
    }

    /// The rectangle of the confirmation's window over `count` items, which
    /// egui identifies by the title stating that count.
    pub(super) fn shelve_confirmation_rect(
        harness: &TestHarness<'static, ShelveConfirmationState>,
        count: usize,
    ) -> egui::Rect {
        harness
            .inner
            .window_rect(&format!("Shelve {count} items?"))
            .expect("the shelve confirmation is shown")
    }

    /// The confirmation shows the shelve title with its permanent-delete
    /// tickbox ticked and with it clear. The button states the level that the
    /// tickbox chose, and the outcome reports that level.
    #[rstest::rstest]
    #[case(
        PermanentDeleteTicked(false),
        SHELVE_BUTTON_LABEL,
        StoredTrackAction::Shelve
    )]
    #[case(
        PermanentDeleteTicked(true),
        DELETE_PERMANENTLY_BUTTON_LABEL,
        StoredTrackAction::DeletePermanently
    )]
    fn the_shelve_confirmation_applies_the_action_that_its_tickbox_chose(
        #[case] ticked: PermanentDeleteTicked,
        #[case] button_label: &str,
        #[case] expected: StoredTrackAction,
    ) {
        const ITEMS: usize = 2;
        let mut harness = shelve_confirmation_at(DIALOG_VIEWPORT, ITEMS, ticked);

        harness
            .inner
            .get_by_label_contains(&format!("Shelve {ITEMS} items?"));
        harness.inner.get_by_label(button_label).click();
        harness.inner.run_steps(2);

        assert_eq!(
            harness.inner.state().outcome.as_ref().map(|o| o.action),
            Some(expected)
        );
    }

    #[test]
    fn snapshot_the_shelve_confirmation() {
        let mut harness = shelve_confirmation(2);
        harness.snapshot("shelve_confirmation");
    }

    /// Items enough to fill the room the list caps at
    /// [`SHELVED_ITEMS_MOST_LINES`].
    const ITEMS_PAST_THE_CAPPED_ROOM: usize = 12;

    const ITEMS_FAR_PAST_THE_CAPPED_ROOM: usize = 40;

    /// The list past the room it caps at, scrolling inside that room.
    #[test]
    fn snapshot_the_shelve_confirmation_past_the_capped_room() {
        let mut harness = shelve_confirmation(ITEMS_FAR_PAST_THE_CAPPED_ROOM);
        harness.snapshot("shelve_confirmation_past_the_capped_room");
    }

    #[test]
    fn the_shelve_confirmation_opens_at_one_height_for_every_list_past_the_capped_room() {
        let past = shelve_confirmation(ITEMS_PAST_THE_CAPPED_ROOM);
        let far_past = shelve_confirmation(ITEMS_FAR_PAST_THE_CAPPED_ROOM);

        assert_eq!(
            shelve_confirmation_rect(&far_past, ITEMS_FAR_PAST_THE_CAPPED_ROOM).size(),
            shelve_confirmation_rect(&past, ITEMS_PAST_THE_CAPPED_ROOM).size(),
            "{ITEMS_FAR_PAST_THE_CAPPED_ROOM} removed items made the confirmation taller than \
             {ITEMS_PAST_THE_CAPPED_ROOM} did: a list past the room it caps at \
             {SHELVED_ITEMS_MOST_LINES} lines has to scroll inside that room"
        );
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
            /// Stored track rows removed per affected recording (ascending).
            expect_track_rows: Vec<Vec<usize>>,
        }

        let cases = [
            Case {
                name: "no keys removes nothing",
                files: vec![(2, true)],
                keys: vec![],
                expect_removed: vec![],
                expect_recordings: 0,
                expect_track_rows: vec![],
            },
            Case {
                name: "file key removes every track of the file",
                files: vec![(2, true)],
                keys: vec![file_key(0)],
                expect_removed: vec![0],
                expect_recordings: 1,
                expect_track_rows: vec![vec![0, 1]],
            },
            Case {
                name: "all tracks selected promotes to full removal",
                files: vec![(2, true)],
                keys: vec![track_key(0, 0), track_key(0, 1)],
                expect_removed: vec![0],
                expect_recordings: 1,
                expect_track_rows: vec![vec![0, 1]],
            },
            Case {
                name: "partial track selection hides just those tracks",
                files: vec![(3, true)],
                keys: vec![track_key(0, 0), track_key(0, 1)],
                expect_removed: vec![],
                expect_recordings: 1,
                expect_track_rows: vec![vec![0, 1]],
            },
            Case {
                name: "removed file without db_ref touches no recording",
                files: vec![(1, false)],
                keys: vec![file_key(0)],
                expect_removed: vec![0],
                expect_recordings: 0,
                expect_track_rows: vec![],
            },
            Case {
                name: "removes one file and leaves the other",
                files: vec![(1, true), (2, true)],
                keys: vec![file_key(1)],
                expect_removed: vec![1],
                expect_recordings: 1,
                expect_track_rows: vec![vec![0, 1]],
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
            let rows: Vec<Vec<usize>> = removals.iter().map(|r| r.track_rows.clone()).collect();
            assert_eq!(
                rows, case.expect_track_rows,
                "removed stored track rows for '{}'",
                case.name
            );
        }
    }

    /// The loaded view of a recording whose tracks sit in the stored table rows
    /// `rows`, numbered the way the loader numbers a recording opened from
    /// history.
    fn loaded_recording_in_stored_rows(rows: &[usize], db_ref: &DatabaseRef) -> LoadedFiles {
        let file = LoadedFile {
            metadata: gt_types::FileMetadata {
                filename: "coast-road.gtd".to_owned(),
                ..gt_test_utils::empty_file_metadata()
            },
            tracks: rows
                .iter()
                .map(|row| LoadedTrack {
                    metadata: TrackMetadata {
                        index: row + 1,
                        ..gt_test_utils::empty_track_metadata()
                    },
                    ..gt_test_utils::loaded_track_with_points(Vec::new())
                })
                .collect(),
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdPath(PathBuf::new()),
            load_warnings: Vec::new(),
        };
        let history = FileHistory::recording(
            db_ref.identity.clone(),
            gt_store::RecordingMeta {
                time_range: gt_store::NavPointTimeRange::covering(&[0]),
                nav_point_count: 0,
                sat_report_count: 0,
                marker_count: 0,
                event_marker_count: 0,
                gtd_size_bytes: 0,
            },
            Some(db_ref.clone()),
        );
        let mut loaded = LoadedFiles::new();
        loaded.push(file, history);
        loaded
    }

    /// Remove the track at view position `ti` from the view, and send the
    /// permanent delete for the stored rows that [`remove_items_from_view`]
    /// returns, the way `App::apply_shelve_outcome` sends it.
    fn remove_the_track_permanently(
        worker: &HistoryWorker,
        loaded: &mut LoadedFiles,
        tree: &mut TreeState,
        db_ref: &DatabaseRef,
        ti: usize,
    ) {
        let removals = remove_items_from_view(&[track_key(0, ti)], loaded, tree);
        let [removal] = removals.as_slice() else {
            panic!("expected one affected recording, got {}", removals.len());
        };
        worker.delete_tracks(db_ref.clone(), removal.track_rows.clone());
        let Response::Mutated {
            op: DbOp::TracksDeleted { .. },
            result,
        } = next_response(worker)
        else {
            panic!("expected a TracksDeleted mutation");
        };
        result.expect("the delete runs");
    }

    /// The recording holds three tracks of 7, 7 and 6 points. The session
    /// deletes the first one permanently, then the first of the two that are
    /// left, which leaves the last one and its six points.
    ///
    /// The second delete is sent under the reference that the database lists
    /// after the first delete. Only the stored row identifies the track that
    /// the delete removes.
    #[test]
    fn removing_a_track_after_a_permanent_delete_deletes_the_track_the_user_chose() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_recording_cut_at(&path, &[7, 14]);
        let worker = worker_on(&path);

        let db_ref = only_recording(&worker).db_ref;
        let mut loaded = loaded_recording_in_stored_rows(&[0, 1, 2], &db_ref);
        let mut tree = TreeState::default();
        remove_the_track_permanently(&worker, &mut loaded, &mut tree, &db_ref, 0);

        let after_the_first_delete = only_recording(&worker);
        assert_eq!(
            after_the_first_delete.meta.nav_point_count, 13,
            "the first delete removed seven points"
        );
        let db_ref = after_the_first_delete.db_ref;
        remove_the_track_permanently(&worker, &mut loaded, &mut tree, &db_ref, 0);

        assert_eq!(
            only_recording(&worker).meta.nav_point_count,
            6,
            "the recording keeps the six points of its last track"
        );
        worker.shutdown();
    }

    /// Shelving a track leaves the stored track table in place. The session
    /// still addresses that table by the row number that each of its tracks
    /// holds.
    #[test]
    fn shelving_a_track_of_a_recording_opened_with_a_shelved_track_shelves_the_track_the_user_chose()
     {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.h5");
        seed_recording_cut_at(&path, &[7, 14]);
        let worker = worker_on(&path);

        let db_ref = only_recording(&worker).db_ref;
        worker.set_tracks_shelved(db_ref.clone(), vec![1], true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the first shelve runs");

        // The open leaves the shelved track out of the view. The two that stay
        // keep the numbers of the rows they came from.
        let mut loaded = loaded_recording_in_stored_rows(&[0, 2], &db_ref);
        let mut tree = TreeState::default();
        let removals = remove_items_from_view(&[track_key(0, 1)], &mut loaded, &mut tree);
        let [removal] = removals.as_slice() else {
            panic!("expected one affected recording, got {}", removals.len());
        };
        worker.set_tracks_shelved(db_ref.clone(), removal.track_rows.clone(), true);
        let Response::Mutated { result, .. } = next_response(&worker) else {
            panic!("expected a mutation response");
        };
        result.expect("the second shelve runs");

        worker.open(db_ref);
        let Response::Opened { result, .. } = next_response(&worker) else {
            panic!("expected an Opened response");
        };
        let states: Vec<TrackState> = result
            .expect("the recording opens")
            .tracks
            .iter()
            .map(|track| track.state)
            .collect();
        assert_eq!(
            states,
            vec![TrackState::Live, TrackState::Shelved, TrackState::Shelved],
            "the track the user shelved is the recording's third"
        );
        worker.shutdown();
    }

    /// Shelving a track and unloading one both take it out of the view through
    /// [`remove_items_from_view`]. The recording then reports the distance, the
    /// recorded time, the time range and the fix losses of its other two
    /// tracks.
    #[test]
    fn a_recording_reports_the_figures_of_the_tracks_that_stay_loaded_after_a_removal() {
        let mut loaded = LoadedFiles::new();
        loaded.push(gt_test_utils::segmented_recording(3), FileHistory::None);
        let mut tree = TreeState::new();
        tree.sync_from_loaded_files(loaded.view());

        remove_items_from_view(&[track_key(0, 0)], &mut loaded, &mut tree);

        let metadata = &loaded.get(0).expect("the recording stays loaded").metadata;
        assert_eq!(
            metadata.total_distance,
            TotalDistance::Measured(Length::new::<kilometer>(5.0))
        );
        assert_eq!(metadata.total_duration, chrono::Duration::minutes(20));
        assert_eq!(
            metadata.time_range,
            Some(TimeRange::new(
                DateTime::UNIX_EPOCH + chrono::Duration::minutes(20),
                DateTime::UNIX_EPOCH + chrono::Duration::minutes(50),
            ))
        );
        assert_eq!(
            metadata.fix_stats.map(|stats| stats.fix_loss_count),
            Some(5)
        );
    }

    #[test]
    fn removing_a_recording_whose_tracks_are_all_shelved_removes_no_stored_track() {
        let mut loaded = make_loaded_files(&[(0, true)]);
        let mut tree = TreeState::default();

        let removals = remove_items_from_view(&[file_key(0)], &mut loaded, &mut tree);

        assert!(
            removals.is_empty(),
            "the remove acts on no stored track, and the shelved ones stay stored"
        );
    }

    /// Two recordings of three tracks each, with the tree synced to them and
    /// three of their tracks hidden by the user.
    fn two_recordings_with_hidden_tracks() -> (LoadedFiles, TreeState) {
        let loaded = make_loaded_files(&[(3, true), (3, true)]);
        let mut tree = TreeState::new();
        tree.sync_from_loaded_files(loaded.view());
        for track in [track_ref(0, 0), track_ref(0, 2), track_ref(1, 1)] {
            tree.hide_track(track);
        }
        (loaded, tree)
    }

    fn track_ref(fi: usize, ti: usize) -> TrackRef {
        TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti))
    }

    /// The check of every track of every recording, in tree order.
    fn track_checks(tree: &TreeState) -> Vec<Vec<CheckState>> {
        tree.files
            .iter()
            .map(|file_node| file_node.tracks.iter().map(|track| track.check).collect())
            .collect()
    }

    /// The user hides the first and third track of one recording and the second
    /// track of another, then removes an item. Each track they hid is still
    /// hidden at the position the removal leaves it at.
    #[rstest::rstest]
    #[case::a_whole_recording(
        vec![file_key(0)],
        vec![vec![CheckState::On, CheckState::Off, CheckState::On]],
    )]
    #[case::a_track_between_two_hidden_ones(
        vec![track_key(0, 1)],
        vec![
            vec![CheckState::Off, CheckState::Off],
            vec![CheckState::On, CheckState::Off, CheckState::On],
        ],
    )]
    #[case::a_hidden_track(
        vec![track_key(0, 0)],
        vec![
            vec![CheckState::On, CheckState::Off],
            vec![CheckState::On, CheckState::Off, CheckState::On],
        ],
    )]
    fn a_hidden_track_stays_hidden_when_another_item_is_removed(
        #[case] removed: Vec<NodeKey>,
        #[case] expected: Vec<Vec<CheckState>>,
    ) {
        let (mut loaded, mut tree) = two_recordings_with_hidden_tracks();

        remove_items_from_view(&removed, &mut loaded, &mut tree);

        assert_eq!(track_checks(&tree), expected);
    }

    /// The user expands the second recording and the last of its tracks, hides
    /// one generated-marker type on that track and expands its group, then
    /// removes the first recording.
    #[test]
    fn a_track_keeps_its_expansion_and_its_marker_toggles_when_another_recording_is_removed() {
        let mut loaded = make_loaded_files(&[(2, true), (2, true)]);
        let mut tree = TreeState::new();
        tree.sync_from_loaded_files(loaded.view());
        let track = track_ref(1, 1);
        tree.expand_file(FileIdx::new(1));
        tree.toggle_expand_track(track);
        tree.toggle_generated_kind_hidden(track, GeneratedMarkerKindTag::Slip);
        tree.toggle_generated_kind_expanded(track, GeneratedMarkerKindTag::Slip);

        remove_items_from_view(&[file_key(0)], &mut loaded, &mut tree);

        let moved = track_ref(0, 1);
        assert!(
            tree.file_node(FileIdx::new(0))
                .is_some_and(|file_node| file_node.expanded),
            "the recording is still expanded"
        );
        assert!(
            tree.track_node(moved)
                .is_some_and(|track_node| track_node.expanded),
            "the track is still expanded"
        );
        assert!(
            !tree.generated_kind_visible(moved, GeneratedMarkerKindTag::Slip),
            "the generated-marker type is still hidden"
        );
        assert!(
            tree.generated_kind_expanded(moved, GeneratedMarkerKindTag::Slip),
            "the generated-marker type's group is still expanded"
        );
    }

    /// The user selects the first and the third track of a recording and
    /// removes the first of them.
    #[test]
    fn the_selection_keeps_the_track_that_stays_loaded_across_a_removal() {
        let mut loaded = make_loaded_files(&[(3, true)]);
        let mut tree = TreeState::new();
        tree.sync_from_loaded_files(loaded.view());
        tree.apply_click(track_key(0, 0), false, false);
        tree.apply_click(track_key(0, 2), true, false);

        remove_items_from_view(&[track_key(0, 0)], &mut loaded, &mut tree);

        assert_eq!(
            tree.selection.iter().copied().collect::<Vec<NodeKey>>(),
            vec![track_key(0, 1)]
        );
    }

    /// The user reveals the last track of the second recording from the
    /// Visible section, then removes the first recording.
    #[test]
    fn the_reveal_request_and_the_selection_anchor_move_with_the_track_across_a_removal() {
        let mut loaded = make_loaded_files(&[(2, true), (2, true)]);
        let mut tree = TreeState::new();
        tree.sync_from_loaded_files(loaded.view());
        tree.reveal(track_key(1, 1));

        remove_items_from_view(&[file_key(0)], &mut loaded, &mut tree);

        assert_eq!(tree.reveal_request, Some(track_key(0, 1)));
        assert_eq!(tree.selection_anchor, Some(track_key(0, 1)));
    }

    /// The shelve confirmation is open over tracks of one recording while the
    /// user removes the first of them from the view.
    #[rstest::rstest]
    #[case::one_of_two_tracks(
        vec![track_key(0, 0), track_key(0, 2)],
        Some(vec![track_key(0, 1)]),
    )]
    #[case::the_only_track(vec![track_key(0, 0)], None)]
    fn an_open_shelve_confirmation_lists_the_tracks_that_stay_loaded(
        #[case] items: Vec<NodeKey>,
        #[case] expected: Option<Vec<NodeKey>>,
    ) {
        let mut loaded = make_loaded_files(&[(3, true)]);
        let mut tree = TreeState::new();
        tree.sync_from_loaded_files(loaded.view());
        tree.shelve_confirm = Some(ShelveConfirmState {
            items,
            delete_permanently: false,
        });

        remove_items_from_view(&[track_key(0, 0)], &mut loaded, &mut tree);

        assert_eq!(
            tree.shelve_confirm.map(|confirm| confirm.items),
            expected,
            "the confirmation closes once every track it lists is gone"
        );
    }

    /// The user asks to unload the last track of a recording, then removes the
    /// first one from the view.
    #[test]
    fn a_pending_unload_lists_the_track_at_the_position_the_removal_leaves_it_at() {
        let mut loaded = make_loaded_files(&[(3, true)]);
        let mut tree = TreeState::new();
        tree.sync_from_loaded_files(loaded.view());
        tree.pending_unload = Some(vec![track_key(0, 2)]);

        remove_items_from_view(&[track_key(0, 0)], &mut loaded, &mut tree);

        assert_eq!(tree.pending_unload, Some(vec![track_key(0, 1)]));
    }

    /// One dialog this module renders, driven with content far larger than any
    /// of the audit viewports.
    #[derive(Debug, Clone, Copy)]
    enum OversizedDialog {
        Shelve,
        OrphanedEventMarkers,
        LoadWarnings,
        RecordingDetails,
        About,
        SnapConsent,
        SnapReplace,
        SnapScope,
        SnapAutoPrompt,
        MapboxToken,
        EnvironmentPrune,
        ForceQuit,
    }

    impl OversizedDialog {
        fn title(self) -> String {
            match self {
                Self::Shelve => format!("Shelve {OVERSIZED_ROW_COUNT} items?"),
                Self::OrphanedEventMarkers => {
                    format!("{OVERSIZED_ROW_COUNT} event markers outside track range")
                }
                Self::LoadWarnings => "Data quality warnings".to_owned(),
                Self::RecordingDetails => "Recording details".to_owned(),
                Self::About => "About GeoTrace".to_owned(),
                Self::SnapConsent => "Snap to road".to_owned(),
                Self::SnapReplace => "Snap to road again?".to_owned(),
                Self::SnapScope => {
                    format!("Snap to road as {}", gt_test_utils::oversized_text('m'))
                }
                Self::SnapAutoPrompt => "Snap to road automatically?".to_owned(),
                Self::MapboxToken => "Mapbox API Token Required".to_owned(),
                Self::EnvironmentPrune => DELETE_ARCHIVED_DAYS_TITLE.to_owned(),
                Self::ForceQuit => "Force quit?".to_owned(),
            }
        }

        /// The action the user must still be able to reach. The two dialogs a
        /// read-only viewer closes from its title bar have none of their own.
        fn reachable_button(self) -> Option<&'static str> {
            match self {
                Self::OrphanedEventMarkers | Self::LoadWarnings => Some("Dismiss"),
                Self::RecordingDetails | Self::About => None,
                Self::SnapAutoPrompt => Some("Manual only"),
                Self::MapboxToken => Some("Cancel - use OpenStreetMap"),
                Self::Shelve
                | Self::SnapConsent
                | Self::SnapReplace
                | Self::SnapScope
                | Self::EnvironmentPrune
                | Self::ForceQuit => Some("Cancel"),
            }
        }
    }

    /// Everything the audited dialogs render from, all oversized.
    struct OversizedDialogState {
        dialog: OversizedDialog,
        tree: TreeState,
        loaded_files: LoadedFiles,
        map: NavMap,
        token_field: MapboxTokenField,
        orphaned_event_markers: Option<Vec<(chrono::DateTime<chrono::Utc>, String)>>,
        load_warnings: Option<(String, Vec<gt_types::LoadWarning>)>,
        recording_details: Option<gt_side_panel::RecordingDetails>,
        about_open: bool,
        interruption_costs: Vec<String>,
        loaded_recordings: Vec<String>,
    }

    impl OversizedDialogState {
        fn new(dialog: OversizedDialog) -> Self {
            let long = gt_test_utils::oversized_text('m');
            let mut tree = TreeState::default();
            let loaded_files = make_loaded_files(&[(OVERSIZED_ROW_COUNT, true)]);
            tree.shelve_confirm = Some(ShelveConfirmState {
                items: (0..OVERSIZED_ROW_COUNT)
                    .map(|ti| track_key(0, ti))
                    .collect(),
                delete_permanently: false,
            });
            let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
            map.set_layer(MapLayer::Satellite);
            let mut metadata = gt_test_utils::empty_file_metadata();
            metadata.filename = long.clone();
            metadata.notes = Some(long.clone());
            Self {
                dialog,
                tree,
                loaded_files,
                map,
                token_field: MapboxTokenField::default(),
                orphaned_event_markers: Some(
                    (0..OVERSIZED_ROW_COUNT)
                        .map(|index| (chrono::Utc::now(), format!("{long}/{index}")))
                        .collect(),
                ),
                load_warnings: Some((
                    long.clone(),
                    (0..OVERSIZED_ROW_COUNT)
                        .map(|count| gt_types::LoadWarning {
                            count: count as u32,
                            issue: long.clone(),
                            description: long.clone(),
                        })
                        .collect(),
                )),
                recording_details: Some(gt_side_panel::RecordingDetails {
                    metadata,
                    identity: Some(long.clone()),
                }),
                about_open: true,
                interruption_costs: (0..OVERSIZED_ROW_COUNT)
                    .map(|index| format!("{long}/{index}"))
                    .collect(),
                loaded_recordings: (0..OVERSIZED_ROW_COUNT)
                    .map(|index| format!("{long}/{index}"))
                    .collect(),
            }
        }
    }

    fn oversized_dialog_ui(ui: &mut egui::Ui, state: &mut OversizedDialogState) {
        let long = gt_test_utils::oversized_text('m');
        match state.dialog {
            OversizedDialog::Shelve => {
                let names = RecordingNames::resolve(state.loaded_files.view(), "{filename}");
                show_shelve_confirmation(
                    ui,
                    &mut state.tree,
                    &mut state.loaded_files,
                    &names,
                    &LoadedLogs::default(),
                );
            }
            OversizedDialog::OrphanedEventMarkers => {
                show_orphaned_event_markers_popup(ui, &mut state.orphaned_event_markers);
            }
            OversizedDialog::LoadWarnings => {
                show_load_warnings_dialog(ui, &mut state.load_warnings);
            }
            OversizedDialog::RecordingDetails => {
                show_recording_details_dialog(ui, &mut state.recording_details);
            }
            OversizedDialog::About => show_about_dialog(ui, &mut state.about_open, &long),
            OversizedDialog::SnapConsent => {
                show_snap_consent_dialog(ui, &long, true);
            }
            OversizedDialog::SnapReplace => {
                show_snap_replace_dialog(ui, &long);
            }
            OversizedDialog::SnapScope => {
                show_snap_scope_dialog(ui, &long, SnapScopeCounts::default());
            }
            OversizedDialog::SnapAutoPrompt => {
                show_snap_auto_prompt(ui, &long);
            }
            OversizedDialog::MapboxToken => {
                show_mapbox_token_dialog(ui, &mut state.map, &mut state.token_field);
            }
            OversizedDialog::EnvironmentPrune => {
                show_environment_prune_confirmation(
                    ui,
                    &EnvironmentPrunePrompt {
                        request: prune_request(PruneScope::Every),
                        covered: covered_days(),
                        loaded_recordings: &state.loaded_recordings,
                    },
                );
            }
            OversizedDialog::ForceQuit => {
                show_force_quit_confirmation(
                    ui,
                    &ForceQuitPromptContents::InterruptionCosts(state.interruption_costs.clone()),
                );
            }
        }
    }

    /// Every dialog this module renders stays inside the screen and keeps its
    /// action reachable, however much its caller hands it.
    #[rstest::rstest]
    fn every_dialog_fits_the_audit_viewports(
        #[values(
            OversizedDialog::Shelve,
            OversizedDialog::OrphanedEventMarkers,
            OversizedDialog::LoadWarnings,
            OversizedDialog::RecordingDetails,
            OversizedDialog::About,
            OversizedDialog::SnapConsent,
            OversizedDialog::SnapReplace,
            OversizedDialog::SnapScope,
            OversizedDialog::SnapAutoPrompt,
            OversizedDialog::MapboxToken,
            OversizedDialog::EnvironmentPrune,
            OversizedDialog::ForceQuit
        )]
        dialog: OversizedDialog,
        #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
    ) {
        let mut harness = TestHarness::builder()
            .size(viewport)
            .ui_state(oversized_dialog_ui, OversizedDialogState::new(dialog));
        harness.inner.run_steps(8);

        let title = dialog.title();
        harness
            .inner
            .assert_window_fits_the_viewport(AuditedWindow::titled(&title));
        if let Some(button) = dialog.reachable_button() {
            harness
                .inner
                .assert_control_is_reachable(AuditedWindow::titled(&title), ControlLabel(button));
        }
    }
}

/// Where a dialog's controls are while its content changes size, and what a
/// press aimed at one of them reaches.
///
/// These tests pin the case where the pointer rests on a control for as long
/// as the user takes to decide and the dialog's content arrives in the
/// meantime. [`crate::app::anchored_dialog`] states what egui does with such a
/// press.
#[cfg(test)]
mod anchored_dialog_layout_tests {
    use std::cell::RefCell;

    use chrono::Duration;
    use egui_kittest::Harness;
    use gt_pending_writes::WriteKind;
    use gt_test_utils::window_fit::NARROW_VIEWPORT;
    use gt_test_utils::{By, HarnessInteraction as _, Queryable as _};

    use crate::app::log_viewer::association_dialog::{
        self, CONFIRM_LABEL, LogAssociationChoice, TITLE as ASSOCIATION_TITLE, tests::DialogState,
    };

    use super::{
        DELETE_ARCHIVED_DAYS_TITLE, EnvironmentPruneChoice, ForceQuitChoice,
        LOADED_RECORDINGS_MOST_LINES, PERMANENT_DELETE_LABEL, PruneScope, SnapScopeChoice, tests,
    };

    const CANCEL_LABEL: &str = "Cancel";

    /// Items the tickbox measurement shelves, all of them tracks of one
    /// stored recording.
    const SHELVED_ITEMS: usize = 2;

    /// The ending the two wordings of the shelve confirmation's history
    /// sentence share.
    const DETAIL_SENTENCE_ENDING: &str = "takes them out of the view.";

    /// Fixes of each recording the association dialog lists.
    const FIX_COUNT: usize = 10;

    /// The attachment the history database reports for the chosen recording,
    /// with a name long enough that the note about it wraps onto three lines.
    const STORED_ATTACHMENT_NAME: &str = "navsyncd-export-2026-05-29-evening-run.log";

    /// The recording name added to the prune confirmation once the pending
    /// load finishes.
    const LOADED_RECORDING: &str = "Evening ferry crossing";

    /// Loaded recordings enough to fill the room the prune confirmation caps
    /// at [`LOADED_RECORDINGS_MOST_LINES`].
    const RECORDINGS_PAST_THE_CAPPED_ROOM: usize = 12;

    /// Loaded recordings far past that cap, to read the capped room's height
    /// against.
    const RECORDINGS_FAR_PAST_THE_CAPPED_ROOM: usize = 40;

    /// Writes still running once two of the four have finished.
    const WRITES_STILL_RUNNING: usize = 2;

    /// The costs the force-quit confirmation lists while four writes run.
    fn four_write_costs() -> Vec<String> {
        vec![
            WriteKind::Settings.interruption_cost(),
            WriteKind::RecordingDatabase.interruption_cost(),
            WriteKind::DatabaseOpen.interruption_cost(),
            WriteKind::TakeOverRecord.interruption_cost(),
        ]
    }

    fn loaded_recordings(count: usize) -> RefCell<Vec<String>> {
        RefCell::new(
            (0..count)
                .map(|index| format!("Recording {index}"))
                .collect(),
        )
    }

    /// A list longer than the capped room scrolls inside it, and the buttons
    /// stay where they are when one more name arrives.
    #[test]
    fn the_prune_confirmation_keeps_its_buttons_in_place_while_a_name_arrives_past_the_capped_room()
    {
        let listed = loaded_recordings(RECORDINGS_PAST_THE_CAPPED_ROOM);
        let choice = RefCell::new(None);
        let mut harness = tests::prune_dialog(PruneScope::Every, &listed, &choice);
        let before = harness.inner.get(By::new().label(CANCEL_LABEL)).rect();

        listed.borrow_mut().push(LOADED_RECORDING.to_owned());
        harness.inner.run_steps(4);

        assert_eq!(
            harness.inner.get(By::new().label(CANCEL_LABEL)).rect(),
            before,
            "the Cancel button of the prune confirmation moved: a press where the user aimed \
             misses it"
        );
    }

    /// The capped room holds the prune confirmation to one height, however
    /// many recordings are loaded when it opens.
    #[test]
    fn the_prune_confirmation_opens_at_one_height_for_every_list_past_the_capped_room() {
        let past = loaded_recordings(RECORDINGS_PAST_THE_CAPPED_ROOM);
        let past_choice = RefCell::new(None);
        let past_harness = tests::prune_dialog(PruneScope::Every, &past, &past_choice);
        let far_past = loaded_recordings(RECORDINGS_FAR_PAST_THE_CAPPED_ROOM);
        let far_past_choice = RefCell::new(None);
        let far_past_harness = tests::prune_dialog(PruneScope::Every, &far_past, &far_past_choice);

        assert_eq!(
            far_past_harness
                .inner
                .window_rect(DELETE_ARCHIVED_DAYS_TITLE)
                .expect("the prune confirmation is shown")
                .size(),
            past_harness
                .inner
                .window_rect(DELETE_ARCHIVED_DAYS_TITLE)
                .expect("the prune confirmation is shown")
                .size(),
            "{RECORDINGS_FAR_PAST_THE_CAPPED_ROOM} loaded recordings made the prune confirmation \
             taller than {RECORDINGS_PAST_THE_CAPPED_ROOM} did: a list past the room it caps at \
             {LOADED_RECORDINGS_MOST_LINES} lines has to scroll inside that room"
        );
    }

    /// The prune confirmation lists the loaded recordings whose days the
    /// delete takes. A recording that finishes loading while the dialog is up
    /// joins that list.
    #[test]
    fn the_prune_confirmation_keeps_its_buttons_in_place_while_a_recording_name_arrives() {
        let loaded_recordings = RefCell::new(Vec::new());
        let choice = RefCell::new(None);
        let mut harness = tests::prune_dialog(PruneScope::Every, &loaded_recordings, &choice);
        let before = harness.inner.get(By::new().label(CANCEL_LABEL)).rect();

        loaded_recordings
            .borrow_mut()
            .push(LOADED_RECORDING.to_owned());
        harness.inner.run_steps(4);

        assert_eq!(
            harness.inner.get(By::new().label(CANCEL_LABEL)).rect(),
            before,
            "the Cancel button of the prune confirmation moved: a press where the user aimed \
             misses it"
        );
    }

    /// The user aims at Cancel and a recording finishes loading before the
    /// press. Cancel deletes nothing.
    #[test]
    fn cancelling_the_prune_confirmation_reports_the_cancel_while_a_recording_name_arrives() {
        let loaded_recordings = RefCell::new(Vec::new());
        let choice = RefCell::new(None);
        let mut harness = tests::prune_dialog(PruneScope::Every, &loaded_recordings, &choice);
        let aimed_at = harness
            .inner
            .get(By::new().label(CANCEL_LABEL))
            .rect()
            .center();
        harness.inner.hover_at(aimed_at);
        harness.inner.run_steps(2);

        loaded_recordings
            .borrow_mut()
            .push(LOADED_RECORDING.to_owned());
        harness.inner.run_steps(2);
        harness.inner.press_where_the_pointer_rests(aimed_at);

        assert!(
            matches!(*choice.borrow(), Some(EnvironmentPruneChoice::Cancel)),
            "the press on Cancel deleted the archived days instead of reporting the cancel"
        );
    }

    /// The force-quit confirmation lists one line per write still running, and
    /// the user aims at Cancel while two of the four finish. Cancel keeps the
    /// writes running.
    #[test]
    fn cancelling_the_force_quit_confirmation_reports_the_cancel_while_two_writes_finish() {
        let costs = RefCell::new(four_write_costs());
        let choice = RefCell::new(None);
        let mut harness = tests::force_quit_dialog_over(&costs, &choice, |_| {});
        let aimed_at = harness
            .inner
            .get(By::new().label(CANCEL_LABEL))
            .rect()
            .center();
        harness.inner.hover_at(aimed_at);
        harness.inner.run_steps(2);

        costs.borrow_mut().truncate(WRITES_STILL_RUNNING);
        harness.inner.run_steps(2);
        harness.inner.press_where_the_pointer_rests(aimed_at);

        assert!(
            matches!(*choice.borrow(), Some(ForceQuitChoice::Dismiss)),
            "the press on Cancel did not report the cancel"
        );
    }

    /// The same two writes finish, and the press aimed at the confirmation
    /// does not reach the shutdown window behind it. That window's own row
    /// holds "Force quit…" beside "Run in background".
    #[test]
    fn a_press_aimed_at_the_force_quit_confirmation_does_not_reach_the_window_behind_it() {
        let costs = RefCell::new(four_write_costs());
        let choice = RefCell::new(None);
        let background_pressed = RefCell::new(false);
        let mut harness = tests::force_quit_dialog_over(&costs, &choice, |ui| {
            if ui
                .allocate_response(ui.available_size(), egui::Sense::click())
                .clicked()
            {
                *background_pressed.borrow_mut() = true;
            }
        });
        let aimed_at = harness
            .inner
            .get(By::new().label(CANCEL_LABEL))
            .rect()
            .center();
        harness.inner.hover_at(aimed_at);
        harness.inner.run_steps(2);

        costs.borrow_mut().truncate(WRITES_STILL_RUNNING);
        harness.inner.run_steps(2);
        harness.inner.press_where_the_pointer_rests(aimed_at);

        assert!(
            !*background_pressed.borrow(),
            "the press aimed at Cancel reached the window under the confirmation"
        );
    }

    /// The snap scope dialog counts the tracks that already have snap data,
    /// and states that snapping again replaces it as soon as one does. Cancel
    /// uploads nothing.
    #[test]
    fn cancelling_the_snap_scope_dialog_reports_the_cancel_while_a_snap_result_arrives() {
        let counts = RefCell::new(tests::nothing_snapped_yet());
        let choice = RefCell::new(None);
        let mut harness = tests::snap_scope_dialog(&counts, &choice);
        let aimed_at = harness
            .inner
            .get(By::new().label(CANCEL_LABEL))
            .rect()
            .center();
        harness.inner.hover_at(aimed_at);
        harness.inner.run_steps(2);

        counts.borrow_mut().all.already_snapped = 1;
        harness.inner.run_steps(2);
        harness.inner.press_where_the_pointer_rests(aimed_at);

        assert!(
            matches!(*choice.borrow(), Some(SnapScopeChoice::Cancel)),
            "the press on Cancel uploaded the tracks instead of reporting the cancel"
        );
    }

    /// The shelve confirmation states in one sentence what it does in history,
    /// and the permanent-delete tickbox chooses the wording. The dialog is 324
    /// points wide at [`NARROW_VIEWPORT`]. "Shelves 2 tracks in 1 recording in
    /// history and takes them out of the view." takes one line at that width,
    /// and "Permanently deletes 2 tracks from 1 recording in history and takes
    /// them out of the view." takes two.
    ///
    /// The second line goes into the room the body already had. The window
    /// keeps the height and the position it opened at, which it holds under
    /// its [`AnchoredDialogKind`].
    #[test]
    fn the_shelve_confirmation_keeps_its_window_and_tickbox_in_place_while_the_delete_is_ticked() {
        let mut harness = tests::shelve_confirmation_at(
            NARROW_VIEWPORT,
            SHELVED_ITEMS,
            tests::PermanentDeleteTicked(false),
        );
        let window = tests::shelve_confirmation_rect(&harness, SHELVED_ITEMS);
        let tickbox = harness
            .inner
            .get(By::new().label_contains(PERMANENT_DELETE_LABEL))
            .rect();
        let one_line = harness
            .inner
            .get(By::new().label_contains(DETAIL_SENTENCE_ENDING))
            .rect()
            .height();

        harness.inner.click_at(tickbox.center());
        harness.inner.run_steps(4);

        let two_lines = harness
            .inner
            .get(By::new().label_contains(DETAIL_SENTENCE_ENDING))
            .rect()
            .height();
        assert!(
            two_lines > one_line,
            "the sentence took {two_lines} points ticked and {one_line} points unticked: this \
             measurement needs a width at which the two wordings wrap onto a different number of \
             lines"
        );
        assert_eq!(
            tests::shelve_confirmation_rect(&harness, SHELVED_ITEMS),
            window,
            "the shelve confirmation moved or resized around the longer sentence under its \
             tickbox: its edge moves past a control the user aimed at, and the press reaches \
             the app behind it"
        );
        assert_eq!(
            harness
                .inner
                .get(By::new().label_contains(PERMANENT_DELETE_LABEL))
                .rect(),
            tickbox,
            "the permanent-delete tickbox moved under the pointer that just ticked it: the press \
             that unticks it misses"
        );
    }

    /// The press the association tests use, over a confirmation whose content
    /// does not change: it reports Cancel and reaches nothing behind the
    /// confirmation.
    #[test]
    fn a_press_where_the_pointer_rests_cancels_an_unchanged_force_quit_confirmation() {
        let costs = RefCell::new(four_write_costs());
        let choice = RefCell::new(None);
        let background_pressed = RefCell::new(false);
        let mut harness = tests::force_quit_dialog_over(&costs, &choice, |ui| {
            if ui
                .allocate_response(ui.available_size(), egui::Sense::click())
                .clicked()
            {
                *background_pressed.borrow_mut() = true;
            }
        });
        let aimed_at = harness
            .inner
            .get(By::new().label(CANCEL_LABEL))
            .rect()
            .center();
        harness.inner.hover_at(aimed_at);
        harness.inner.run_steps(4);

        harness.inner.press_where_the_pointer_rests(aimed_at);

        assert!(matches!(*choice.borrow(), Some(ForceQuitChoice::Dismiss)));
        assert!(!*background_pressed.borrow());
    }

    /// The association dialog over two stored recordings: the second only
    /// overlaps part of the log and is listed below the first.
    fn association_dialog() -> Harness<'static, DialogState> {
        association_dialog::tests::harness_over_sized(
            vec![
                (
                    association_dialog::tests::recording(
                        "alongside.gtd",
                        Duration::zero(),
                        FIX_COUNT,
                    ),
                    association_dialog::tests::stored_in_history("nav-devkit-mk2"),
                ),
                (
                    association_dialog::tests::recording(
                        "late.gtd",
                        Duration::seconds(5),
                        FIX_COUNT,
                    ),
                    association_dialog::tests::stored_in_history("nav-devkit-mk4"),
                ),
            ],
            tests::DIALOG_VIEWPORT,
        )
    }

    /// Selecting a recording sends the duplicate-attachment query. The dialog
    /// draws a line above the checkbox that stores the log when the result
    /// arrives. A press aimed at a recording row must not tick that box:
    /// attaching writes the log into the history database.
    #[test]
    fn pressing_a_recording_row_while_the_stored_attachment_line_arrives_attaches_nothing() {
        let mut harness = association_dialog();
        harness.get(By::new().label("late.gtd")).click();
        harness.run_steps(3);
        let aimed_at = harness.get(By::new().label("late.gtd")).rect().center();
        harness.hover_at(aimed_at);
        harness.run_steps(2);

        association_dialog::tests::deliver_the_stored_attachment(
            &mut harness,
            STORED_ATTACHMENT_NAME,
        );
        harness.run_steps(2);
        harness.press_where_the_pointer_rests(aimed_at);
        harness.get(By::new().label(CONFIRM_LABEL)).click();
        harness.run_steps(2);

        assert!(
            matches!(
                harness.state().choice,
                Some(LogAssociationChoice::Confirmed { attach: false, .. })
            ),
            "the press on a recording row reported {:?}",
            harness.state().choice,
        );
    }

    #[test]
    fn the_association_dialog_keeps_its_recording_rows_in_place_while_the_result_arrives() {
        let mut harness = association_dialog();
        harness.get(By::new().label("late.gtd")).click();
        harness.run_steps(3);
        let before = harness.get(By::new().label("late.gtd")).rect();

        association_dialog::tests::deliver_the_stored_attachment(
            &mut harness,
            STORED_ATTACHMENT_NAME,
        );
        harness.run_steps(4);
        let after = harness.get(By::new().label("late.gtd")).rect();

        assert_eq!(
            after, before,
            "the row of the chosen recording moved in the {ASSOCIATION_TITLE} dialog: a press \
             where the user aimed misses it"
        );
    }
}
