use std::collections::HashMap;

use egui::{Button, Label, RichText, ScrollArea, Sides, TextEdit};
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PATH as ICON_PATH;
use egui_phosphor::regular::ROAD_HORIZON as ICON_ROAD_HORIZON;
use egui_phosphor::regular::TRASH as ICON_TRASH;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use gt_filter::GlobalFilter;
use gt_fmt::{NameFields, render_name_template};
use gt_loaded_files::LoadedFilesView;
use gt_types::{
    DataCategory, FileIdx, FileMetadata, GeneratedMarkerKind, LoadWarning, LoadedFile, LoadedTrack,
    PointIdx, TrackIdx, TrackRef,
};
use gt_ui_theme::ELLIPSIS;
use gt_ui_types::{DataPointRef, DisplayCategory, DisplayMask, HighlightScope, MapHighlight};

use crate::filter::{FilterPanelState, render_filter_panel};
use crate::tree::{CheckState, DeleteConfirmState, NodeKey, TreeState};
use crate::widgets::{
    MetadataView, checkbox_width, expand_arrow, fix_stats_tooltip_row, has_metadata_details,
    paint_map_hover_bg, point_item_row, tri_checkbox,
};

/// A recording's metadata, captured when its note icon is clicked so the app can
/// open the details dialog. Owns its data so it outlives the source file.
#[derive(Debug, Clone)]
pub struct RecordingDetails {
    pub metadata: gt_types::FileMetadata,
    /// The raw recording identity, if any; stripped for display by the dialog.
    pub identity: Option<String>,
}

/// Snap-to-road state for the whole panel, resolved by the app each frame.
///
/// A panel-local view (like [`RecordingDetails`] for gt-history) so the panel
/// needs no dependency on the snap machinery: the app maps its scheduler and
/// settings into plain data, the panel only renders it.
#[derive(Clone, Copy)]
pub struct SnapPanelView<'a> {
    /// `GEOTRACE_OFFLINE` is set: every snap trigger is grayed out.
    pub offline: bool,
    /// Upload consent has not been acknowledged for the configured server, so
    /// the trigger carries the `…` suffix - a click opens the consent dialog.
    pub consent_pending: bool,
    /// Per-track snap state. Tracks without an entry are [`SnapRowView::Idle`].
    pub rows: &'a HashMap<TrackRef, SnapRowView>,
}

/// One track's snap state as the panel shows it, mirroring the app's
/// scheduler states plus the settings-derived unsnappable case.
#[derive(Debug, Clone, PartialEq)]
pub enum SnapRowView {
    /// Snappable, no run this session. The default for tracks without an entry.
    Idle,
    /// The file declares a travel mode without a road network (boat, rail,
    /// aircraft); the value is the mode's display name for the hover text.
    Unsnappable {
        travel_mode: String,
    },
    Queued,
    InFlight {
        completed_chunks: usize,
        total_chunks: usize,
    },
    Failed {
        error: String,
    },
    Done {
        snapped: usize,
        interpolated: usize,
        unsnapped: usize,
        /// Run confidence in `0..=1`, when the server reported one.
        confidence_score: Option<f64>,
        /// Whether the snapped track is currently drawn on the map.
        shown: bool,
        /// `Some` when the run is stale - produced under parameters or a
        /// server that differ from the current settings. The lines name
        /// each difference; the row offers a re-run. `None` = current.
        stale: Option<String>,
    },
}

pub struct PanelContext<'a> {
    pub loaded_files: LoadedFilesView<'a>,
    pub tree: &'a mut TreeState,
    pub highlight: &'a mut MapHighlight,
    pub filter: &'a mut GlobalFilter,
    pub filter_state: &'a mut FilterPanelState,
    pub map_center_request: &'a mut Option<(f64, f64)>,
    pub popup_pos_request: &'a mut Option<egui::Pos2>,
    pub zoom_to_visible_request: &'a mut bool,
    /// Set by clicking the ⚠ icon on a file row. Consumed by the app to show a centered dialog.
    pub warnings_request: &'a mut Option<(String, Vec<LoadWarning>)>,
    /// Set when "Reset filters" is clicked, so the app can also drop the query
    /// filter (which the side panel cannot reach directly).
    pub clear_query_request: &'a mut bool,
    /// The map's display mask, read to hint on category rows whose ink the
    /// display toggles currently hide.
    pub display_mask: DisplayMask,
    /// User template for the recording name shown on each file row. See
    /// [`gt_fmt::render_name_template`].
    pub recording_name_template: &'a str,
    /// Set by clicking a file row's note icon. Consumed by the app to open the
    /// recording-details dialog.
    pub metadata_request: &'a mut Option<RecordingDetails>,
    /// Snap-to-road state per track, resolved by the app.
    pub snap: SnapPanelView<'a>,
    /// Set by clicking a track's snap trigger. Consumed by the app, which
    /// routes it through the consent dialog when consent is pending and
    /// queues the run otherwise.
    pub snap_request: &'a mut Option<TrackRef>,
    /// Set by the snapped-track visibility toggle (the status glyph or the
    /// context-menu entry) of a completed run. Consumed by the app, which
    /// flips whether that track's snapped track draws on the map.
    pub snap_visibility_request: &'a mut Option<TrackRef>,
}

/// Trailing eye-slash on a category row whose map ink is hidden by the
/// display toggles - the tree and the mask explain each other instead of
/// silently compounding.
fn masked_hint(ui: &mut egui::Ui, mask: DisplayMask, category: DataCategory) {
    masked_display_hint(ui, mask, DisplayCategory::from(category));
}

/// [`masked_hint`] for ink without a tree data category (the snapped track).
fn masked_display_hint(ui: &mut egui::Ui, mask: DisplayMask, category: DisplayCategory) {
    if !mask.is_visible(category) {
        ui.label(RichText::new(ICON_EYE_SLASH).weak())
            .on_hover_text("Hidden by the map display toggles");
    }
}

impl<'a> PanelContext<'a> {
    fn files(&self) -> &'a [LoadedFile] {
        self.loaded_files.files()
    }

    fn file(&self, file: FileIdx) -> Option<&'a LoadedFile> {
        self.loaded_files.entry_for(file).map(|entry| entry.file())
    }

    fn file_stored_in_history(&self, file: FileIdx) -> bool {
        self.loaded_files.file_stored_in_history(file)
    }

    /// The recording identity for a file, or `None` if it has none. Used by the
    /// `{identity}` display-name token.
    fn identity(&self, file: FileIdx) -> Option<&'a str> {
        self.loaded_files.entry_for(file).and_then(|e| e.identity())
    }
}

