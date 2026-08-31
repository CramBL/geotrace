use egui::{Button, Label, RichText, ScrollArea, Sides, TextEdit};
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use egui_phosphor::regular::CLOCK as ICON_CLOCK;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::LINE_SEGMENTS as ICON_LINE_SEGMENTS;
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PATH as ICON_PATH;
use egui_phosphor::regular::ROAD_HORIZON as ICON_ROAD_HORIZON;
use egui_phosphor::regular::TRASH as ICON_TRASH;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use gt_filter::GlobalFilter;
use gt_loaded_files::{LoadedFilesView, RecordingNames};
use gt_types::{
    DataCategory, FileIdx, GeneratedMarkerKind, GeoBounds, LoadWarning, LoadedFile, LoadedTrack,
    PointIdx, TrackGeometry, TrackIdx, TrackRef,
};
use gt_ui_theme::ELLIPSIS;
use gt_ui_theme::buttons::FramelessIconButton;
use gt_ui_types::{
    DataPointRef, DisplayCategory, DisplayMask, HighlightScope, MapHighlight, MapScope,
    QueryMatches, SnapCosting,
};
use rustc_hash::FxHashMap;

use crate::filter::{FilterPanelState, render_filter_panel};
use crate::track_columns::{self, TrackColumnCells, TrackColumnWidths, TrackRowCellColor};
use crate::tree::{CheckState, DeleteConfirmState, NodeKey, TreeState};
use crate::widgets::{
    CHECKBOX_PADDING, MetadataView, PointClickRequests, checkbox_width, expand_arrow,
    expand_arrow_width, has_metadata_details, paint_map_hover_bg, point_item_row,
    recording_tooltip_rows, tri_checkbox,
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
    /// GeoTrace runs offline: every snap trigger is grayed out.
    pub offline: bool,
    /// Upload consent has not been acknowledged for the configured server, so
    /// the trigger carries the `…` suffix - a click opens the consent dialog.
    pub consent_pending: bool,
    /// Per-track snap state. Tracks without an entry are [`SnapRowView::Idle`].
    pub rows: &'a FxHashMap<TrackRef, SnapRowView>,
    /// The costing choices of the re-run submenu, labels pre-rendered by
    /// the app from the wire type's canonical spelling.
    pub costing_choices: &'a [(SnapCosting, String)],
    /// The global snapping activity, for the progress strip at the panel
    /// bottom.
    pub progress: &'a SnapProgressView,
}

/// The global snapping activity: what runs, what waits. Rendered as the
/// progress strip pinned to the panel bottom while anything is pending.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapProgressView {
    /// The run whose request is in flight, if any.
    pub in_flight: Option<SnapInFlightView>,
    /// Tracks waiting in the queue (including autos parked while hidden).
    pub queued: usize,
}

impl SnapProgressView {
    /// Whether anything is pending - the strip only renders then.
    pub fn active(&self) -> bool {
        self.in_flight.is_some() || self.queued > 0
    }
}

/// The in-flight run as the progress strip shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapInFlightView {
    /// The track being snapped, resolved to its display name by the panel.
    pub track: TrackRef,
    pub completed_chunks: usize,
    pub total_chunks: usize,
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
    /// The track has no real fix a snap run could send.
    NothingToSend,
    Done {
        snapped: usize,
        interpolated: usize,
        unsnapped: usize,
        /// Run confidence in `0..=1`, when the server reported one.
        confidence_score: Option<f64>,
        /// Whether the snapped track is currently drawn on the map.
        shown: bool,
        /// `Some` when the run is stale - produced under parameters or a
        /// server that differ from the current settings. Each entry names
        /// one difference; the row offers a re-run. `None` = current.
        stale: Option<Vec<String>>,
        /// At least one chunk failed and left a gap in the result.
        partial: bool,
        /// The run's warnings, pre-rendered by the app (the panel has no
        /// gt-snap dependency), shown in the status hover.
        warnings: Vec<String>,
    },
}

/// What a "Snap again as" choice applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapCostingTarget {
    Track(TrackRef),
    /// The recording's tracks, narrowed to a scope by the dialog the app
    /// raises next.
    Recording(FileIdx),
}

pub struct PanelContext<'a> {
    pub loaded_files: LoadedFilesView<'a>,
    pub tree: &'a mut TreeState,
    pub highlight: &'a mut MapHighlight,
    pub filter: &'a mut GlobalFilter,
    pub filter_state: &'a mut FilterPanelState,
    pub map_center_request: &'a mut Option<(f64, f64)>,
    pub popup_pos_request: &'a mut Option<egui::Pos2>,
    /// The last query run's effect, so a point row cannot pin a point the query
    /// removed from the map. `None` until a query has run.
    pub query_matches: Option<&'a QueryMatches>,
    pub zoom_to_visible_request: &'a mut bool,
    /// Set by clicking the ⚠ icon on a file row. Consumed by the app to show a centered dialog.
    pub warnings_request: &'a mut Option<(String, Vec<LoadWarning>)>,
    /// Set when "Reset filters" is clicked, so the app can also drop the query
    /// filter (which the side panel cannot reach directly).
    pub clear_query_request: &'a mut bool,
    /// The map's display mask, read to hint on category rows whose ink the
    /// display toggles currently hide.
    pub display_mask: DisplayMask,
    /// Display name of each loaded file, resolved by the app from the user's
    /// recording-name template.
    pub recording_names: &'a RecordingNames,
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
    /// Set by the "Snap again as" submenu: re-run the track under an
    /// explicit costing. Consumed by the app, which stores the session
    /// override and queues the run (through the consent dialog if pending).
    pub snap_costing_request: &'a mut Option<(SnapCostingTarget, SnapCosting)>,
    /// Set by the "Show sky trails" track action. Consumed by the app, which
    /// opens the sky trails window on that track.
    pub sky_trails_request: &'a mut Option<gt_ui_types::SkyTrailsRequest>,
}

