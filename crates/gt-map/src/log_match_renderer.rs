//! The map's log matches: one hexagon per line a log's filters selected, at
//! the position the line was recorded at.
//!
//! The hexagon is the map's shape for a log event, and reads as nothing else:
//! halos, discs and rings belong to sky glyphs and query matches, and pins to
//! markers. Matches closer together than a glyph and a half collapse into one
//! hexagon, which carries a count once it stands for enough of them: zooming
//! in dissolves the clusters.
//!
//! Layers draw one after another, each layer's counts with its own hexagons,
//! so a hexagon of the layer above covers a glyph and its count together. Two
//! layers do overlap where they matched the same place: they are two filters,
//! and each keeps its own colour.
//!
//! The cursor picks the hexagon of the topmost layer it is on. That hexagon
//! lists its lines in a tooltip and takes the highlight ring, and the log
//! viewer marks the rows of those same lines. Clicking it opens the log viewer
//! on that log, at the first of those lines.
//!
//! The global filter applies here as it does to the recorded track. Nothing
//! draws for a match whose entry falls outside the time window, or whose fix
//! sits on a track the filter rejects: such a match is left out of every
//! cluster's count and takes no pointer.

use std::cell::RefCell;
use std::num::NonZeroUsize;

use egui::{Align2, Color32, FontId, Response, RichText, Ui, Vec2};
use gt_filter::GlobalFilter;
use gt_fmt::ELLIPSIS;
use gt_types::{LoadedFile, MercPoint};
use gt_ui_types::{LogMatch, LogMatchColor, LogMatchGlyph, LogMatchSource, LogMatches};
use walkers::{MapMemory, Plugin, Projector};

use crate::collision_grid;
use crate::hover_labels::TOOLTIP_POINTER_GAP_PX;
use crate::icon_mesh::{IconId, IconInstance, IconMeshBatch, IconMeshLibrary};
use crate::transform::MercTransform;

/// Circumradius of one match's hexagon, comparable to the fix dot it sits on.
const GLYPH_CIRCUMRADIUS_PX: f32 = 8.0;

/// Gap between a glyph's circumradius and the cross-highlight ring around it,
/// so the ring encloses the glyph rather than tracing it.
const HOVER_RING_GAP_PX: f32 = 5.0;

/// Stroke width of the cross-highlight ring.
const HOVER_RING_STROKE_WIDTH_PX: f32 = 3.0;

/// Circumradius of a cluster's hexagon, which carries a count.
const CLUSTER_CIRCUMRADIUS_PX: f32 = 11.0;

/// The share of an instance's half extent the hexagon asset's circumradius
/// takes, the rest being the room its outline needs (see `hexagon.svg`).
const ASSET_CIRCUMRADIUS_FRACTION: f32 = 0.925;

/// Screen distance under which two matches of one filter collapse into a
/// single hexagon: a glyph and a half across. Collapsing leaves the surviving
/// hexagons of a filter at least this far apart, so they never cover each
/// other's count.
const CLUSTER_SPACING_PX: f32 = 3.0 * GLYPH_CIRCUMRADIUS_PX;

/// Matches a cluster must stand for before it draws larger and states its
/// count. Below it the cluster is a plain glyph: a count of "2" says less than
/// the hexagon already does.
const CLUSTER_COUNT_FROM: usize = 5;

/// Height of the count inside a cluster's hexagon.
const CLUSTER_COUNT_FONT_PX: f32 = 11.0;

/// How far outside the glyph the ring around a shared colour sits. Two filters
/// handed the same colour still read as two: one of them is ringed.
const SHARED_COLOR_RING_SCALE: f32 = 1.35;

/// Lines the hover tooltip writes out before it states how many are left.
const HOVER_LINE_CAP: usize = 5;

/// Characters of a message the hover tooltip shows: one long line must not
/// stretch the tooltip across the map.
const HOVER_MESSAGE_MAX_CHARS: NonZeroUsize = match NonZeroUsize::new(64) {
    Some(chars) => chars,
    None => NonZeroUsize::MIN,
};