pub fn show_side_panel(ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
    let header = ui.horizontal(|ui| {
        let (_, grip) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), egui::Sense::drag());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ctx.tree.detached {
                if ui.small_button("Dock").clicked() {
                    ctx.tree.detached = false;
                }
            } else if ui
                .small_button(ICON_ARROW_SQUARE_OUT)
                .on_hover_text("Pop out")
                .clicked()
            {
                ctx.tree.detached = true;
            }
        });
        grip
    });
    if header.inner.dragged()
        && ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|p| !ui.clip_rect().contains(p))
    {
        ctx.tree.detached = true;
    }

    ui.separator();
    if render_filter_panel(ui, ctx.files(), ctx.filter, ctx.filter_state) {
        *ctx.clear_query_request = true;
    }

    let filter_snapshot = *ctx.filter;
    let vis = ctx.tree.visibility();
    let filtered_out: Vec<NodeKey> = ctx
        .files()
        .iter()
        .enumerate()
        .flat_map(|(fi, file)| {
            let fi = FileIdx::new(fi);
            let file_enabled = fi.get(&vis.files).is_some_and(|fv| fv.enabled);
            file.tracks
                .iter()
                .enumerate()
                .filter_map(move |(ti, track)| {
                    let ti = TrackIdx::new(ti);
                    let track_enabled = file_enabled
                        && fi
                            .get(&vis.files)
                            .and_then(|fv| ti.get(&fv.tracks))
                            .is_some_and(|tv| tv.enabled);
                    let passes = gt_filter::track_passes_filter(&track.metadata, &filter_snapshot);
                    if !track_enabled || !passes {
                        Some(NodeKey::Track(TrackRef::new(fi, ti)))
                    } else {
                        None
                    }
                })
        })
        .collect();
    let has_filtered = !filtered_out.is_empty();
    let clicked = ui
        .scope(|ui| {
            if has_filtered {
                let v = ui.visuals_mut();
                v.widgets.hovered.bg_fill = gt_ui_theme::DANGER_HOVER;
                v.widgets.hovered.fg_stroke.color = gt_ui_theme::DANGER_FG;
                v.widgets.active.bg_fill = gt_ui_theme::DANGER_ACTIVE;
                v.widgets.active.fg_stroke.color = gt_ui_theme::DANGER_FG;
            }
            ui.add_enabled(
                has_filtered,
                Button::new(format!("{ICON_TRASH} Remove filtered data")),
            )
            .clicked()
        })
        .inner;
    if clicked {
        ctx.tree.delete_confirm = Some(DeleteConfirmState {
            items: filtered_out,
            delete_permanently: false,
        });
    }

    ui.separator();

    ui.horizontal(|ui| {
        if ui.small_button("Show all").clicked() {
            ctx.tree.set_all_enabled(true);
            *ctx.zoom_to_visible_request = true;
        }
        if ui.small_button("Hide all").clicked() {
            ctx.tree.set_all_enabled(false);
        }
    });

    // Compute per-file display names: strip the longest shared directory prefix
    // so files like `/home/user/recordings/a.gtd` and `/home/user/recordings/b.gtd`
    // display as `a.gtd` and `b.gtd` instead of their full paths.
    let display_names: Vec<String> = {
        let files = ctx.files();
        let all_names: Vec<&str> = files.iter().map(|f| f.metadata.filename.as_str()).collect();
        let prefix_len = strip_common_path_prefix(&all_names);
        files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let name = f.metadata.filename.as_str();
                // `prefix_len` is always a valid char boundary (guaranteed by
                // `strip_common_path_prefix`), but use `get` to satisfy the
                // `clippy::string_slice` lint and handle degenerate inputs safely.
                let stripped = name
                    .get(prefix_len..)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(name);
                recording_display_name(
                    ctx.recording_name_template,
                    &f.metadata,
                    ctx.identity(FileIdx::new(i)),
                    stripped,
                )
            })
            .collect()
    };

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for fi in 0..ctx.files().len() {
                let display_name = display_names.get(fi).map_or("", String::as_str);
                render_file_row(ui, FileIdx::new(fi), display_name, ctx);
            }
        });
}

fn render_file_row(ui: &mut egui::Ui, fi: FileIdx, display_name: &str, ctx: &mut PanelContext<'_>) {
    let Some(file) = ctx.file(fi) else {
        return;
    };
    let Some(file_node) = ctx.tree.file_node(fi) else {
        return;
    };
    let is_expanded = file_node.expanded;
    let check = file_node.check;
    let file_key = NodeKey::File(fi);

    let file_map_hovered = ctx.highlight.hover.is_some_and(|s| match s {
        HighlightScope::Point(r) => r.track.fi == fi,
        HighlightScope::Track(track) | HighlightScope::TrackCategory { track, .. } => {
            track.fi == fi
        }
        HighlightScope::File { file_index } => file_index == fi,
    });

    let map_hover_bg = gt_ui_theme::map_hover_color(ui.visuals().dark_mode);

    let row_response = ui.horizontal(|ui| {
        // Keep the checkbox, note icon and expand arrow visually grouped rather
        // than spread out by the default item spacing.
        ui.spacing_mut().item_spacing.x = 2.0;
        let chk_resp = tri_checkbox(ui, check);
        if chk_resp.clicked() {
            ctx.tree.toggle_file_check(fi);
        }
        // A note icon between the checkbox and the expand arrow opens the details
        // dialog, so metadata is one click away without pushing a block under the
        // row. Only shown when there is something to reveal.
        let identity = ctx.identity(fi);
        if has_metadata_details(&MetadataView::from_file_metadata(&file.metadata, identity)) {
            // A frameless button (not a Label) so the pointer reads as clickable
            // and the icon highlights on hover, instead of showing a text cursor.
            let icon = ui
                .add(Button::new(ICON_NOTE).frame(false))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Recording details");
            if icon.clicked() {
                *ctx.metadata_request = Some(RecordingDetails {
                    metadata: file.metadata.clone(),
                    identity: identity.map(str::to_owned),
                });
            }
        }
        let arrow = expand_arrow(is_expanded);
        let dist = gt_fmt::format_distance(file.metadata.total_distance_km);
        let dur = gt_fmt::format_human_terse_duration(file.metadata.total_duration);
        let is_selected = ctx.tree.selection.contains(&file_key);
        // Truncate only the identity: the distance and duration stay pinned on the
        // right so a long recording name clips itself instead of hiding the metrics
        // or forcing the panel to grow. `Sides::shrink_left` lays the right group
        // out first, then truncates the identity into whatever width is left.
        let (resp, ()) = Sides::new().shrink_left().truncate().show(
            ui,
            |ui| {
                let label = format!("{arrow} {display_name}");
                ui.add(Button::selectable(is_selected, RichText::new(label)).truncate())
            },
            |ui| {
                // Right widgets are laid out right-to-left, so add trailing items first.
                if !file.load_warnings.is_empty() {
                    let icon = RichText::new(ICON_WARNING)
                        .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode));
                    if ui
                        .add(Label::new(icon).sense(egui::Sense::click()))
                        .on_hover_text("Data quality warnings - click for details")
                        .clicked()
                    {
                        *ctx.warnings_request =
                            Some((file.metadata.filename.clone(), file.load_warnings.clone()));
                    }
                }
                ui.label(format!("{ICON_CLOCK} {dur}"))
                    .on_hover_text("Total duration");
                ui.label(format!("{ICON_ROAD_HORIZON} {dist}"))
                    .on_hover_text("Total distance");
            },
        );
        if let Some(stats) = file.metadata.fix_stats {
            resp.on_hover_ui(|ui| {
                ui.label(file.metadata.filename.as_str());
                fix_stats_tooltip_row(ui, stats);
            })
        } else {
            resp.on_hover_text(file.metadata.filename.as_str())
        }
    });

    let file_label_resp = row_response.inner;
    if file_map_hovered {
        paint_map_hover_bg(ui, row_response.response.rect, map_hover_bg);
    }
    if file_label_resp.hovered() {
        ctx.highlight.hover = Some(HighlightScope::File { file_index: fi });
    }
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if file_label_resp.double_clicked() {
        if let Some(center) = file_bounding_center(ctx.file(fi)) {
            *ctx.map_center_request = Some(center);
        }
    } else if file_label_resp.clicked() {
        if modifiers.ctrl || modifiers.shift {
            ctx.tree
                .apply_click(file_key, modifiers.ctrl, modifiers.shift);
        } else {
            ctx.tree.toggle_expand_file(fi);
            ctx.tree.apply_click(file_key, false, false);
        }
    }
    file_label_resp.context_menu(|ui| {
        if ui.button("Show only this file").clicked() {
            ctx.tree.show_only_file(fi);
            ui.close();
        }
        ui.separator();
        let stored_in_history = ctx.file_stored_in_history(fi);
        let unload = ui.button("Unload").on_hover_text(if stored_in_history {
            "Unloads this recording from the view; it stays in History"
        } else {
            "Unloads this file from the current view"
        });
        if unload.clicked() {
            ctx.tree.pending_unload = Some(vec![file_key]);
            ui.close();
        }
        if ctx.tree.selection.len() >= 2 && ui.button("Unload selected").clicked() {
            ctx.tree.pending_unload = Some(ctx.tree.selection.iter().cloned().collect());
            ui.close();
        }
    });

    if is_expanded {
        ui.indent(format!("file_{fi}"), |ui| {
            let track_count = ctx.file(fi).map_or(0, |f| f.tracks.len());
            for ti in 0..track_count {
                render_track_row(ui, fi, TrackIdx::new(ti), ctx);
            }
        });
    }
}