/// Trailing eye-slash on a category row whose map ink is hidden by the display
/// toggles.
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

    fn track(&self, track_ref: TrackRef) -> Option<&'a LoadedTrack> {
        self.file(track_ref.fi)
            .and_then(|file| track_ref.index.get(&file.tracks))
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
                    let passes = gt_filter::track_passes_filter(track, &filter_snapshot);
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

    // The progress strip pins to the panel bottom as an inner panel; the
    // file tree scrolls in the remaining central space.
    if ctx.snap.progress.active() {
        egui::Panel::bottom("snap_progress_strip").show(ui, |ui| {
            snap_progress_strip(ui, ctx.snap, ctx.recording_names, ctx.files());
        });
    }
    // The tree and filter as the frame started, which is what the map drew
    // from, so a point row's pin gate agrees with what is on screen. Toggles
    // made further down this frame land on the next one.
    let frame_visibility = ctx.tree.visibility().clone();
    let scope = MapScope {
        files: ctx.files(),
        visibility: &frame_visibility,
        filter: &filter_snapshot,
        display_mask: ctx.display_mask,
        query_matches: ctx.query_matches,
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            render_visible_tracks_section(ui, ctx);
            // A scroll area defaults to at least 64 points tall, which would
            // push the tree past the height the section leaves it.
            ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let columns = TreeTrackColumns::measure(ui, ctx);
                    let names = ctx.recording_names;
                    for fi in 0..ctx.files().len() {
                        let fi = FileIdx::new(fi);
                        let display_name = names.get(fi).unwrap_or_default();
                        render_file_row(ui, fi, display_name, scope, &columns, ctx);
                    }
                });
        });
    ctx.tree.reveal_request = None;
}

/// The global snap progress strip: the in-flight run with its chunk
/// progress, the queue length, and the offline pause - visible only while
/// something is pending, so an idle panel loses no space.
fn snap_progress_strip(
    ui: &mut egui::Ui,
    snap: SnapPanelView<'_>,
    display_names: &RecordingNames,
    files: &[gt_types::LoadedFile],
) {
    let progress = snap.progress;
    ui.add_space(STRIP_PADDING);
    // The current action, with the queue length at the right edge. The
    // horizontal wrapper keeps the right-to-left layout from claiming the
    // strip's full height (which would push the bar out).
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if progress.queued > 0 {
                ui.label(RichText::new(format!("{} queued", progress.queued)).weak());
            }
            match &progress.in_flight {
                Some(run) => {
                    let label = display_names
                        .track_label(files, run.track)
                        .unwrap_or_default();
                    // Chunk currently being fetched, 1-based; a run only stays
                    // in flight while at least one chunk remains.
                    let current = (run.completed_chunks + 1).min(run.total_chunks);
                    ui.add(
                        Label::new(format!(
                            "Snapping {label} - chunk {current}/{}",
                            run.total_chunks
                        ))
                        .truncate(),
                    );
                }
                // Queued work with nothing in flight: paused (offline) or the
                // moment between dispatches. The label always states which.
                None if snap.offline => {
                    ui.label(
                        RichText::new("Snapping paused - offline")
                            .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                    );
                }
                None => {
                    ui.label(RichText::new("Waiting to snap").weak());
                }
            }
        });
    });
    if let Some(run) = &progress.in_flight {
        let fraction = if run.total_chunks == 0 {
            0.0
        } else {
            run.completed_chunks as f32 / run.total_chunks as f32
        };
        ui.add(egui::ProgressBar::new(fraction).desired_height(PROGRESS_BAR_HEIGHT));
    }
    ui.add_space(STRIP_PADDING);
}

const PROGRESS_BAR_HEIGHT: f32 = 4.0;

/// Vertical padding above and below the strip contents.
const STRIP_PADDING: f32 = 2.0;

/// Spacing between a recording row's leading controls.
const CHECKBOX_GROUP_SPACING: f32 = 2.0;

/// Id of the visible-tracks panel. Its rows take their widget ids from it,
/// which keeps them distinct from the tree row of the same track.
const VISIBLE_SECTION_ID: &str = "visible_tracks_section";

/// The share of the region the section and the tree divide that the section
/// takes until the divider is dragged.
pub const VISIBLE_SECTION_DEFAULT_FRACTION: f32 = 0.25;

/// The largest share of that region the divider can give the section.
const VISIBLE_SECTION_MAX_FRACTION: f32 = 0.75;

/// The smallest section height, in track rows.
const VISIBLE_SECTION_MIN_ROWS: f32 = 2.0;

/// The interact height the section lays its rows out to, tighter than the
/// tree's so more rows fit.
const VISIBLE_SECTION_INTERACT_HEIGHT: f32 = 13.0;

/// The vertical gap between the section's rows.
const VISIBLE_SECTION_ROW_SPACING: f32 = 1.0;

/// The icon width the section draws its checkboxes from. The glyph is drawn at
/// the width plus four points, small enough that a row stays as tall as its
/// label.
const VISIBLE_SECTION_ICON_WIDTH: f32 = 10.0;

/// A track row of the section. The cells are built before any row draws: the
/// column widths come from measuring them.
struct VisibleTrackRow {
    track_ref: TrackRef,
    cells: TrackColumnCells,
}

