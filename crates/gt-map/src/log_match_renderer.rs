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

use egui::{Align2, Color32, FontId, Response, Ui, Vec2};
use gt_ui_types::{LogMatchColor, LogMatchLayer, LogMatches};
use walkers::{MapMemory, Plugin, Projector};

use crate::collision_grid;
use crate::icon_mesh::{IconId, IconInstance, IconMeshBatch, IconMeshLibrary};
use crate::transform::MercTransform;

/// Circumradius of one match's hexagon, comparable to the fix dot it sits on.
const GLYPH_CIRCUMRADIUS_PX: f32 = 8.0;

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

/// Draws what the loaded logs' filters selected, above the track line and
/// below the markers.
pub(crate) struct LogMatchRenderer<'a> {
    matches: &'a LogMatches,
    icon_meshes: Option<&'a IconMeshLibrary>,
    dark_mode: bool,
}

impl<'a> LogMatchRenderer<'a> {
    pub(crate) fn new(
        matches: &'a LogMatches,
        icon_meshes: Option<&'a IconMeshLibrary>,
        dark_mode: bool,
    ) -> Self {
        Self {
            matches,
            icon_meshes,
            dark_mode,
        }
    }
}

impl Plugin for LogMatchRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform = MercTransform::new(projector, map_memory, ui.max_rect().center());
        let viewport = transform.viewport_merc_bounds(ui.max_rect());
        let cluster_spacing_merc =
            collision_grid::decimation_cell_merc(CLUSTER_SPACING_PX, map_memory.zoom());
        let outline = gt_ui_theme::LOG_HEXAGON_OUTLINE;

        let mut batch = IconMeshBatch::gpu_when_available(ui, self.icon_meshes);
        let mut counted_clusters: Vec<(egui::Pos2, usize)> = Vec::new();
        for layer in self.matches.layers() {
            let fill = layer_color(layer, self.dark_mode);
            let shared = matches!(layer.color, LogMatchColor::LayerSlot { shared: true, .. });
            counted_clusters.clear();
            for cluster in
                collision_grid::cluster_positions(&layer.positions, cluster_spacing_merc, viewport)
            {
                let center = transform.to_screen(cluster.merc);
                let counted = cluster.count >= CLUSTER_COUNT_FROM;
                let circumradius = if counted {
                    CLUSTER_CIRCUMRADIUS_PX
                } else {
                    GLYPH_CIRCUMRADIUS_PX
                };
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
                    counted_clusters.push((center, cluster.count));
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
    }
}

/// The colour this layer's hexagons are filled with.
fn layer_color(layer: &LogMatchLayer, dark_mode: bool) -> Color32 {
    match layer.color {
        LogMatchColor::LiveFilter => gt_ui_theme::LOG_LIVE_FILTER.resolve(dark_mode),
        LogMatchColor::LayerSlot { index, .. } => {
            gt_ui_theme::log_layer_slot_color(index).resolve(dark_mode)
        }
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