/// What the snap trigger can do for a track right now: whether it is
/// clickable, why not when grayed, and whether a click still needs the
/// consent dialog (the `…` suffix). `None` when a completed run leaves
/// nothing to trigger - the row then shows the status glyph instead.
struct SnapAction {
    enabled: bool,
    hover: String,
    consent_pending: bool,
}

/// The trigger state for a track, shared by the row button and the context
/// menu entry so the two can never disagree.
fn snap_action(row: &SnapRowView, snap: SnapPanelView<'_>) -> Option<SnapAction> {
    let action = match row {
        // A current completed run leaves nothing to trigger; its status
        // glyph stays fully usable offline (cached results are local).
        SnapRowView::Done { stale: None, .. } => return None,
        // Unsnappable beats offline: it is the permanent condition, and its
        // hover names the declared mode.
        SnapRowView::Unsnappable { travel_mode } => SnapAction {
            enabled: false,
            hover: format!(
                "Not snappable - the declared travel mode {travel_mode} has no road network"
            ),
            consent_pending: false,
        },
        _ if snap.offline => SnapAction {
            enabled: false,
            hover: "Snapping is disabled while GEOTRACE_OFFLINE is set".to_owned(),
            consent_pending: false,
        },
        // A stale run keeps its status glyph; this action is the re-run.
        SnapRowView::Done {
            stale: Some(reasons),
            ..
        } => SnapAction {
            enabled: true,
            hover: format!("{reasons}\nClick to snap again with the current settings."),
            consent_pending: snap.consent_pending,
        },
        SnapRowView::Queued => SnapAction {
            enabled: false,
            hover: "Queued for snapping".to_owned(),
            consent_pending: false,
        },
        SnapRowView::InFlight {
            completed_chunks,
            total_chunks,
        } => SnapAction {
            enabled: false,
            hover: format!("Snapping - completed {completed_chunks} of {total_chunks} chunks"),
            consent_pending: false,
        },
        SnapRowView::Failed { error } => SnapAction {
            enabled: true,
            hover: format!("Snap to road failed - {error}. Click to retry."),
            consent_pending: snap.consent_pending,
        },
        SnapRowView::Idle => SnapAction {
            enabled: true,
            hover: "Snap to road - match this track against the OpenStreetMap road network"
                .to_owned(),
            consent_pending: snap.consent_pending,
        },
    };
    Some(action)
}

/// Extra dimming applied to the status glyph while the snapped track is
/// hidden, so the toggle state is readable at a glance.
const HIDDEN_GLYPH_ALPHA: f32 = 0.5;

/// The toggle hint appended to the snap status hover.
fn snapped_track_toggle_label(shown: bool) -> &'static str {
    if shown {
        "Snapped track shown on the map - click to hide"
    } else {
        "Snapped track hidden - click to show"
    }
}

/// The completed-run breakdown shown in the snap status hover: per-kind point
/// counts and the run confidence.
fn snap_status_rows(
    ui: &mut egui::Ui,
    snapped: usize,
    interpolated: usize,
    unsnapped: usize,
    confidence_score: Option<f64>,
) {
    ui.label(RichText::new("Snapped to road").strong());
    egui::Grid::new("snap_status_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            let mut row = |label: &str, value: String| {
                ui.label(RichText::new(label).weak());
                ui.label(value);
                ui.end_row();
            };
            row("Snapped", gt_fmt::format_count(snapped));
            row("Interpolated", gt_fmt::format_count(interpolated));
            row("Unsnapped", gt_fmt::format_count(unsnapped));
            if let Some(score) = confidence_score {
                row("Confidence", gt_fmt::format_fraction_percent(score));
            }
        });
}