/// The visible-tracks section above the tree: the recordings and the tracks
/// toggled on right now, however far the tree is scrolled. Its height is a
/// fixed share of the region it shares with the tree, changed only by dragging
/// the divider on its lower edge. The share comes from
/// [`TreeState::visible_section_fraction`], clamped to what the divider
/// reaches, and this function writes the rendered share back there for the app
/// to persist.
fn render_visible_tracks_section(ui: &mut egui::Ui, ctx: &mut PanelContext<'_>) {
    let groups = ctx.tree.visible_tracks_by_file();
    let rows: Vec<Vec<VisibleTrackRow>> = groups
        .iter()
        .map(|group| {
            group
                .tracks
                .iter()
                .filter_map(|&track_ref| {
                    let cells = TrackColumnCells::for_track(ctx.track(track_ref)?);
                    Some(VisibleTrackRow { track_ref, cells })
                })
                .collect()
        })
        .collect();
    let column_widths = TrackColumnWidths::measure(ui, rows.iter().flatten().map(|row| &row.cells));
    let region_height = ui.available_height();
    let row_pitch =
        VISIBLE_SECTION_INTERACT_HEIGHT + CHECKBOX_PADDING + VISIBLE_SECTION_ROW_SPACING;
    let min_height = row_pitch * VISIBLE_SECTION_MIN_ROWS;
    let max_height = (region_height * VISIBLE_SECTION_MAX_FRACTION).max(min_height);
    let stored_height =
        (region_height * ctx.tree.visible_section_fraction()).clamp(min_height, max_height);

    let section = egui::Panel::top(VISIBLE_SECTION_ID)
        .resizable(true)
        .frame(egui::Frame::side_top_panel(ui.style()).fill(ui.visuals().faint_bg_color))
        .default_size(stored_height)
        .size_range(min_height..=max_height)
        .show(ui, |ui| {
            let spacing = ui.spacing_mut();
            spacing.item_spacing.y = VISIBLE_SECTION_ROW_SPACING;
            spacing.interact_size.y = VISIBLE_SECTION_INTERACT_HEIGHT;
            spacing.icon_width = VISIBLE_SECTION_ICON_WIDTH;

            // The scroll area fills the section's height whatever it holds, so
            // the tree below keeps its place when the last track is hidden.
            ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if groups.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.label(RichText::new("No tracks visible").weak());
                        });
                        return;
                    }
                    let leading_space =
                        ui.spacing().indent + checkbox_width(ui) + ui.spacing().item_spacing.x;
                    track_columns::render_header(ui, leading_space, column_widths);
                    let names = ctx.recording_names;
                    for (group, rows) in groups.iter().zip(&rows) {
                        let display_name = names.get(group.file).unwrap_or_default();
                        render_visible_file_caption(ui, group.file, display_name, ctx);
                        ui.indent(group.file, |ui| {
                            for row in rows {
                                render_visible_track_row(
                                    ui,
                                    row.track_ref,
                                    &row.cells,
                                    column_widths,
                                    ctx,
                                );
                            }
                        });
                    }
                });
        });

    ctx.tree
        .set_visible_section_fraction(section.response.rect.height() / region_height);
}

/// The "Show only this track" entry of a track row's context menu, in the tree
/// and in the Visible section.
fn show_only_track_menu_entry(ui: &mut egui::Ui, track_ref: TrackRef, ctx: &mut PanelContext<'_>) {
    if ui.button("Show only this track").clicked() {
        ctx.tree.show_only_track(track_ref);
        *ctx.zoom_to_visible_request = true;
        ui.close();
    }
}

/// The line the section groups a recording's track rows under. It names the
/// recording and takes the map hover, and has no control of its own.
fn render_visible_file_caption(
    ui: &mut egui::Ui,
    fi: FileIdx,
    display_name: &str,
    ctx: &mut PanelContext<'_>,
) {
    let Some(file) = ctx.file(fi) else {
        return;
    };
    let map_hovered = ctx.highlight.hovers_anything_in_file(fi);

    let response = ui
        .add(Label::new(RichText::new(display_name).small().weak().italics()).truncate())
        .on_hover_ui(|ui| recording_tooltip_rows(ui, &file.metadata));

    if map_hovered {
        paint_map_hover_bg(
            ui,
            response.rect,
            gt_ui_theme::map_hover_color(ui.visuals().dark_mode),
        );
    }
    if response.hovered() {
        ctx.highlight.hover = Some(HighlightScope::File { file_index: fi });
    }
}

fn render_visible_track_row(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    cells: &TrackColumnCells,
    column_widths: TrackColumnWidths,
    ctx: &mut PanelContext<'_>,
) {
    let Some(track) = ctx.track(track_ref) else {
        return;
    };
    let key = NodeKey::Track(track_ref);
    let passes = gt_filter::track_passes_filter(track, ctx.filter);
    let panel_hovered = ctx.highlight.hovers_the_whole_track(track_ref);
    let map_hovered = ctx.highlight.hovers_a_point_of_track(track_ref);
    let is_selected = ctx.tree.selection.contains(&key);
    let cell_color = TrackRowCellColor {
        panel_hovered,
        selected: is_selected,
        passes_filter: passes,
    }
    .resolve(ui);

    let (response, ()) =
        track_columns::render_row_as_one_surface(ui, &track.metadata, cells, is_selected, |ui| {
            if tri_checkbox(ui, CheckState::On).clicked() {
                ctx.tree.hide_track(track_ref);
            }
            cells.paint(ui, column_widths, cell_color);
        });
    if map_hovered {
        paint_map_hover_bg(
            ui,
            response.rect,
            gt_ui_theme::map_hover_color(ui.visuals().dark_mode),
        );
    }
    if response.hovered() {
        ctx.highlight.hover = Some(HighlightScope::Track(track_ref));
    }
    if response.double_clicked() {
        if let Some(center) = track_bounding_center(track) {
            *ctx.map_center_request = Some(center);
        }
    } else if response.clicked() {
        ctx.tree.reveal(key);
    }
    response.context_menu(|ui| {
        show_only_track_menu_entry(ui, track_ref, ctx);
        if ui.button("Hide").clicked() {
            ctx.tree.hide_track(track_ref);
            ui.close();
        }
        if ui.button("Hide recording").clicked() {
            ctx.tree.hide_file(track_ref.fi);
            ui.close();
        }
    });
}