/// How the hover tooltip writes a line's moment, as the map's other hover
/// labels write a fix's.
const HOVER_TIME_FORMAT: &str = "%H:%M:%S";

/// Draws what the loaded logs' filters selected, above the track line and
/// below the markers.
#[derive(bon::Builder)]
pub(crate) struct LogMatchRenderer<'a> {
    matches: &'a LogMatches,

    /// The loaded files, resolving a match's fix to the track it was recorded
    /// on.
    files: &'a [LoadedFile],
    filter: &'a GlobalFilter,
    icon_meshes: Option<&'a IconMeshLibrary>,
    dark_mode: bool,

    /// Whether a hexagon may take the pointer this frame: a marker above the
    /// layer owns it first.
    hover_enabled: bool,

    /// Where the log viewer's hovered row was recorded, which the map rings.
    hovered_row_position: Option<MercPoint>,

    /// Where the hexagon under the cursor is published for the viewer.
    hovered_glyph: &'a RefCell<Option<LogMatchGlyph>>,

    /// Where a click on that hexagon is published for the viewer, which opens
    /// on its log.
    clicked_glyph: &'a RefCell<Option<LogMatchGlyph>>,
}

/// The hexagon the cursor is on, once every layer has drawn.
struct HoveredHexagon<'a> {
    center: egui::Pos2,
    circumradius: f32,
    source: &'a LogMatchSource,
    glyph: LogMatchGlyph,
}

impl Plugin for LogMatchRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform = MercTransform::new(projector, map_memory, ui.max_rect().center());
        let viewport = transform.viewport_merc_bounds(ui.max_rect());
        let cluster_spacing_merc =
            collision_grid::decimation_cell_merc(CLUSTER_SPACING_PX, map_memory.zoom());
        let outline = gt_ui_theme::LOG_HEXAGON_OUTLINE;
        let pointer = (self.hover_enabled && response.hovered())
            .then(|| response.hover_pos())
            .flatten();

        let mut batch = IconMeshBatch::gpu_when_available(ui, self.icon_meshes);
        let mut counted_clusters: Vec<(egui::Pos2, usize)> = Vec::new();
        // Reused across layers: one allocation per frame.
        let mut drawn_matches: Vec<&LogMatch> = Vec::new();
        let mut hovered: Option<HoveredHexagon<'_>> = None;
        for layer in self.matches.layers() {
            let fill = gt_ui_theme::log_match_color(layer.color, self.dark_mode);
            let shared = matches!(layer.color, LogMatchColor::LayerSlot { shared: true, .. });
            counted_clusters.clear();
            drawn_matches.clear();
            drawn_matches.extend(layer.matches_passing_filter(self.files, self.filter));
            for cluster in collision_grid::cluster_positions(
                drawn_matches.iter().map(|entry| entry.merc),
                cluster_spacing_merc,
                viewport,
            ) {
                let center = transform.to_screen(cluster.merc);
                let counted = cluster.members.len() >= CLUSTER_COUNT_FROM;
                let circumradius = if counted {
                    CLUSTER_CIRCUMRADIUS_PX
                } else {
                    GLYPH_CIRCUMRADIUS_PX
                };
                // The last hit is the hexagon the cursor is on: a later layer
                // draws over an earlier one.
                if pointer.is_some_and(|pos| (pos - center).length() <= circumradius) {
                    hovered = Some(HoveredHexagon {
                        center,
                        circumradius,
                        source: &layer.log,
                        glyph: LogMatchGlyph {
                            log: layer.log.id,
                            color: layer.color,
                            // The entries come out ascending: a layer's
                            // matches are in file order.
                            entry_indices: cluster
                                .members
                                .iter()
                                .filter_map(|&member| Some(drawn_matches.get(member)?.entry_index))
                                .collect(),
                        },
                    });
                }
                if shared {
                    batch.push(hexagon(
                        center,
                        circumradius * SHARED_COLOR_RING_SCALE,
                        HexagonTints {
                            fill: Color32::TRANSPARENT,
                            outline: fill,
                        },
                    ));
                }
                batch.push(hexagon(
                    center,
                    circumradius,
                    HexagonTints { fill, outline },
                ));
                if counted {
                    counted_clusters.push((center, cluster.members.len()));
                }
            }
            // The barrier flushes this layer's hexagons, so its counts draw
            // over them and under the next layer's.
            batch.barrier(ui.painter());
            for &(center, count) in &counted_clusters {
                ui.painter().text(
                    center,
                    Align2::CENTER_CENTER,
                    count,
                    FontId::monospace(CLUSTER_COUNT_FONT_PX),
                    outline,
                );
            }
        }
        batch.paint(ui.painter());

        // The ring the app draws around a cross-highlighted element, here at
        // the position of the viewer's hovered row and around the hexagon
        // under the cursor. Log hover rings are the live-filter gold, keeping
        // the log layer's hover language in its reserved colour.
        let hover_ring = gt_ui_theme::LOG_LIVE_FILTER.resolve(self.dark_mode);
        if let Some(merc) = self.hovered_row_position {
            draw_hover_ring(
                ui,
                transform.to_screen(merc),
                GLYPH_CIRCUMRADIUS_PX,
                hover_ring,
            );
        }
        if let Some(hexagon) = hovered {
            draw_hover_ring(ui, hexagon.center, hexagon.circumradius, hover_ring);
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                response.id,
                egui::PopupAnchor::Pointer,
            )
            .gap(TOOLTIP_POINTER_GAP_PX)
            .show(|ui| hovered_lines_ui(ui, hexagon.source, &hexagon.glyph.entry_indices));
            if response.clicked() {
                *self.clicked_glyph.borrow_mut() = Some(hexagon.glyph.clone());
            }
            *self.hovered_glyph.borrow_mut() = Some(hexagon.glyph);
        }
    }
}