/// The trailing per-track snap control: the manual trigger while a run is
/// possible (grayed with hover text when it is not, never hidden), and the
/// run's status glyph with the breakdown hover once complete.
fn snap_control(ui: &mut egui::Ui, track_ref: TrackRef, ctx: &mut PanelContext<'_>) {
    let row = ctx.snap.rows.get(&track_ref).unwrap_or(&SnapRowView::Idle);

    // A completed run always shows its status glyph (fresh or stale); a
    // stale run additionally gets the re-run trigger from `snap_action`.
    if let SnapRowView::Done {
        snapped,
        interpolated,
        unsnapped,
        confidence_score,
        shown,
        stale,
    } = row
    {
        let (snapped, interpolated, unsnapped, confidence_score, shown) = (
            *snapped,
            *interpolated,
            *unsnapped,
            *confidence_score,
            *shown,
        );
        // The status glyph doubles as the per-track visibility toggle: weak
        // while the snapped track draws, extra-faint while hidden. A stale
        // run's glyph turns warning-colored so the outdated result is
        // visible at a glance, never silently current-looking.
        let text = match (stale, shown) {
            (Some(_), _) => {
                RichText::new(ICON_PATH).color(gt_ui_theme::warning_amber(ui.visuals().dark_mode))
            }
            (None, true) => RichText::new(ICON_PATH).weak(),
            (None, false) => RichText::new(ICON_PATH).weak().color(
                ui.visuals()
                    .weak_text_color()
                    .gamma_multiply(HIDDEN_GLYPH_ALPHA),
            ),
        };
        let stale = stale.clone();
        let glyph = ui
            .add(Button::new(text).frame(false))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_ui(|ui| {
                snap_status_rows(ui, snapped, interpolated, unsnapped, confidence_score);
                if let Some(reasons) = &stale {
                    ui.label(
                        RichText::new(format!("Stale - {}", stale_hover_line(reasons)))
                            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                    );
                }
                ui.label(RichText::new(snapped_track_toggle_label(shown)).weak());
            });
        if glyph.clicked() {
            *ctx.snap_visibility_request = Some(track_ref);
        }
        masked_display_hint(ui, ctx.display_mask, DisplayCategory::SnappedTracks);
    }

    let Some(action) = snap_action(row, ctx.snap) else {
        return;
    };
    let failed = matches!(row, SnapRowView::Failed { .. });
    let label = if action.consent_pending {
        format!("{ICON_PATH}{ELLIPSIS}")
    } else {
        ICON_PATH.to_owned()
    };
    let mut text = RichText::new(label);
    if failed {
        text = text.color(gt_ui_theme::warning_amber(ui.visuals().dark_mode));
    }
    let button = ui.add_enabled(action.enabled, Button::new(text).frame(false));
    let button = if action.enabled {
        button
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&action.hover)
    } else {
        button.on_disabled_hover_text(&action.hover)
    };
    if button.clicked() {
        *ctx.snap_request = Some(track_ref);
    }
}

/// The stale reasons flattened into one hover line ("a - b, c - d").
fn stale_hover_line(reasons: &str) -> String {
    reasons.lines().collect::<Vec<_>>().join(", ")
}

/// The context-menu counterpart of [`snap_control`]: same action state, text
/// label instead of the icon. A completed run shows the entry disabled with
/// the status hover, per the never-hide rule.
fn snap_menu_entry(ui: &mut egui::Ui, track_ref: TrackRef, ctx: &mut PanelContext<'_>) {
    let row = ctx.snap.rows.get(&track_ref).unwrap_or(&SnapRowView::Idle);
    match snap_action(row, ctx.snap) {
        Some(action) => {
            // A stale completed run re-runs; everything else is a first run.
            let verb = if matches!(row, SnapRowView::Done { .. }) {
                "Snap again"
            } else {
                "Snap to road"
            };
            let label = if action.consent_pending {
                format!("{verb}{ELLIPSIS}")
            } else {
                verb.to_owned()
            };
            let button = ui.add_enabled(action.enabled, Button::new(label));
            let button = if action.enabled {
                button.on_hover_text(&action.hover)
            } else {
                button.on_disabled_hover_text(&action.hover)
            };
            if button.clicked() {
                *ctx.snap_request = Some(track_ref);
                ui.close();
            }
        }
        None => {
            let SnapRowView::Done {
                snapped,
                interpolated,
                unsnapped,
                confidence_score,
                ..
            } = row
            else {
                return;
            };
            let (snapped, interpolated, unsnapped, confidence_score) =
                (*snapped, *interpolated, *unsnapped, *confidence_score);
            ui.add_enabled(false, Button::new("Snap to road"))
                .on_disabled_hover_ui(|ui| {
                    snap_status_rows(ui, snapped, interpolated, unsnapped, confidence_score);
                });
        }
    }
    // The visibility toggle applies to any completed run, stale or not.
    if let SnapRowView::Done { shown, .. } = row {
        let toggle_label = if *shown {
            "Hide snapped track"
        } else {
            "Show snapped track"
        };
        if ui.button(toggle_label).clicked() {
            *ctx.snap_visibility_request = Some(track_ref);
            ui.close();
        }
    }
}