/// The track rows the tree draws right now. The columns line up across
/// recordings: one set of widths is measured over the cells of every expanded
/// recording's tracks.
struct TreeTrackColumns {
    cells: FxHashMap<TrackRef, TrackColumnCells>,
    widths: TrackColumnWidths,
    arrow_width: f32,
    /// The first expanded recording with a track. The header row is drawn
    /// above that recording's track rows, once for the whole tree, and is
    /// `None` while the tree shows no track row.
    header_file: Option<FileIdx>,
}

impl TreeTrackColumns {
    fn measure(ui: &egui::Ui, ctx: &PanelContext<'_>) -> Self {
        let mut cells = Vec::new();
        let mut header_file = None;
        for (fi, file) in ctx.files().iter().enumerate() {
            let fi = FileIdx::new(fi);
            if !ctx.tree.file_node(fi).is_some_and(|node| node.expanded) {
                continue;
            }
            if !file.tracks.is_empty() {
                header_file.get_or_insert(fi);
            }
            for (ti, track) in file.tracks.iter().enumerate() {
                let track_ref = TrackRef::new(fi, TrackIdx::new(ti));
                cells.push((track_ref, TrackColumnCells::for_track(track)));
            }
        }
        let widths = TrackColumnWidths::measure(ui, cells.iter().map(|(_, cells)| cells));
        Self {
            cells: cells.into_iter().collect(),
            widths,
            arrow_width: expand_arrow_width(ui),
            header_file,
        }
    }

    /// What a track row draws before its first column: the checkbox and the
    /// expand arrow, each followed by the layout's gap.
    fn leading_space(&self, ui: &egui::Ui) -> f32 {
        checkbox_width(ui) + self.arrow_width + 2.0 * ui.spacing().item_spacing.x
    }
}

