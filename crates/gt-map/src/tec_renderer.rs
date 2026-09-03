//! The ionospheric TEC overlay: one filled rectangle per grid node of the
//! archived maps, drawn beneath every track renderer.
//!
//! The published grid is 2.5 degrees of latitude by 5 of longitude, so a node
//! covers a few hundred kilometres. Each node draws as its own rectangle at the
//! value the producer published there, interpolated between the two maps
//! bracketing the shown instant. Nothing is interpolated across the grid: a
//! smooth field would draw structure the product does not carry.

use chrono::{DateTime, Utc};
use egui::{Color32, Mesh, Pos2, Rect, Shape, Ui};
use gt_ionex::grid::{GridPoint, MapGrid};
use gt_ionex::instant_selection::{TecEmptyReason, TecInstantSelection};
use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::tec::TotalElectronContent;
use gt_types::mercator::MercPoint;
use gt_types::{Latitude, Longitude, mercator};
use gt_ui_theme::{tec_color, tec_fill_alpha};
use walkers::{MapMemory, Plugin, Projector};

use crate::hover_labels::TOOLTIP_POINTER_GAP_PX;
use crate::transform::MercTransform;

/// Latitude of the poles, which a node's half step reaches past: the
/// published grid's outermost nodes sit at 87.5 degrees.
const POLE_LATITUDE_DEGREES: f64 = 90.0;

/// The maps the heatmap draws from and the instant it draws them at.
#[derive(Debug, Clone, Copy)]
pub struct TecHeatmapSnapshot<'a> {
    pub maps: &'a GlobalIonosphereMaps,
    pub instant: DateTime<Utc>,
}

impl TecHeatmapSnapshot<'_> {
    /// Nodes the archived grid holds, which the display toggle counts.
    pub fn node_count(&self) -> usize {
        let grid = self.maps.grid();
        grid.latitudes
            .node_count()
            .saturating_mul(grid.longitudes.node_count())
    }
}

/// The heatmap's per-frame inputs: what it draws, where its instant stepper
/// stands, and why it has nothing to draw.
pub struct TecLayer<'a> {
    /// The archived maps and the instant to draw them at. [`None`] when no day
    /// covering the instant is archived.
    pub snapshot: Option<TecHeatmapSnapshot<'a>>,
    /// The instant the display toggle's stepper moves, which a hovered or
    /// selected fix overrides.
    pub instant: &'a mut TecInstantSelection,
    pub empty_reason: Option<TecEmptyReason>,
}

/// One grid node projected to screen space, ready to paint and to hit-test.
pub(crate) struct NodeCell {
    pub(crate) rect: Rect,
    pub(crate) fill: Color32,
    /// The value behind the fill, for the hover.
    pub(crate) content: TotalElectronContent,
    pub(crate) latitude: Latitude,
    pub(crate) longitude: Longitude,
}

/// A declared longitude as the projection takes it. A grid declared from 0 to
/// 360 degrees names its western meridians above 180, while a value already
/// inside the range is left alone so the 180 degree node stays at the eastern
/// edge of the world.
fn projected_longitude_degrees(declared: f64) -> f64 {
    if (-180.0..=180.0).contains(&declared) {
        declared
    } else {
        mercator::wrap_longitude_degrees(declared)
    }
}

/// The area one node covers: half a grid step around it in each direction,
/// held inside the world. `mercator::normalize` brings an area reaching past
/// the projection's latitude limit back to the edge of the map.
fn node_area(grid: MapGrid, point: GridPoint) -> Option<(MercPoint, MercPoint)> {
    let latitude_degrees = grid.latitudes.degrees_at(point.latitude_index)?;
    let longitude_degrees =
        projected_longitude_degrees(grid.longitudes.degrees_at(point.longitude_index)?);
    let half_latitude_step = grid.latitudes.axis().step_degrees().abs() / 2.0;
    let half_longitude_step = grid.longitudes.axis().step_degrees().abs() / 2.0;

    let latitude =
        |degrees: f64| Latitude::new(degrees.clamp(-POLE_LATITUDE_DEGREES, POLE_LATITUDE_DEGREES));
    let longitude = |degrees: f64| Longitude::new(degrees.clamp(-180.0, 180.0));

    Some((
        mercator::normalize(
            latitude(latitude_degrees + half_latitude_step),
            longitude(longitude_degrees - half_longitude_step),
        ),
        mercator::normalize(
            latitude(latitude_degrees - half_latitude_step),
            longitude(longitude_degrees + half_longitude_step),
        ),
    ))
}