fn render_track_row(ui: &mut egui::Ui, fi: FileIdx, ti: TrackIdx, ctx: &mut PanelContext<'_>) {
    let track_ref = TrackRef::new(fi, ti);
    let (track, passes, is_expanded, panel_hovered, map_hovered, key) = {
        let Some(file) = ctx.file(fi) else {
            return;
        };
        let Some(track) = ti.get(&file.tracks) else {
            return;
        };
        let passes = gt_filter::track_passes_filter(&track.metadata, ctx.filter);
        let is_expanded = ctx.tree.track_node(track_ref).is_some_and(|t| t.expanded);
        let panel_hovered = ctx
            .highlight
            .hover
            .is_some_and(|s| matches!(s, HighlightScope::Track(t) if t == track_ref));
        let map_hovered = ctx
            .highlight
            .hover
            .is_some_and(|s| matches!(s, HighlightScope::Point(r) if r.track == track_ref));
        let key = NodeKey::Track(track_ref);
        (
            track.clone(),
            passes,
            is_expanded,
            panel_hovered,
            map_hovered,
            key,
        )
    };

    let was_all_hidden = ctx.tree.all_hidden();
    let map_hover_bg = gt_ui_theme::map_hover_color(ui.visuals().dark_mode);

    let check = ctx
        .tree
        .track_node(track_ref)
        .map_or(CheckState::On, |t| t.check);

    let row_response = ui.horizontal(|ui| {
        let chk_resp = tri_checkbox(ui, check);
        if chk_resp.clicked() {
            ctx.tree.toggle_track_check(track_ref);
        }
        let newly_enabled =
            chk_resp.clicked() && matches!(check, CheckState::Off | CheckState::Mixed);
        let arrow = expand_arrow(is_expanded);
        let dist = gt_fmt::format_distance(track.metadata.distance_km);
        let dur = gt_fmt::format_human_terse_duration(track.metadata.duration);
        let label = format!("{arrow} #{}  {dist}  {dur}", track.metadata.index);
        let mut text = RichText::new(label);
        if !passes {
            text = text.weak();
        }
        if panel_hovered {
            text = text.color(gt_ui_theme::HIGHLIGHT_BLUE);
        }
        let is_selected = ctx.tree.selection.contains(&key);
        let resp = ui.selectable_label(is_selected, text);
        let time_header = gt_fmt::format_time_range(
            track.metadata.time_range.start,
            track.metadata.time_range.end,
        );
        let fix_stats = track.metadata.fix_stats;
        let resp = resp.on_hover_ui(|ui| {
            ui.label(RichText::new(&time_header).strong());
            match fix_stats {
                Some(stats) => fix_stats_tooltip_row(ui, stats),
                None => {
                    ui.label("No satellite data");
                }
            }
        });
        snap_control(ui, track_ref, ctx);
        (resp, newly_enabled)
    });

    if map_hovered {
        paint_map_hover_bg(ui, row_response.response.rect, map_hover_bg);
    }
    let (response, newly_enabled) = row_response.inner;
    if newly_enabled && was_all_hidden {
        *ctx.zoom_to_visible_request = true;
    }
    if response.hovered() {
        ctx.highlight.hover = Some(HighlightScope::Track(track_ref));
    }
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if response.double_clicked() {
        let bb = track.metadata.bounding_box;
        let center_lat = (bb.min().y + bb.max().y) / 2.0;
        let center_lon = (bb.min().x + bb.max().x) / 2.0;
        *ctx.map_center_request = Some((center_lat, center_lon));
    } else if response.clicked() {
        if modifiers.ctrl || modifiers.shift {
            ctx.tree.apply_click(key, modifiers.ctrl, modifiers.shift);
        } else {
            ctx.tree.toggle_expand_track(track_ref);
            ctx.tree.apply_click(key, false, false);
        }
    }
    response.context_menu(|ui| {
        if ui.button("Show only this track").clicked() {
            ctx.tree.show_only_track(track_ref);
            *ctx.zoom_to_visible_request = true;
            ui.close();
        }
        snap_menu_entry(ui, track_ref, ctx);
        ui.separator();
        if ui.button("Unload").clicked() {
            ctx.tree.pending_unload = Some(vec![key]);
            ui.close();
        }
        if ctx.tree.selection.len() >= 2 && ui.button("Unload selected").clicked() {
            ctx.tree.pending_unload = Some(ctx.tree.selection.iter().cloned().collect());
            ui.close();
        }
    });

    if is_expanded {
        ui.indent(format!("track_{fi}_{ti}"), |ui| {
            render_track_categories(ui, track_ref, &track, ctx);
        });
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "all arguments are distinct; extracting a context struct avoids re-borrowing tree mid-render"
)]
fn render_category_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    cat: DataCategory,
    count: usize,
    label: &str,
    visible: bool,
    expanded: bool,
    display_mask: DisplayMask,
    tree: &mut TreeState,
    highlight: &mut MapHighlight,
    render_items: impl FnOnce(&mut egui::Ui, &mut MapHighlight),
) {
    if count == 0 {
        return;
    }
    let header = ui.horizontal(|ui| {
        let chk = tri_checkbox(
            ui,
            if visible {
                CheckState::On
            } else {
                CheckState::Off
            },
        );
        if chk.clicked() {
            tree.set_category_visible(track_ref, cat, !visible);
        }
        let arrow = expand_arrow(expanded);
        let resp = ui.selectable_label(expanded, format!("{arrow} {label}  {count}"));
        if resp.clicked() {
            tree.toggle_category_expanded(track_ref, cat);
        }
        masked_hint(ui, display_mask, cat);
        resp
    });
    if header.inner.hovered() {
        highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: cat,
        });
    }
    if expanded {
        ui.indent((cat, track_ref), |ui| {
            render_items(ui, highlight);
        });
    }
}

fn render_track_categories(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    ctx: &mut PanelContext<'_>,
) {
    let Some(track_node) = ctx.tree.track_node(track_ref) else {
        return;
    };
    let track_visible = track_node.track_visible;
    let tpv_visible = track_node.tpv_visible;
    let sat_visible = track_node.satellites_visible;
    let cm_visible = track_node.custom_markers_visible;
    let tpv_expanded = track_node.categories_expanded.contains(&DataCategory::Tpv);
    let sat_expanded = track_node
        .categories_expanded
        .contains(&DataCategory::SatelliteReport);
    let cm_expanded = track_node
        .categories_expanded
        .contains(&DataCategory::CustomMarker);
    let em_expanded = track_node
        .categories_expanded
        .contains(&DataCategory::EventMarker);
    let channels_expanded = track_node.channels_expanded;
    let em_agg = track_node.event_paths.aggregate();
    let event_filter = track_node.event_filter.clone();

    let track_resp = ui.horizontal(|ui| {
        let chk = tri_checkbox(
            ui,
            if track_visible {
                CheckState::On
            } else {
                CheckState::Off
            },
        );
        if chk.clicked() {
            ctx.tree
                .set_category_visible(track_ref, DataCategory::Track, !track_visible);
        }
        let resp = ui.label("Track polyline");
        masked_hint(ui, ctx.display_mask, DataCategory::Track);
        resp
    });
    if track_resp.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: DataCategory::Track,
        });
    }

    render_category_section(
        ui,
        track_ref,
        DataCategory::Tpv,
        track.points.len(),
        "Track points",
        tpv_visible,
        tpv_expanded,
        ctx.display_mask,
        ctx.tree,
        ctx.highlight,
        |ui, highlight| {
            render_tpv_items(
                ui,
                track_ref,
                track,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            );
        },
    );

    let sat_count = track
        .points
        .iter()
        .filter(|p| p.satellites.is_some())
        .count();
    render_category_section(
        ui,
        track_ref,
        DataCategory::SatelliteReport,
        sat_count,
        "Satellite reports",
        sat_visible,
        sat_expanded,
        ctx.display_mask,
        ctx.tree,
        ctx.highlight,
        |ui, highlight| {
            render_satellite_report_items(
                ui,
                track_ref,
                track,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            );
        },
    );

    render_category_section(
        ui,
        track_ref,
        DataCategory::CustomMarker,
        track.custom_markers.len(),
        "Custom markers",
        cm_visible,
        cm_expanded,
        ctx.display_mask,
        ctx.tree,
        ctx.highlight,
        |ui, highlight| {
            render_custom_marker_items(
                ui,
                track_ref,
                track,
                highlight,
                ctx.map_center_request,
                ctx.popup_pos_request,
            );
        },
    );

    render_generated_markers_section(ui, track_ref, track, ctx);

    if !track.event_markers.is_empty() {
        render_event_markers_section(
            ui,
            track_ref,
            track,
            em_agg,
            em_expanded,
            &event_filter,
            ctx,
        );
    }

    if !track.channels.is_empty() {
        render_channels_section(ui, track_ref, track, channels_expanded, ctx);
    }
}