fn render_file_row(
    ui: &mut egui::Ui,
    fi: FileIdx,
    display_name: &str,
    scope: MapScope<'_>,
    columns: &TreeTrackColumns,
    ctx: &mut PanelContext<'_>,
) {
    let Some(file) = ctx.file(fi) else {
        return;
    };
    let Some(file_node) = ctx.tree.file_node(fi) else {
        return;
    };
    let is_expanded = file_node.expanded;
    let check = file_node.check;
    let file_key = NodeKey::File(fi);

    // The plot writes its hover after this panel renders: a plot hover marks the
    // row one frame later.
    let file_map_hovered = ctx.highlight.hovers_anything_in_file(fi);

    let map_hover_bg = gt_ui_theme::map_hover_color(ui.visuals().dark_mode);

    let row_response = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = CHECKBOX_GROUP_SPACING;
        let chk_resp = tri_checkbox(ui, check);
        if chk_resp.clicked() {
            ctx.tree.toggle_file_check(fi);
        }
        // A note icon between the checkbox and the expand arrow opens the details
        // dialog, so metadata is one click away without pushing a block under the
        // row. Only shown when there is something to reveal.
        let identity = ctx.identity(fi);
        if has_metadata_details(&MetadataView::from_file_metadata(&file.metadata, identity)) {
            let icon = FramelessIconButton::new(ICON_NOTE).hover_text_ui(ui, "Recording details");
            if icon.clicked() {
                *ctx.metadata_request = Some(RecordingDetails {
                    metadata: file.metadata.clone(),
                    identity: identity.map(str::to_owned),
                });
            }
        }
        let arrow = expand_arrow(is_expanded);
        let dist = file
            .metadata
            .total_distance
            .measured()
            .map_or_else(|| gt_ui_theme::EM_DASH.to_owned(), gt_fmt::format_distance);
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
                    if FramelessIconButton::new(icon)
                        .hover_text_ui(ui, "Data quality warnings - click for details")
                        .clicked()
                    {
                        *ctx.warnings_request =
                            Some((file.metadata.filename.clone(), file.load_warnings.clone()));
                    }
                }
                ui.label(format!("{ICON_CLOCK} {dur}"))
                    .on_hover_text("Recorded time, excluding the time between tracks");
                ui.label(format!("{ICON_ROAD_HORIZON} {dist}"))
                    .on_hover_text("Total distance");
            },
        );
        resp.on_hover_ui(|ui| recording_tooltip_rows(ui, &file.metadata))
    });

    let file_label_resp = row_response.inner;
    if ctx.tree.reveal_request == Some(file_key) {
        file_label_resp.scroll_to_me(Some(egui::Align::Center));
    }
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
        if ctx.file(fi).is_some_and(|file| !file.tracks.is_empty()) {
            snap_costing_submenu(
                ui,
                SnapCostingTarget::Recording(fi),
                SNAP_AGAIN_AS_LABEL,
                ctx,
            );
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
            if columns.header_file == Some(fi) {
                track_columns::render_header(ui, columns.leading_space(ui), columns.widths);
            }
            let track_count = ctx.file(fi).map_or(0, |f| f.tracks.len());
            for ti in 0..track_count {
                render_track_row(ui, fi, TrackIdx::new(ti), scope, columns, ctx);
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
        // A current completed run leaves nothing to trigger. Its status glyph
        // stays usable offline: cached results are local.
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
        // Like `Unsnappable` this is a property of the data, not of the
        // connection, so it outranks the offline state too.
        SnapRowView::NothingToSend => SnapAction {
            enabled: false,
            hover: NOTHING_TO_SEND_HOVER.to_owned(),
            consent_pending: false,
        },
        _ if snap.offline => SnapAction {
            enabled: false,
            hover: OFFLINE_HOVER.to_owned(),
            consent_pending: false,
        },
        // A stale run keeps its status glyph; this action is the re-run.
        SnapRowView::Done {
            stale: Some(reasons),
            ..
        } => SnapAction {
            enabled: true,
            hover: format!(
                "{}\nClick to snap again with the current settings.",
                reasons.join("\n")
            ),
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

/// The re-run submenu's label for a row, `None` for rows without one.
/// "Snap again as" re-runs an existing (completed or failed) run under an
/// explicit costing; "Snap as" overrides a road-less declared travel mode
/// that never ran. In-progress rows finish first, idle rows keep the plain
/// action.
fn costing_submenu_label(row: &SnapRowView) -> Option<&'static str> {
    match row {
        SnapRowView::Done { .. } | SnapRowView::Failed { .. } => Some(SNAP_AGAIN_AS_LABEL),
        SnapRowView::Unsnappable { .. } => Some("Snap as"),
        SnapRowView::Idle
        | SnapRowView::Queued
        | SnapRowView::InFlight { .. }
        | SnapRowView::NothingToSend => None,
    }
}

/// Extra dimming applied to the status glyph while the snapped track is
/// hidden, so the toggle state is readable at a glance.
const HIDDEN_GLYPH_ALPHA: f32 = 0.5;

/// Label of the costing submenu wherever it re-runs existing results: one
/// track's, or a scope of a recording's.
const SNAP_AGAIN_AS_LABEL: &str = "Snap again as";

/// Hover text of every snap control grayed out by offline mode.
const OFFLINE_HOVER: &str = "Snapping is disabled in offline mode";

/// Hover text of the snap control of a track with no fix worth sending.
const NOTHING_TO_SEND_HOVER: &str =
    "Nothing to snap - this track has no run of real fixes, only receiver dead-reckoning estimates";

/// The label with the `…` suffix while a click still needs the consent
/// dialog (the suffix marks exactly that, per the design).
fn consent_suffixed(label: &str, consent_pending: bool) -> String {
    if consent_pending {
        format!("{label}{ELLIPSIS}")
    } else {
        label.to_owned()
    }
}

/// The toggle hint appended to the snap status hover.
fn snapped_track_toggle_label(shown: bool) -> &'static str {
    if shown {
        "Snapped track shown on the map - click to hide"
    } else {
        "Snapped track hidden - click to show"
    }
}

/// The completed-run breakdown shown in the snap status hover: per-kind
/// point counts, the run confidence, the partial marker, and the run's
/// warnings - anomalies are surfaced here, never hidden.
fn snap_status_rows(
    ui: &mut egui::Ui,
    snapped: usize,
    interpolated: usize,
    unsnapped: usize,
    confidence_score: Option<f64>,
    partial: bool,
    warnings: &[String],
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
    let amber = gt_ui_theme::warning_amber(ui.visuals().dark_mode);
    if partial {
        ui.label(
            RichText::new("Partial result - failed chunks left gaps without snap data")
                .color(amber),
        );
    }
    for warning in warnings {
        ui.label(RichText::new(warning).color(amber));
    }
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
        partial,
        warnings,
    } = row
    {
        let (snapped, interpolated, unsnapped, confidence_score, shown, partial) = (
            *snapped,
            *interpolated,
            *unsnapped,
            *confidence_score,
            *shown,
            *partial,
        );
        // The status glyph doubles as the per-track visibility toggle: weak
        // while the snapped track draws, extra-faint while hidden. Stale and
        // partial runs color it warning-amber, and the hover names which
        // condition applies.
        let text = if stale.is_some() || partial {
            RichText::new(ICON_PATH).color(gt_ui_theme::warning_amber(ui.visuals().dark_mode))
        } else if shown {
            RichText::new(ICON_PATH).weak()
        } else {
            RichText::new(ICON_PATH).weak().color(
                ui.visuals()
                    .weak_text_color()
                    .gamma_multiply(HIDDEN_GLYPH_ALPHA),
            )
        };
        let stale = stale.clone();
        let warnings = warnings.clone();
        let glyph = FramelessIconButton::new(text).hover_tooltip_ui(ui, |ui| {
            snap_status_rows(
                ui,
                snapped,
                interpolated,
                unsnapped,
                confidence_score,
                partial,
                &warnings,
            );
            if let Some(reasons) = &stale {
                ui.label(
                    RichText::new(format!("Stale - {}", reasons.join(", ")))
                        .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                );
            }
            ui.label(RichText::new(snapped_track_toggle_label(shown)).weak());
        });
        if glyph.clicked() {
            *ctx.snap_visibility_request = Some(track_ref);
        }
        glyph.context_menu(|ui| snap_track_costing_submenu(ui, track_ref, row, ctx));
        masked_display_hint(ui, ctx.display_mask, DisplayCategory::SnappedTracks);
    }

    let Some(action) = snap_action(row, ctx.snap) else {
        return;
    };
    // The trigger uses `ICON_LINE_SEGMENTS`, completed runs use `ICON_PATH`.
    let failed = matches!(row, SnapRowView::Failed { .. });
    let label = consent_suffixed(ICON_LINE_SEGMENTS, action.consent_pending);
    let mut text = RichText::new(label);
    if failed {
        text = text.color(gt_ui_theme::warning_amber(ui.visuals().dark_mode));
    }
    let button = FramelessIconButton::new(text)
        .enabled(action.enabled)
        .hover_text_ui(ui, &action.hover);
    snap_trigger_overlay(ui, row, button.rect);
    if button.clicked() {
        *ctx.snap_request = Some(track_ref);
    }
}

/// Corner badge size of the queued-state clock, in points.
const QUEUED_BADGE_FONT: f32 = 9.0;

/// Spinner diameter over the in-flight trigger, in points.
const IN_FLIGHT_SPINNER_SIZE: f32 = 10.0;