/// The nodes of `snapshot` that fall inside `rect`, projected and coloured.
///
/// A node the producer left unpublished, or one whose value the shown instant
/// falls outside the archived epochs of, draws nothing.
pub(crate) fn visible_cells(
    snapshot: &TecHeatmapSnapshot<'_>,
    transform: &MercTransform,
    rect: Rect,
    dark_mode: bool,
    opacity_percent: f32,
) -> Vec<NodeCell> {
    let grid = snapshot.maps.grid();
    let alpha = tec_fill_alpha(opacity_percent);
    let mut cells = Vec::new();
    for latitude_index in 0..grid.latitudes.node_count() {
        for longitude_index in 0..grid.longitudes.node_count() {
            let point = GridPoint {
                latitude_index,
                longitude_index,
            };
            let Some(content) = snapshot.maps.node_value_at(point, snapshot.instant) else {
                continue;
            };
            let Some((north_west, south_east)) = node_area(grid, point) else {
                continue;
            };
            let cell_rect = Rect::from_two_pos(
                transform.to_screen(north_west),
                transform.to_screen(south_east),
            );
            if !cell_rect.intersects(rect) {
                continue;
            }
            let (Some(latitude_degrees), Some(longitude_degrees)) = (
                grid.latitudes.degrees_at(latitude_index),
                grid.longitudes.degrees_at(longitude_index),
            ) else {
                continue;
            };
            cells.push(NodeCell {
                rect: cell_rect,
                fill: tec_color(content.tecu())
                    .resolve(dark_mode)
                    .gamma_multiply_u8(alpha),
                content,
                latitude: Latitude::new(latitude_degrees),
                longitude: Longitude::new(projected_longitude_degrees(longitude_degrees)),
            });
        }
    }
    cells
}

/// The overlay plugin. Registered before every other renderer, so the grid
/// sits beneath both the interference cells and the track ink.
pub(crate) struct TecHeatmapRenderer<'a> {
    snapshot: TecHeatmapSnapshot<'a>,
    opacity_percent: f32,
    hover_enabled: bool,
}

impl<'a> TecHeatmapRenderer<'a> {
    pub(crate) const fn new(
        snapshot: TecHeatmapSnapshot<'a>,
        opacity_percent: f32,
        hover_enabled: bool,
    ) -> Self {
        Self {
            snapshot,
            opacity_percent,
            hover_enabled,
        }
    }
}

impl Plugin for TecHeatmapRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        response: &egui::Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform = MercTransform::new(projector, map_memory, ui.max_rect().center());
        let cells = visible_cells(
            &self.snapshot,
            &transform,
            ui.max_rect(),
            ui.visuals().dark_mode,
            self.opacity_percent,
        );
        draw_cells(ui, &cells);

        if !self.hover_enabled {
            return;
        }
        let Some(pointer) = response.hovered().then(|| response.hover_pos()).flatten() else {
            return;
        };
        if let Some(cell) = cell_at_pointer(&cells, pointer) {
            // Anchored at the pointer: `response` is the whole map, so a
            // response-anchored tooltip lands in the map's corner.
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                response.id,
                egui::PopupAnchor::Pointer,
            )
            .gap(TOOLTIP_POINTER_GAP_PX)
            .show(|ui| node_tooltip(ui, cell, self.snapshot.instant));
        }
    }
}

/// Paint the overlay beneath every other renderer, as one mesh so a whole
/// grid costs a single shape.
fn draw_cells(ui: &Ui, cells: &[NodeCell]) {
    let mut mesh = Mesh::default();
    for cell in cells {
        mesh.add_colored_rect(cell.rect, cell.fill);
    }
    ui.painter().add(Shape::mesh(mesh));
}

/// The node under the pointer, if any. Nodes do not overlap, so the first
/// containing one is returned.
fn cell_at_pointer(cells: &[NodeCell], pointer: Pos2) -> Option<&NodeCell> {
    cells.iter().find(|cell| cell.rect.contains(pointer))
}