/// A read-only list of the track's ad-hoc sensor channels. Channels are not yet
/// shown on the map or queryable, so this section has no visibility toggle - it
/// only surfaces what was loaded.
fn render_channels_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    is_open: bool,
    ctx: &mut PanelContext<'_>,
) {
    let count = track.channels.len();
    let header = ui.horizontal(|ui| {
        // Pad the checkbox column so the label aligns with the toggleable
        // sections above, even though channels have nothing to toggle.
        ui.add_space(checkbox_width(ui));
        let arrow = expand_arrow(is_open);
        ui.selectable_label(is_open, format!("{arrow} Channels  {count}"))
    });
    if header.inner.clicked() {
        ctx.tree.toggle_channels_expanded(track_ref);
    }

    if !is_open {
        return;
    }

    ui.indent(("channels", track_ref), |ui| {
        for channel in &track.channels {
            let unit = channel
                .unit
                .as_ref()
                .map(|u| format!(" ({u})"))
                .unwrap_or_default();
            let components = if channel.is_vector() {
                format!("  [{}]", channel.components.join(", "))
            } else {
                String::new()
            };
            let samples = channel.times.len();
            ui.label(format!(
                "{}{unit}  {samples} samples{components}",
                channel.name
            ));
        }
    });
}

fn render_event_markers_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    em_agg: CheckState,
    is_open: bool,
    filter_text: &str,
    ctx: &mut PanelContext<'_>,
) {
    let count = track.event_markers.len();
    let header_response = ui.horizontal(|ui| {
        let chk_resp = tri_checkbox(ui, em_agg);
        if chk_resp.clicked() {
            ctx.tree.toggle_all_event_paths(track_ref);
        }
        let arrow = expand_arrow(is_open);
        let label = format!("{arrow} Events  {count}");
        let resp = ui.selectable_label(false, label);
        masked_hint(ui, ctx.display_mask, DataCategory::EventMarker);
        resp
    });

    if header_response.inner.clicked() {
        ctx.tree
            .toggle_category_expanded(track_ref, DataCategory::EventMarker);
    }
    if header_response.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: DataCategory::EventMarker,
        });
    }

    if !is_open {
        return;
    }

    let header_id = egui::Id::new(("events_section", track_ref));

    ui.horizontal(|ui| {
        ui.add_space(16.0);
        let mut text = filter_text.to_owned();
        let resp = ui.add(
            TextEdit::singleline(&mut text)
                .hint_text("Filter…")
                .desired_width(120.0)
                .id(egui::Id::new(("event_filter", header_id))),
        );
        if resp.changed()
            && let Some(track_node) = ctx.tree.track_node_mut(track_ref)
        {
            track_node.event_filter = text.clone();
        }
        if !text.is_empty()
            && ui.small_button("×").clicked()
            && let Some(track_node) = ctx.tree.track_node_mut(track_ref)
        {
            track_node.event_filter.clear();
        }
    });

    let current_filter = ctx
        .tree
        .track_node(track_ref)
        .map_or("", |t| t.event_filter.as_str());

    let mut paths: Vec<&str> = track
        .event_markers
        .iter()
        .map(|m| m.variant_path.as_str())
        .collect();
    paths.sort_unstable();
    paths.dedup();

    let filtered: Vec<&str> = if current_filter.is_empty() {
        paths
    } else {
        paths
            .into_iter()
            .filter(|p| p.contains(current_filter))
            .collect()
    };

    let mut prefix_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for path in &filtered {
        let segments: Vec<&str> = path.split('/').collect();
        for depth in 1..=segments.len() {
            if let Some(slice) = segments.get(..depth) {
                prefix_set.insert(slice.join("/"));
            }
        }
    }

    // No max_height cap - expands inline with the track's content.
    for prefix in &prefix_set {
        let depth = prefix.chars().filter(|&c| c == '/').count();
        let segment = prefix.split('/').next_back().unwrap_or(prefix.as_str());
        let marker_count = track
            .event_markers
            .iter()
            .filter(|m| {
                m.variant_path == *prefix || m.variant_path.starts_with(&format!("{prefix}/"))
            })
            .count();

        let node_check = ctx
            .tree
            .track_node(track_ref)
            .and_then(|t| t.event_paths.nodes.get(prefix.as_str()).copied())
            .unwrap_or(CheckState::On);

        ui.horizontal(|ui| {
            ui.add_space(16.0 + depth as f32 * 12.0);
            let chk_resp = tri_checkbox(ui, node_check);
            if chk_resp.clicked() {
                ctx.tree.toggle_event_path(track_ref, prefix);
            }
            ui.label(format!("{segment}  {marker_count}"));
        });
    }
}

fn render_tpv_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, point) in track.points.iter().enumerate() {
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::Tpv,
            point_index: PointIdx::new(pi),
        };
        let label = point.tpv.time().utc().format("%H:%M:%S").to_string();
        let lat_lon = (point.tpv.lat().as_degrees(), point.tpv.lon().as_degrees());
        point_item_row(
            ui,
            point_ref,
            label,
            lat_lon,
            highlight,
            map_center_request,
            popup_pos_request,
        );
    }
}

fn render_satellite_report_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, point) in track.points.iter().enumerate() {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::SatelliteReport,
            point_index: PointIdx::new(pi),
        };
        let time_str = sats.best_time().map_or_else(
            || gt_ui_theme::EM_DASH.to_string(),
            |t| t.format("%H:%M:%S").to_string(),
        );
        let label = format!(
            "{time_str}  {}/{}",
            sats.fix_count(),
            sats.satellite_count()
        );
        let lat_lon = (point.tpv.lat().as_degrees(), point.tpv.lon().as_degrees());
        point_item_row(
            ui,
            point_ref,
            label,
            lat_lon,
            highlight,
            map_center_request,
            popup_pos_request,
        );
    }
}

fn render_custom_marker_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    highlight: &mut MapHighlight,
    map_center_request: &mut Option<(f64, f64)>,
    popup_pos_request: &mut Option<egui::Pos2>,
) {
    for (pi, marker) in track.custom_markers.iter().enumerate() {
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::CustomMarker,
            point_index: PointIdx::new(pi),
        };
        let label = format!("{}  {}", marker.time.format("%H:%M:%S"), marker.label);
        let lat_lon = (marker.lat.as_degrees(), marker.lon.as_degrees());
        point_item_row(
            ui,
            point_ref,
            label,
            lat_lon,
            highlight,
            map_center_request,
            popup_pos_request,
        );
    }
}