/// The in-progress overlays on the (disabled, therefore faded) trigger
/// glyph: a clock badge while queued, a spinner while the request is in
/// flight - waiting and working read differently at a glance.
fn snap_trigger_overlay(ui: &mut egui::Ui, row: &SnapRowView, rect: egui::Rect) {
    match row {
        SnapRowView::Queued => {
            ui.painter().text(
                rect.right_bottom(),
                egui::Align2::RIGHT_BOTTOM,
                ICON_CLOCK,
                egui::FontId::proportional(QUEUED_BADGE_FONT),
                ui.visuals().text_color(),
            );
        }
        SnapRowView::InFlight { .. } => {
            ui.put(
                egui::Rect::from_center_size(
                    rect.center(),
                    egui::Vec2::splat(IN_FLIGHT_SPINNER_SIZE),
                ),
                egui::Spinner::new().size(IN_FLIGHT_SPINNER_SIZE),
            );
        }
        SnapRowView::Idle
        | SnapRowView::Unsnappable { .. }
        | SnapRowView::Failed { .. }
        | SnapRowView::NothingToSend
        | SnapRowView::Done { .. } => {}
    }
}

/// The explicit-costing re-run submenu, shared by the track row's context
/// menu, the status glyph's, and the recording row's: completed and failed
/// runs can re-run under any costing (costing comparisons), and a declared
/// road-less mode can be overridden (wrong declarations happen). Grayed
/// offline, since every choice reaches the server.
fn snap_costing_submenu(
    ui: &mut egui::Ui,
    target: SnapCostingTarget,
    label: &str,
    ctx: &mut PanelContext<'_>,
) {
    let label = consent_suffixed(label, ctx.snap.consent_pending);
    if ctx.snap.offline {
        ui.add_enabled(false, Button::new(label))
            .on_disabled_hover_text(OFFLINE_HOVER);
        return;
    }
    ui.menu_button(label, |ui| {
        for (costing, name) in ctx.snap.costing_choices {
            if ui.button(name).clicked() {
                *ctx.snap_costing_request = Some((target, *costing));
                ui.close();
            }
        }
    });
}

/// The submenu for one track, labeled by what its row state allows. Idle,
/// queued and in-flight rows get no submenu: they keep the plain action
/// until a run of theirs completes.
fn snap_track_costing_submenu(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    row: &SnapRowView,
    ctx: &mut PanelContext<'_>,
) {
    if let Some(label) = costing_submenu_label(row) {
        snap_costing_submenu(ui, SnapCostingTarget::Track(track_ref), label, ctx);
    }
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
            let label = consent_suffixed(verb, action.consent_pending);
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
                partial,
                warnings,
                ..
            } = row
            else {
                return;
            };
            let (snapped, interpolated, unsnapped, confidence_score, partial) = (
                *snapped,
                *interpolated,
                *unsnapped,
                *confidence_score,
                *partial,
            );
            let warnings = warnings.clone();
            ui.add_enabled(false, Button::new("Snap to road"))
                .on_disabled_hover_ui(|ui| {
                    snap_status_rows(
                        ui,
                        snapped,
                        interpolated,
                        unsnapped,
                        confidence_score,
                        partial,
                        &warnings,
                    );
                });
        }
    }
    snap_track_costing_submenu(ui, track_ref, row, ctx);

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

/// What a track's fixes say about the coordinates the receiver wrote for
/// them, when something is wrong with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoordinateWarning {
    /// This many fixes hold a latitude or a longitude outside its range. Each
    /// of them is drawn between the fixes around it.
    FixesOutOfRange(usize),
    /// No fix of the recording holds a position, so nothing places this
    /// track's fixes.
    NoValidPosition,
}

impl CoordinateWarning {
    fn for_track(track: &LoadedTrack) -> Option<Self> {
        match track.geometry {
            TrackGeometry::NoValidPosition => Some(Self::NoValidPosition),
            TrackGeometry::Measured(_) => match track.metadata.invalid_position_count {
                0 => None,
                count => Some(Self::FixesOutOfRange(count)),
            },
        }
    }

    fn hover_text(self) -> String {
        match self {
            Self::FixesOutOfRange(count) => format!(
                "{count} {} with a coordinate out of range, drawn between the fixes around {}",
                gt_fmt::pluralize(count, "fix", "fixes"),
                gt_fmt::pluralize(count, "it", "them")
            ),
            Self::NoValidPosition => {
                "No fix has a valid coordinate, so the track is not drawn on the map".to_owned()
            }
        }
    }
}