/// The cross-highlight ring around a glyph the viewer or the cursor points at.
fn draw_hover_ring(ui: &Ui, center: egui::Pos2, circumradius: f32, color: Color32) {
    ui.painter().circle_stroke(
        center,
        circumradius + HOVER_RING_GAP_PX,
        egui::Stroke::new(HOVER_RING_STROKE_WIDTH_PX, color),
    );
}

/// The hovered hexagon's lines, under the log they were read out of, the last
/// row stating how many of them the tooltip left out.
fn hovered_lines_ui(ui: &mut Ui, source: &LogMatchSource, entry_indices: &[usize]) {
    // One line per row: a message is already cut to a width the tooltip fits.
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    if let Some(display_name) = &source.display_name {
        ui.label(RichText::new(display_name).strong());
    }
    let parsed = &source.parsed;
    for entry in entry_indices
        .iter()
        .take(HOVER_LINE_CAP)
        .filter_map(|&entry_index| parsed.entries().get(entry_index))
    {
        ui.label(
            RichText::new(format!(
                "{}  {}",
                entry.timestamp.format(HOVER_TIME_FORMAT),
                gt_fmt::truncate_with_ellipsis(parsed.message(entry), HOVER_MESSAGE_MAX_CHARS)
            ))
            .monospace(),
        );
    }
    let left_out = entry_indices.len().saturating_sub(HOVER_LINE_CAP);
    if left_out > 0 {
        ui.label(
            RichText::new(format!(
                "{ELLIPSIS}and {left_out} more {}",
                gt_fmt::pluralize(left_out, "line", "lines")
            ))
            .weak(),
        );
    }
}

/// What a hexagon is painted with: the filter's colour inside, and the tone
/// separating it from the track line and the tiles.
#[derive(Clone, Copy)]
struct HexagonTints {
    fill: Color32,
    outline: Color32,
}

/// One hexagon of `circumradius`.
fn hexagon(center: egui::Pos2, circumradius: f32, tints: HexagonTints) -> IconInstance {
    IconInstance {
        icon: IconId::Hexagon,
        center,
        half_extents: Vec2::splat(circumradius / ASSET_CIRCUMRADIUS_FRACTION),
        direction: None,
        tints: [tints.fill, tints.outline],
    }
}