/// Render the "Generated markers" section as a tree: a category header (master
/// show/hide + expand) over one collapsible, individually toggleable group per
/// event type, with the markers of each type beneath their group.
fn render_generated_markers_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    ctx: &mut PanelContext<'_>,
) {
    let count = track.generated_markers.len();
    if count == 0 {
        return;
    }
    let Some(node) = ctx.tree.track_node(track_ref) else {
        return;
    };
    let visible = node.generated_markers_visible;
    let expanded = node
        .categories_expanded
        .contains(&DataCategory::GeneratedMarker);

    let header = ui.horizontal(|ui| {
        let chk = tri_checkbox(
            ui,
            if visible {
                CheckState::On
            } else {
                CheckState::Off
            },
        );
        if chk.clicked() {
            ctx.tree
                .set_category_visible(track_ref, DataCategory::GeneratedMarker, !visible);
        }
        let arrow = expand_arrow(expanded);
        let resp = ui.selectable_label(expanded, format!("{arrow} Generated markers  {count}"));
        masked_hint(ui, ctx.display_mask, DataCategory::GeneratedMarker);
        resp
    });
    if header.inner.clicked() {
        ctx.tree
            .toggle_category_expanded(track_ref, DataCategory::GeneratedMarker);
    }
    if header.inner.hovered() {
        ctx.highlight.hover = Some(HighlightScope::TrackCategory {
            track: track_ref,
            category: DataCategory::GeneratedMarker,
        });
    }
    if !expanded {
        return;
    }

    // Group markers by event type, ordered by the tag's variant order.
    let mut groups: std::collections::BTreeMap<gt_types::GeneratedMarkerKindTag, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (pi, marker) in track.generated_markers.iter().enumerate() {
        groups.entry(marker.kind.tag()).or_default().push(pi);
    }

    ui.indent((DataCategory::GeneratedMarker, track_ref), |ui| {
        for (&tag, indices) in &groups {
            let tag_count = indices.len();
            let tag_hidden = !ctx.tree.generated_kind_visible(track_ref, tag);
            let tag_expanded = ctx.tree.generated_kind_expanded(track_ref, tag);

            let row = ui.horizontal(|ui| {
                let chk = tri_checkbox(
                    ui,
                    if tag_hidden {
                        CheckState::Off
                    } else {
                        CheckState::On
                    },
                );
                if chk.clicked() {
                    ctx.tree.toggle_generated_kind_hidden(track_ref, tag);
                }
                let arrow = expand_arrow(tag_expanded);
                ui.selectable_label(
                    tag_expanded,
                    format!("{arrow} {}  {tag_count}", tag.label()),
                )
            });
            if row.inner.clicked() {
                ctx.tree.toggle_generated_kind_expanded(track_ref, tag);
            }
            if !tag_expanded {
                continue;
            }
            ui.indent((tag, track_ref), |ui| {
                for &pi in indices {
                    let Some(marker) = track.generated_markers.get(pi) else {
                        continue;
                    };
                    let point_ref = DataPointRef {
                        track: track_ref,
                        category: DataCategory::GeneratedMarker,
                        point_index: PointIdx::new(pi),
                    };
                    // A multi-satellite slip shows its satellite count; the
                    // others need no per-marker detail beyond the time.
                    let detail = match &marker.kind {
                        GeneratedMarkerKind::Slip(event) if event.slips.len() > 1 => {
                            format!("  ({})", event.slips.len())
                        }
                        _ => String::new(),
                    };
                    let label = format!("{}{detail}", marker.time.format("%H:%M:%S"));
                    let lat_lon = (marker.lat.as_degrees(), marker.lon.as_degrees());
                    point_item_row(
                        ui,
                        point_ref,
                        label,
                        lat_lon,
                        ctx.highlight,
                        ctx.map_center_request,
                        ctx.popup_pos_request,
                    );
                }
            });
        }
    });
}

fn file_bounding_center(file: Option<&LoadedFile>) -> Option<(f64, f64)> {
    let tracks = &file?.tracks;
    if tracks.is_empty() {
        return None;
    }
    let min_lat = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.min().y)
        .fold(f64::INFINITY, f64::min);
    let max_lat = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.max().y)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_lon = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.min().x)
        .fold(f64::INFINITY, f64::min);
    let max_lon = tracks
        .iter()
        .map(|t| t.metadata.bounding_box.max().x)
        .fold(f64::NEG_INFINITY, f64::max);
    Some(((min_lat + max_lat) / 2.0, (min_lon + max_lon) / 2.0))
}

/// Build a file's side-panel display name from the user template.
///
/// `identity` is the file's raw recording identity (or `None`); its internal
/// `auto:` marker is stripped for the `{identity}` token so it never leaks into
/// the label. `filename` is the already-common-prefix-stripped name used for the
/// `{filename}` token and as the ultimate fallback.
fn recording_display_name(
    template: &str,
    metadata: &FileMetadata,
    identity: Option<&str>,
    filename: &str,
) -> String {
    let fields = NameFields {
        title: metadata.title.as_deref(),
        device: metadata.device.as_deref(),
        identity: identity.map(|id| gt_loaded_files::display_identity(id).0),
        filename,
    };
    render_name_template(template, &fields)
}