fn render_track_row(
    ui: &mut egui::Ui,
    fi: FileIdx,
    ti: TrackIdx,
    scope: MapScope<'_>,
    columns: &TreeTrackColumns,
    ctx: &mut PanelContext<'_>,
) {
    let track_ref = TrackRef::new(fi, ti);
    let Some(cells) = columns.cells.get(&track_ref) else {
        return;
    };
    let (track, passes, is_expanded, panel_hovered, map_hovered, key) = {
        let Some(file) = ctx.file(fi) else {
            return;
        };
        let Some(track) = ti.get(&file.tracks) else {
            return;
        };
        let passes = gt_filter::track_passes_filter(track, ctx.filter);
        let is_expanded = ctx.tree.track_node(track_ref).is_some_and(|t| t.expanded);
        let panel_hovered = ctx.highlight.hovers_the_whole_track(track_ref);
        // The plot writes its hover after this panel renders: a plot hover marks
        // the row one frame later.
        let map_hovered = ctx.highlight.hovers_a_point_of_track(track_ref);
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

    let is_selected = ctx.tree.selection.contains(&key);
    let cell_color = TrackRowCellColor {
        panel_hovered,
        selected: is_selected,
        passes_filter: passes,
    }
    .resolve(ui);

    let (response, newly_enabled) =
        track_columns::render_row_as_one_surface(ui, &track.metadata, cells, is_selected, |ui| {
            let chk_resp = tri_checkbox(ui, check);
            if chk_resp.clicked() {
                ctx.tree.toggle_track_check(track_ref);
            }
            let font = egui::TextStyle::Body.resolve(ui.style());
            track_columns::paint_column_cell(
                ui,
                columns.arrow_width,
                expand_arrow(is_expanded),
                &font,
                cell_color,
                egui::Align2::CENTER_CENTER,
            );
            cells.paint(ui, columns.widths, cell_color);
            if let Some(warning) = CoordinateWarning::for_track(&track) {
                ui.label(
                    RichText::new(ICON_WARNING)
                        .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                )
                .on_hover_text(warning.hover_text());
            }
            snap_control(ui, track_ref, ctx);
            chk_resp.clicked() && matches!(check, CheckState::Off | CheckState::Mixed)
        });

    if map_hovered {
        paint_map_hover_bg(ui, response.rect, map_hover_bg);
    }
    if ctx.tree.reveal_request == Some(key) {
        response.scroll_to_me(Some(egui::Align::Center));
    }
    if newly_enabled && was_all_hidden {
        *ctx.zoom_to_visible_request = true;
    }
    if response.hovered() {
        ctx.highlight.hover = Some(HighlightScope::Track(track_ref));
    }
    let modifiers = ui.ctx().input(|i| i.modifiers);
    if response.double_clicked() {
        if let Some(center) = track_bounding_center(&track) {
            *ctx.map_center_request = Some(center);
        }
    } else if response.clicked() {
        if modifiers.ctrl || modifiers.shift {
            ctx.tree.apply_click(key, modifiers.ctrl, modifiers.shift);
        } else {
            ctx.tree.toggle_expand_track(track_ref);
            ctx.tree.apply_click(key, false, false);
        }
    }
    response.context_menu(|ui| {
        show_only_track_menu_entry(ui, track_ref, ctx);
        snap_menu_entry(ui, track_ref, ctx);
        if ui.button("Show sky trails…").clicked() {
            *ctx.sky_trails_request = Some(gt_ui_types::SkyTrailsRequest::whole_track(track_ref));
            ui.close();
        }
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
            render_track_categories(ui, track_ref, &track, scope, ctx);
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
    scope: MapScope<'_>,
    ctx: &mut PanelContext<'_>,
) {
    let Some(track_node) = ctx.tree.track_node(track_ref) else {
        return;
    };
    let track_visible = track_node.track_visible;
    let tpv_visible = track_node.tpv_visible;
    let sat_visible = track_node.satellites_visible;
    let cm_visible = track_node.custom_markers_visible;
    let tpv_expanded = track_node.categories_expanded.contains(DataCategory::Tpv);
    let sat_expanded = track_node
        .categories_expanded
        .contains(DataCategory::SatelliteReport);
    let cm_expanded = track_node
        .categories_expanded
        .contains(DataCategory::CustomMarker);
    let em_expanded = track_node
        .categories_expanded
        .contains(DataCategory::EventMarker);
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
                scope,
                highlight,
                &mut PointClickRequests {
                    map_center: ctx.map_center_request,
                    popup_pos: ctx.popup_pos_request,
                },
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
                scope,
                highlight,
                &mut PointClickRequests {
                    map_center: ctx.map_center_request,
                    popup_pos: ctx.popup_pos_request,
                },
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
                scope,
                highlight,
                &mut PointClickRequests {
                    map_center: ctx.map_center_request,
                    popup_pos: ctx.popup_pos_request,
                },
            );
        },
    );

    render_generated_markers_section(ui, track_ref, track, scope, ctx);

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
    scope: MapScope<'_>,
    highlight: &mut MapHighlight,
    requests: &mut PointClickRequests<'_>,
) {
    for (pi, point) in track.points.iter().enumerate() {
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::Tpv,
            point_index: PointIdx::new(pi),
        };
        let label = point.tpv.time().utc().format("%H:%M:%S").to_string();
        let lat_lon = drawn_at(track, pi);
        point_item_row(ui, point_ref, label, lat_lon, scope, highlight, requests);
    }
}

fn render_satellite_report_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    scope: MapScope<'_>,
    highlight: &mut MapHighlight,
    requests: &mut PointClickRequests<'_>,
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
        let lat_lon = drawn_at(track, pi);
        point_item_row(ui, point_ref, label, lat_lon, scope, highlight, requests);
    }
}

/// Where the map draws the fix at `index`, in degrees.
fn drawn_at(track: &LoadedTrack, index: usize) -> Option<(f64, f64)> {
    let (latitude, longitude) = track.resolved_position_at(index)?;
    Some((latitude.as_degrees(), longitude.as_degrees()))
}

fn render_custom_marker_items(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    scope: MapScope<'_>,
    highlight: &mut MapHighlight,
    requests: &mut PointClickRequests<'_>,
) {
    for (pi, marker) in track.custom_markers.iter().enumerate() {
        let point_ref = DataPointRef {
            track: track_ref,
            category: DataCategory::CustomMarker,
            point_index: PointIdx::new(pi),
        };
        let label = format!("{}  {}", marker.time.format("%H:%M:%S"), marker.label);
        let lat_lon = Some((marker.lat.as_degrees(), marker.lon.as_degrees()));
        point_item_row(ui, point_ref, label, lat_lon, scope, highlight, requests);
    }
}