/// The node hover shows the value behind the fill. The display toggle's own
/// hover describes the layer.
fn node_tooltip(ui: &mut Ui, cell: &NodeCell, instant: DateTime<Utc>) {
    for line in gt_ionex::text::node_summary(cell.content, instant, cell.latitude, cell.longitude) {
        ui.label(line);
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeDelta};
    use gt_ionex::grid::{AxisDeclaration, GridAxis, LatitudeAxis, LongitudeAxis};
    use gt_ionex::maps::TecMap;
    use rstest::rstest;

    use super::*;

    /// Side of the test viewport, in pixels.
    const CANVAS_PX: f32 = 400.0;

    /// World width in pixels for a view that frames the whole grid.
    const WORLD_VIEW_TOTAL_PX: f64 = 512.0;

    fn epoch(hour: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(2024, 5, 10)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .unwrap()
            .and_utc()
    }

    fn axis(first: f64, last: f64, step: f64) -> GridAxis {
        GridAxis::new(AxisDeclaration {
            first_degrees: first,
            last_degrees: last,
            step_degrees: step,
        })
        .unwrap()
    }

    /// The axes JPL publishes global maps on.
    fn jpl_grid() -> MapGrid {
        MapGrid {
            latitudes: LatitudeAxis::new(axis(87.5, -87.5, -2.5)),
            longitudes: LongitudeAxis::new(axis(-180.0, 180.0, 5.0)),
            shell_height_km: 450.0,
        }
    }

    /// Two maps two hours apart on `grid`, every node valued by `tecu` of its
    /// node indices, except the northwestern node of the first map, which the
    /// producer left unpublished.
    fn maps_on(grid: MapGrid, tecu: impl Fn(usize, usize) -> f64) -> GlobalIonosphereMaps {
        let map_at = |hour: u32, gap: bool| {
            let bands = (0..grid.latitudes.node_count())
                .map(|latitude_index| {
                    (0..grid.longitudes.node_count())
                        .map(|longitude_index| {
                            (!(gap && latitude_index == 0 && longitude_index == 0)).then(|| {
                                TotalElectronContent::from_tecu(tecu(
                                    latitude_index,
                                    longitude_index,
                                ))
                            })
                        })
                        .collect()
                })
                .collect();
            TecMap::new(epoch(hour), bands)
        };
        GlobalIonosphereMaps::new(
            grid,
            TimeDelta::hours(2),
            vec![map_at(0, true), map_at(2, false)],
        )
    }

    fn world_view() -> (MercTransform, Rect) {
        let rect = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(WORLD_VIEW_TOTAL_PX as f32, WORLD_VIEW_TOTAL_PX as f32),
        );
        let transform = MercTransform::for_test_view(
            WORLD_VIEW_TOTAL_PX,
            Latitude::new(0.0),
            Longitude::new(0.0),
            rect.center(),
        );
        (transform, rect)
    }

    /// Whether two degree or TEC-unit values agree within the rounding of the
    /// projection round trip.
    fn nearly_equal(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    fn snapshot_of(maps: &GlobalIonosphereMaps, hour: u32) -> TecHeatmapSnapshot<'_> {
        TecHeatmapSnapshot {
            maps,
            instant: epoch(hour),
        }
    }

    /// Every node of the grid is drawn when the whole world is in view, minus
    /// the one the producer left unpublished.
    #[test]
    fn a_world_view_draws_every_published_node() {
        let maps = maps_on(jpl_grid(), |latitude, longitude| {
            (latitude + longitude) as f64
        });
        let (transform, rect) = world_view();
        let snapshot = snapshot_of(&maps, 0);

        let cells = visible_cells(&snapshot, &transform, rect, true, 100.0);
        assert_eq!(snapshot.node_count(), 71 * 73);
        assert_eq!(
            cells.len(),
            snapshot.node_count() - 1,
            "the unpublished node draws nothing"
        );
    }

    /// A node whose value the shown instant falls outside the archived epochs
    /// of draws nothing at all.
    #[test]
    fn an_instant_outside_the_archived_epochs_draws_nothing() {
        let maps = maps_on(jpl_grid(), |_, _| 20.0);
        let (transform, rect) = world_view();
        let snapshot = TecHeatmapSnapshot {
            maps: &maps,
            instant: epoch(0) - TimeDelta::hours(1),
        };

        assert!(visible_cells(&snapshot, &transform, rect, true, 100.0).is_empty());
    }

    /// A view tight around one node keeps that node and drops the rest.
    #[test]
    fn only_the_nodes_in_the_window_are_drawn() {
        let maps = maps_on(jpl_grid(), |_, _| 20.0);
        let rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        // A world 4 million pixels wide puts one 5-degree node well past the
        // canvas, so only the node under the centre and its neighbours reach it.
        let transform = MercTransform::for_test_view(
            4_000_000.0,
            Latitude::new(55.0),
            Longitude::new(12.0),
            rect.center(),
        );
        let snapshot = snapshot_of(&maps, 0);

        let cells = visible_cells(&snapshot, &transform, rect, true, 100.0);
        assert_eq!(cells.len(), 1, "one node covers the whole canvas");
    }

    /// The node under the pointer is the one whose area contains it, and the
    /// value reported is that node's own.
    #[test]
    fn the_pointer_picks_the_node_it_is_over() {
        let maps = maps_on(jpl_grid(), |latitude, longitude| {
            (latitude * 100 + longitude) as f64
        });
        let (transform, rect) = world_view();
        let snapshot = snapshot_of(&maps, 2);
        let cells = visible_cells(&snapshot, &transform, rect, true, 100.0);

        let hit = cell_at_pointer(&cells, rect.center()).expect("a node under the centre");
        assert!(nearly_equal(hit.latitude.as_degrees(), 0.0));
        assert!(nearly_equal(hit.longitude.as_degrees(), 0.0));
        // Node 35 of the latitude axis and 36 of the longitude axis.
        assert!(
            nearly_equal(hit.content.tecu(), 3536.0),
            "{} TECU is not the centre node's value",
            hit.content.tecu()
        );
        assert!(cell_at_pointer(&cells, egui::pos2(-9000.0, -9000.0)).is_none());
    }

    /// A node's rectangle covers half a grid step around it, so neighbouring
    /// nodes tile without a gap between them.
    #[test]
    fn neighbouring_nodes_share_an_edge() {
        let maps = maps_on(jpl_grid(), |_, _| 20.0);
        let (transform, rect) = world_view();
        let cells = visible_cells(&snapshot_of(&maps, 0), &transform, rect, true, 100.0);

        let equator = cells
            .iter()
            .filter(|cell| nearly_equal(cell.latitude.as_degrees(), 0.0))
            .collect::<Vec<_>>();
        for pair in equator.windows(2) {
            let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            assert!(
                (left.rect.right() - right.rect.left()).abs() < 1e-3,
                "{:?} and {:?} do not share an edge",
                left.rect,
                right.rect
            );
        }
    }

    /// The outermost declared nodes reach past the Mercator projection's
    /// latitude cut-off, and their areas stop there.
    #[test]
    fn the_polar_nodes_stop_at_the_projections_limit() {
        let grid = jpl_grid();
        let (north_west, _) = node_area(
            grid,
            GridPoint {
                latitude_index: 0,
                longitude_index: 0,
            },
        )
        .expect("the northernmost node");
        let limit = mercator::normalize(
            Latitude::new(mercator::MAX_LATITUDE_DEGREES),
            Longitude::new(-180.0),
        );
        assert!(north_west.y.is_finite());
        assert!(nearly_equal(north_west.y, limit.y));
    }

    /// A grid declared in the 0 to 360 convention draws at the same meridians
    /// as one declared from -180.
    #[test]
    fn an_eastward_grid_draws_where_its_nodes_are() {
        let grid = MapGrid {
            latitudes: LatitudeAxis::new(axis(87.5, -87.5, -2.5)),
            longitudes: LongitudeAxis::new(axis(0.0, 355.0, 5.0)),
            shell_height_km: 450.0,
        };
        let maps = maps_on(grid, |_, longitude| longitude as f64);
        let (transform, rect) = world_view();
        let cells = visible_cells(&snapshot_of(&maps, 2), &transform, rect, true, 100.0);

        let western = cells
            .iter()
            .find(|cell| nearly_equal(cell.longitude.as_degrees(), -5.0))
            .expect("the node one step west of the prime meridian");
        // Declared at 355 degrees east, the last node of the axis.
        assert!(
            nearly_equal(western.content.tecu(), 71.0),
            "{} TECU is not the last node's value",
            western.content.tecu()
        );
    }

    /// The opacity control scales the fill and nothing else.
    #[rstest]
    #[case::transparent(0.0, 0)]
    #[case::half(50.0, 128)]
    #[case::opaque(100.0, 255)]
    fn the_opacity_percentage_scales_the_fill(#[case] percent: f32, #[case] expected_alpha: u8) {
        let maps = maps_on(jpl_grid(), |_, _| 20.0);
        let (transform, rect) = world_view();
        let cells = visible_cells(&snapshot_of(&maps, 0), &transform, rect, true, percent);

        let fill = cells.first().expect("a node").fill;
        assert_eq!(fill.a(), expected_alpha);
    }
}