/// Returns the byte offset at which each name's *display* form begins — i.e.
/// the length of the longest common directory prefix shared by all names.
///
/// Returns `0` when there is nothing meaningful to strip: fewer than two names,
/// no name contains a path separator (so there is no directory structure to
/// collapse), or the common bytes do not reach a separator boundary.
fn strip_common_path_prefix(names: &[&str]) -> usize {
    if names.len() < 2 {
        return 0;
    }
    if !names.iter().any(|n| n.contains(['/', '\\'])) {
        return 0;
    }
    let Some(&first) = names.first() else {
        return 0;
    };
    // Count matching bytes between `first` and every other name.
    let common_bytes = names.iter().skip(1).fold(first.len(), |acc, name| {
        let len = first
            .bytes()
            .zip(name.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        acc.min(len)
    });
    // `common_bytes` may land in the middle of a multi-byte character; snap to a
    // valid char boundary before searching for the last separator. Because the
    // leading `common_bytes` bytes are identical in all names, the resulting
    // index is a valid char boundary in every name.
    let common_bytes = first.floor_char_boundary(common_bytes);
    match first.get(..common_bytes).and_then(|s| s.rfind(['/', '\\'])) {
        // +1 to start the display name after the separator itself.
        Some(pos) => pos + 1,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{FileMetadata, recording_display_name, strip_common_path_prefix};

    #[test]
    fn display_name_strips_auto_prefix_from_identity_token() {
        let meta = FileMetadata::default();
        assert_eq!(
            recording_display_name("{identity}", &meta, Some("auto:Morning ride"), "ride.gtd"),
            "Morning ride"
        );
    }

    #[test]
    fn display_name_uses_metadata_and_falls_back_to_filename() {
        let meta = FileMetadata {
            title: Some("Morning ride".to_owned()),
            device: Some("uBlox F9P".to_owned()),
            ..FileMetadata::default()
        };
        assert_eq!(
            recording_display_name("{title} — {device}", &meta, None, "ride.gtd"),
            "Morning ride — uBlox F9P"
        );
        // No title/device and no identity: the filename carries the label.
        let empty = FileMetadata::default();
        assert_eq!(
            recording_display_name("{title}", &empty, None, "ride.gtd"),
            "ride.gtd"
        );
    }

    #[test]
    fn empty_slice_returns_zero() {
        assert_eq!(strip_common_path_prefix(&[]), 0);
    }

    #[test]
    fn single_name_returns_zero() {
        assert_eq!(
            strip_common_path_prefix(&["/home/user/recordings/ride.gtd"]),
            0
        );
    }

    #[test]
    fn no_path_separators_returns_zero() {
        assert_eq!(strip_common_path_prefix(&["ride_0.gtd", "ride_1.gtd"]), 0);
    }

    #[test]
    fn shared_directory_prefix_is_stripped() {
        let names = [
            "/home/user/recordings/2024-01-15.gtd",
            "/home/user/recordings/2024-01-16.gtd",
        ];
        assert_eq!(
            strip_common_path_prefix(&names),
            "/home/user/recordings/".len()
        );
    }

    #[test]
    fn common_bytes_mid_component_trims_to_last_separator() {
        // "/home/user/recordings/…" vs "/home/user/recent/…" share "/home/user/"
        // even though more bytes match inside the next component.
        let names = ["/home/user/recordings/a.gtd", "/home/user/recent/b.gtd"];
        assert_eq!(strip_common_path_prefix(&names), "/home/user/".len());
    }

    #[test]
    fn no_common_directory_prefix_strips_only_root_slash() {
        // The only shared byte is the leading '/', so we strip that.
        let names = ["/alpha/a.gtd", "/beta/b.gtd"];
        assert_eq!(strip_common_path_prefix(&names), 1);
    }

    #[test]
    fn truly_no_common_prefix_returns_zero() {
        let names = ["alpha/a.gtd", "beta/b.gtd"];
        assert_eq!(strip_common_path_prefix(&names), 0);
    }

    #[test]
    fn windows_backslash_separator() {
        let names = [
            r"C:\Users\alice\recordings\ride_a.gtd",
            r"C:\Users\alice\recordings\ride_b.gtd",
        ];
        assert_eq!(
            strip_common_path_prefix(&names),
            r"C:\Users\alice\recordings\".len()
        );
    }
}

#[cfg(test)]
mod snap_action_tests {
    use std::collections::HashMap;

    use rstest::rstest;

    use super::*;

    fn view(offline: bool, consent_pending: bool) -> SnapPanelView<'static> {
        // The rows map is irrelevant to snap_action; a leaked empty map keeps
        // the borrow 'static for the test helper.
        static EMPTY: std::sync::OnceLock<HashMap<TrackRef, SnapRowView>> =
            std::sync::OnceLock::new();
        SnapPanelView {
            offline,
            consent_pending,
            rows: EMPTY.get_or_init(HashMap::new),
        }
    }

    fn unsnappable() -> SnapRowView {
        SnapRowView::Unsnappable {
            travel_mode: "Boat".to_owned(),
        }
    }

    fn failed() -> SnapRowView {
        SnapRowView::Failed {
            error: "server unreachable".to_owned(),
        }
    }

    fn done() -> SnapRowView {
        SnapRowView::Done {
            snapped: 1,
            interpolated: 2,
            unsnapped: 3,
            confidence_score: None,
            shown: true,
            stale: None,
        }
    }

    fn stale_done() -> SnapRowView {
        SnapRowView::Done {
            snapped: 1,
            interpolated: 2,
            unsnapped: 3,
            confidence_score: None,
            shown: true,
            stale: Some("Snapped as Bicycle - would now snap as Auto".to_owned()),
        }
    }

    /// Pins the trigger's priority order: a current Done never has an action
    /// (its status glyph stays usable offline), a stale Done offers the
    /// re-run (grayed offline - it needs the network), Unsnappable beats
    /// offline (the permanent condition names the mode), offline grays
    /// everything else, only Idle, Failed, and stale Done are clickable, and
    /// only clickable states carry the consent-pending `…` suffix.
    #[rstest]
    #[case(SnapRowView::Idle, false, Some(true))]
    #[case(SnapRowView::Idle, true, Some(false))]
    #[case(failed(), false, Some(true))]
    #[case(failed(), true, Some(false))]
    #[case(unsnappable(), false, Some(false))]
    #[case(unsnappable(), true, Some(false))]
    #[case(SnapRowView::Queued, false, Some(false))]
    #[case(SnapRowView::Queued, true, Some(false))]
    #[case(SnapRowView::InFlight { completed_chunks: 1, total_chunks: 2 }, false, Some(false))]
    #[case(SnapRowView::InFlight { completed_chunks: 1, total_chunks: 2 }, true, Some(false))]
    #[case(done(), false, None)]
    #[case(done(), true, None)]
    #[case(stale_done(), false, Some(true))]
    #[case(stale_done(), true, Some(false))]
    fn action_enablement_per_state_and_offline(
        #[case] row: SnapRowView,
        #[case] offline: bool,
        #[case] expected_enabled: Option<bool>,
    ) {
        let action = snap_action(&row, view(offline, false));
        assert_eq!(action.map(|a| a.enabled), expected_enabled);
    }

    /// The disabled hover must name the reason: the declared travel mode for
    /// unsnappable tracks (even offline - the permanent condition wins), and
    /// the offline switch for everything else.
    #[rstest]
    #[case(unsnappable(), "Boat")]
    #[case(SnapRowView::Idle, "GEOTRACE_OFFLINE")]
    #[case(failed(), "GEOTRACE_OFFLINE")]
    fn offline_hover_names_the_blocking_condition(
        #[case] row: SnapRowView,
        #[case] expected_substring: &str,
    ) {
        let action = snap_action(&row, view(true, false)).map(|a| a.hover);
        let hover = action.unwrap_or_default();
        assert!(
            hover.contains(expected_substring),
            "hover {hover:?} should mention {expected_substring:?}"
        );
    }

    /// The `…` suffix marks a click that still needs the consent dialog - so
    /// it only ever appears on clickable states, never on grayed ones.
    #[rstest]
    #[case(SnapRowView::Idle, true)]
    #[case(failed(), true)]
    #[case(stale_done(), true)]
    #[case(unsnappable(), false)]
    #[case(SnapRowView::Queued, false)]
    fn consent_suffix_only_on_clickable_states(#[case] row: SnapRowView, #[case] expected: bool) {
        let action = snap_action(&row, view(false, true));
        assert_eq!(action.map(|a| a.consent_pending), Some(expected));
    }
}