/// Render the "Generated markers" section as a tree: a category header (master
/// show/hide + expand) over one collapsible, individually toggleable group per
/// event type, with the markers of each type beneath their group.
fn render_generated_markers_section(
    ui: &mut egui::Ui,
    track_ref: TrackRef,
    track: &LoadedTrack,
    scope: MapScope<'_>,
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
        .contains(DataCategory::GeneratedMarker);

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
                    // A multi-satellite slip shows its satellite count. The
                    // others show only the time.
                    let detail = match &marker.kind {
                        GeneratedMarkerKind::Slip(event) if event.slips.len() > 1 => {
                            format!("  ({})", event.slips.len())
                        }
                        _ => String::new(),
                    };
                    let label = format!("{}{detail}", marker.time.format("%H:%M:%S"));
                    let lat_lon = Some((marker.lat.as_degrees(), marker.lon.as_degrees()));
                    point_item_row(
                        ui,
                        point_ref,
                        label,
                        lat_lon,
                        scope,
                        ctx.highlight,
                        &mut PointClickRequests {
                            map_center: ctx.map_center_request,
                            popup_pos: ctx.popup_pos_request,
                        },
                    );
                }
            });
        }
    });
}

/// A track with no geometry is drawn nowhere: there is no centre to jump to.
fn track_bounding_center(track: &LoadedTrack) -> Option<(f64, f64)> {
    let (lat, lon) = track.geometry.measured()?.bounding_box.center();
    Some((lat.as_degrees(), lon.as_degrees()))
}

fn file_bounding_center(file: Option<&LoadedFile>) -> Option<(f64, f64)> {
    let bounds = file?
        .tracks
        .iter()
        .filter_map(|t| Some(t.geometry.measured()?.bounding_box))
        .reduce(GeoBounds::union)?;
    let (lat, lon) = bounds.center();
    Some((lat.as_degrees(), lon.as_degrees()))
}

#[cfg(test)]
mod snap_action_tests {
    use rstest::rstest;

    use super::*;

    fn view(offline: bool, consent_pending: bool) -> SnapPanelView<'static> {
        // The rows map is irrelevant to snap_action. A `static` empty map keeps
        // the borrow 'static for the test helper.
        static EMPTY: std::sync::OnceLock<FxHashMap<TrackRef, SnapRowView>> =
            std::sync::OnceLock::new();
        static IDLE: std::sync::OnceLock<SnapProgressView> = std::sync::OnceLock::new();
        SnapPanelView {
            offline,
            consent_pending,
            rows: EMPTY.get_or_init(FxHashMap::default),
            costing_choices: &[],
            progress: IDLE.get_or_init(SnapProgressView::default),
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
            partial: false,
            warnings: Vec::new(),
        }
    }

    fn stale_done() -> SnapRowView {
        SnapRowView::Done {
            snapped: 1,
            interpolated: 2,
            unsnapped: 3,
            confidence_score: None,
            shown: true,
            stale: Some(vec![
                "Snapped as Bicycle - would now snap as Auto".to_owned(),
            ]),
            partial: false,
            warnings: Vec::new(),
        }
    }

    /// Pins the trigger's priority order: a current Done never has an action
    /// (its status glyph stays usable offline), a stale Done offers the
    /// re-run (grayed offline - it needs the network), Unsnappable and
    /// NothingToSend beat offline (the permanent conditions), offline grays
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
    #[case(SnapRowView::NothingToSend, false, Some(false))]
    #[case(SnapRowView::NothingToSend, true, Some(false))]
    fn action_enablement_per_state_and_offline(
        #[case] row: SnapRowView,
        #[case] offline: bool,
        #[case] expected_enabled: Option<bool>,
    ) {
        let action = snap_action(&row, view(offline, false));
        assert_eq!(action.map(|a| a.enabled), expected_enabled);
    }

    /// The disabled hover must name the reason: the declared travel mode for
    /// unsnappable tracks and the missing fixes for a track with nothing to
    /// send (both even offline - the permanent condition wins), and the
    /// offline switch for everything else.
    #[rstest]
    #[case(unsnappable(), "Boat")]
    #[case(SnapRowView::NothingToSend, "no run of real fixes")]
    #[case(SnapRowView::Idle, "offline mode")]
    #[case(failed(), "offline mode")]
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

#[cfg(test)]
mod coordinate_warning_tests {
    use gt_types::TrackGeometry;
    use rstest::rstest;

    use super::CoordinateWarning;

    #[rstest]
    #[case::one_fix(
        CoordinateWarning::FixesOutOfRange(1),
        "1 fix with a coordinate out of range, drawn between the fixes around it"
    )]
    #[case::several_fixes(
        CoordinateWarning::FixesOutOfRange(2),
        "2 fixes with a coordinate out of range, drawn between the fixes around them"
    )]
    #[case::no_valid_position(
        CoordinateWarning::NoValidPosition,
        "No fix has a valid coordinate, so the track is not drawn on the map"
    )]
    fn a_coordinate_warning_states_what_is_wrong_with_the_track(
        #[case] warning: CoordinateWarning,
        #[case] expected: &str,
    ) {
        assert_eq!(warning.hover_text(), expected);
    }

    #[test]
    fn a_track_whose_fixes_all_hold_a_coordinate_in_range_raises_no_warning() {
        let track = gt_test_utils::loaded_track_with_points(gt_test_utils::nav_test_data());

        assert_eq!(CoordinateWarning::for_track(&track), None);
    }

    #[test]
    fn a_track_with_fixes_out_of_range_counts_them() {
        let mut track = gt_test_utils::loaded_track_with_points(gt_test_utils::nav_test_data());
        track.metadata.invalid_position_count = 2;

        assert_eq!(
            CoordinateWarning::for_track(&track),
            Some(CoordinateWarning::FixesOutOfRange(2))
        );
    }

    #[test]
    fn a_track_without_a_geometry_reports_no_valid_position() {
        let mut track = gt_test_utils::loaded_track_with_points(
            gt_test_utils::nav_points_without_a_valid_position(3),
        );
        track.metadata.invalid_position_count = 3;

        assert_eq!(track.geometry, TrackGeometry::NoValidPosition);
        assert_eq!(
            CoordinateWarning::for_track(&track),
            Some(CoordinateWarning::NoValidPosition)
        );
    }
}
