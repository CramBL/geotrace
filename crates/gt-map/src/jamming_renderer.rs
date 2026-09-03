//! The aircraft-interference overlay: one filled polygon per H3 cell,
//! drawn beneath every track renderer.
//!
//! Cells whose aircraft count is too small for their share to mean anything
//! draw hatched, so they stay visible and stay distinguishable from a
//! measured value.

use std::ops::RangeInclusive;

use egui::{Color32, Pos2, Shape, Stroke, Ui};
use gt_jam::dataset::JamDataset;
use gt_jam::wire::HexObservation;
use gt_types::mercator::MercPoint;
use gt_types::{Latitude, Longitude, mercator};
use gt_ui_theme::{INTERFERENCE_FILL_ALPHA, interference_color};
use h3o::{CellIndex, LatLng};

use walkers::{MapMemory, Plugin, Projector};

use crate::hover_labels::TOOLTIP_POINTER_GAP_PX;
use crate::transform::MercTransform;

/// Aircraft below which a cell's share carries no weight and the cell draws
/// hatched. gpsjam publishes cells with as few as two aircraft, where one
/// bad report reads as 50 %.
pub const MIN_AIRCRAFT_FOR_SOLID_FILL: u32 = 5;

/// Span of the whole world in normalised Mercator x, which is what a
/// longitude past the antimeridian wraps by.
const WORLD_WIDTH: f64 = 1.0;

const HALF_WORLD_WIDTH: f64 = WORLD_WIDTH / 2.0;

/// Spacing between hatch lines, in pixels.
const HATCH_SPACING_PX: f32 = 6.0;

/// Width of a hatch line.
const HATCH_STROKE_WIDTH: f32 = 1.0;

/// A cell projected to screen space, ready to paint and to hit-test.
pub(crate) struct CellShape {
    pub(crate) outline: Vec<Pos2>,
    pub(crate) fill: Color32,
    /// Whether the cell's aircraft count is too small for a solid fill.
    pub(crate) low_sample: bool,
    /// The tally behind the fill, for the hover.
    pub(crate) observation: HexObservation,
}

/// Whether `observation` has enough aircraft for its share to be drawn as a
/// measured value.
pub(crate) const fn is_low_sample(observation: &HexObservation) -> bool {
    observation.aircraft() < MIN_AIRCRAFT_FOR_SOLID_FILL
}

/// Project one cell's boundary to screen space.
///
/// [`None`] when the boundary has too few vertices to form a polygon, which
/// no published cell does.
fn cell_outline(cell: CellIndex, transform: &MercTransform) -> Option<Vec<Pos2>> {
    let boundary = cell.boundary();
    if boundary.len() < 3 {
        return None;
    }
    let center = LatLng::from(cell);
    let center_x = mercator::normalize(Latitude::new(center.lat()), Longitude::new(center.lng())).x;
    Some(
        boundary
            .iter()
            .map(|vertex| {
                let mut merc =
                    mercator::normalize(Latitude::new(vertex.lat()), Longitude::new(vertex.lng()));
                // A cell straddling the antimeridian has vertices normalising
                // to both edges of the world. Each one is moved onto the turn
                // of the world its cell centre is on, which keeps the polygon
                // one cell wide.
                if merc.x - center_x > HALF_WORLD_WIDTH {
                    merc.x -= WORLD_WIDTH;
                } else if center_x - merc.x > HALF_WORLD_WIDTH {
                    merc.x += WORLD_WIDTH;
                }
                transform.to_screen(merc)
            })
            .collect(),
    )
}

/// The cells of `dataset` that fall inside `rect`, projected and coloured.
///
/// Culling is [`JamDataset::observations_within`]'s, which pads the window
/// by one cell radius so a cell reaching into the viewport is kept.
pub(crate) fn visible_cells(
    dataset: &JamDataset,
    transform: &MercTransform,
    rect: egui::Rect,
    dark_mode: bool,
) -> Vec<CellShape> {
    let bounds = transform.viewport_merc_bounds(rect);
    let (lat, lon) = geographic_window(bounds);

    dataset
        .observations_within(lat, lon)
        .filter_map(|observation| {
            let rate = observation.rate()?;
            let outline = cell_outline(observation.cell, transform)?;
            Some(CellShape {
                outline,
                fill: interference_color(rate.bad_fraction)
                    .resolve(dark_mode)
                    .gamma_multiply_u8(INTERFERENCE_FILL_ALPHA),
                low_sample: is_low_sample(observation),
                observation: *observation,
            })
        })
        .collect()
}

/// The whole world's longitudes, for a window that wraps.
const FULL_LON_RANGE: RangeInclusive<f64> = -180.0..=180.0;

/// The geographic window a Mercator viewport covers, as the inclusive
/// degree ranges [`JamDataset::observations_within`] takes.
///
/// A viewport crossing the antimeridian, or wider than the world, widens to
/// every longitude: [`JamDataset::observations_within`] takes the wider
/// range containing the window, and a wrapped west past east would otherwise
/// select nothing.
fn geographic_window(bounds: gt_types::MercBounds) -> (RangeInclusive<f64>, RangeInclusive<f64>) {
    // Mercator y grows southward, so the smaller y is the northern edge.
    let (north, west) = mercator::denormalize(MercPoint {
        x: bounds.x_min,
        y: bounds.y_min,
    });
    let (south, east) = mercator::denormalize(MercPoint {
        x: bounds.x_max,
        y: bounds.y_max,
    });
    let lat = south.min(north)..=south.max(north);

    let covers_world = bounds.x_max - bounds.x_min >= 1.0;
    let lon = if covers_world || west > east {
        FULL_LON_RANGE
    } else {
        west..=east
    };
    (lat, lon)
}

/// The overlay plugin. Registered before every track renderer, so the cells
/// sit beneath the track ink.
pub(crate) struct JammingRenderer<'a> {
    dataset: &'a JamDataset,
    hover_enabled: bool,
}

impl<'a> JammingRenderer<'a> {
    pub(crate) const fn new(dataset: &'a JamDataset, hover_enabled: bool) -> Self {
        Self {
            dataset,
            hover_enabled,
        }
    }
}

impl Plugin for JammingRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        response: &egui::Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform = MercTransform::new(projector, map_memory, ui.max_rect().center());
        let cells = visible_cells(
            self.dataset,
            &transform,
            ui.max_rect(),
            ui.visuals().dark_mode,
        );
        draw_cells(ui, &cells);

        // Cell hover is disabled while a track element is hovered: a cell
        // covers the whole viewport at track zoom.
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
            .show(|ui| cell_tooltip(ui, cell, self.dataset.day()));
        }
    }
}

/// The cell under the pointer, if any. Cells do not overlap, so the first
/// containing one is returned.
fn cell_at_pointer(cells: &[CellShape], pointer: Pos2) -> Option<&CellShape> {
    cells
        .iter()
        .find(|cell| contains_point(&cell.outline, pointer))
}

/// Whether a convex polygon contains a point.
fn contains_point(polygon: &[Pos2], point: Pos2) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    inward_edges(polygon).all(|(vertex, inward)| (point - vertex).dot(inward) >= 0.0)
}

/// The cell hover shows the counts behind the colour. The display toggle's own
/// hover describes the dataset.
fn cell_tooltip(ui: &mut Ui, cell: &CellShape, day: chrono::NaiveDate) {
    let observation = &cell.observation;
    let Some(rate) = observation.rate() else {
        return;
    };
    for line in gt_jam::text::cell_summary(
        &day.to_string(),
        observation.good,
        observation.bad,
        rate.percent(),
    ) {
        ui.label(line);
    }
    if cell.low_sample {
        ui.label(egui::RichText::new(gt_jam::text::LOW_SAMPLE_CAVEAT).italics());
    }
}

/// Paint the overlay beneath the track ink.
fn draw_cells(ui: &Ui, cells: &[CellShape]) {
    let painter = ui.painter();
    for cell in cells {
        if cell.low_sample {
            paint_hatched(ui, cell);
        } else {
            painter.add(Shape::convex_polygon(
                cell.outline.clone(),
                cell.fill,
                Stroke::NONE,
            ));
        }
    }
}

/// A hatched cell: diagonals at 45 degrees, each clipped to the cell's own
/// outline so nothing paints outside the hexagon.
fn paint_hatched(ui: &Ui, cell: &CellShape) {
    let Some(bounds) = bounding_rect(&cell.outline) else {
        return;
    };
    let painter = ui.painter();
    let stroke = Stroke::new(HATCH_STROKE_WIDTH, cell.fill);

    let span = bounds.width() + bounds.height();
    let mut offset = 0.0;
    while offset < span {
        let line = [
            Pos2::new(bounds.left() + offset, bounds.top()),
            Pos2::new(bounds.left(), bounds.top() + offset),
        ];
        if let Some(clipped) = clip_to_convex(line, &cell.outline) {
            painter.line_segment(clipped, stroke);
        }
        offset += HATCH_SPACING_PX;
    }
}

/// Twice the signed area of a polygon. Negative when its vertices wind the
/// other way, which determines which side of an edge the interior is on.
fn signed_area_x2(polygon: &[Pos2]) -> f32 {
    polygon
        .iter()
        .enumerate()
        .filter_map(|(index, &vertex)| {
            let next = polygon.get((index + 1) % polygon.len())?;
            Some(vertex.x * next.y - next.x * vertex.y)
        })
        .sum()
}

/// Each edge as a start vertex and the normal pointing into the polygon.
///
/// The winding is measured: the projection flips y, which reverses it. A
/// point is inside the polygon when `(point - vertex).dot(inward) >= 0.0` for
/// every edge.
fn inward_edges(polygon: &[Pos2]) -> impl Iterator<Item = (Pos2, egui::Vec2)> + '_ {
    let winding = if signed_area_x2(polygon) < 0.0 {
        -1.0
    } else {
        1.0
    };
    polygon
        .iter()
        .enumerate()
        .filter_map(move |(index, &vertex)| {
            let next = polygon.get((index + 1) % polygon.len())?;
            let edge = *next - vertex;
            Some((vertex, egui::vec2(-edge.y, edge.x) * winding))
        })
}

/// Clip a segment to a convex polygon, or [`None`] when it falls outside.
///
/// H3 cells are convex, so intersecting the segment's parameter range with
/// each edge's inner half-plane is exact.
fn clip_to_convex(segment: [Pos2; 2], polygon: &[Pos2]) -> Option<[Pos2; 2]> {
    let [from, to] = segment;
    let direction = to - from;
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);

    for (vertex, inward) in inward_edges(polygon) {
        let denominator = direction.dot(inward);
        let distance = (vertex - from).dot(inward);

        if denominator.abs() < f32::EPSILON {
            // Parallel to the edge: outside it means the whole segment is.
            if distance > 0.0 {
                return None;
            }
            continue;
        }
        let t = distance / denominator;
        if denominator > 0.0 {
            enter = enter.max(t);
        } else {
            leave = leave.min(t);
        }
        if enter > leave {
            return None;
        }
    }
    Some([from + direction * enter, from + direction * leave])
}

fn bounding_rect(outline: &[Pos2]) -> Option<egui::Rect> {
    let first = outline.first()?;
    let mut rect = egui::Rect::from_min_max(*first, *first);
    for point in outline {
        rect.extend_with(*point);
    }
    Some(rect)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use chrono::NaiveDate;
    use h3o::LatLng;
    use rstest::rstest;

    use super::*;

    /// Pixels the whole world spans in the transforms below, which is the
    /// width a cell wrapped across the antimeridian would project to.
    const WORLD_PX: f64 = 1024.0;

    /// A cell from the captured fixture day: 55.016 N, 15.413 E.
    const BALTIC: &str = "841f0c9ffffffff";

    /// Side of the test viewport, in pixels.
    const CANVAS_PX: f32 = 400.0;

    /// World width in pixels for the test view: about 100 km across the
    /// canvas, so a ring of 22 km cells fits with room around it.
    const TEST_VIEW_TOTAL_PX: f64 = 160_000.0;

    fn observation(hex: &str, good: u32, bad: u32) -> HexObservation {
        HexObservation {
            cell: CellIndex::from_str(hex).expect("cell index"),
            good,
            bad,
        }
    }

    #[rstest]
    #[case::one_aircraft(1, 0, true)]
    #[case::four_aircraft(3, 1, true)]
    #[case::exactly_the_threshold(4, 1, false)]
    #[case::many_aircraft(400, 12, false)]
    fn low_sample_follows_the_aircraft_count(
        #[case] good: u32,
        #[case] bad: u32,
        #[case] expected: bool,
    ) {
        assert_eq!(is_low_sample(&observation(BALTIC, good, bad)), expected);
    }

    /// A cell's outline is a closed ring of projected vertices.
    #[test]
    fn a_cell_projects_to_a_polygon() {
        let transform = MercTransform::for_test(1024.0);
        let cell = CellIndex::from_str(BALTIC).expect("cell index");
        let outline = cell_outline(cell, &transform).expect("outline");
        assert!(outline.len() >= 5, "an H3 cell has 5 or 6 vertices");
    }

    /// A cell straddling the antimeridian projects to a polygon of its own
    /// width: its vertices normalise to both edges of the world, and each one
    /// is placed on the turn of the world its cell centre is on.
    #[test]
    fn a_cell_on_the_antimeridian_stays_one_cell_wide() {
        let position = LatLng::new(0.0, 179.99).expect("a position");
        let cell = position.to_cell(gt_jam::H3_RESOLUTION);
        let longitudes: Vec<f64> = cell.boundary().iter().map(|vertex| vertex.lng()).collect();
        assert!(
            longitudes.iter().any(|lng| *lng > 0.0) && longitudes.iter().any(|lng| *lng < 0.0),
            "{cell} was picked for straddling the antimeridian, got {longitudes:?}"
        );

        let outline = cell_outline(cell, &MercTransform::for_test(WORLD_PX)).expect("outline");
        let bounds = bounding_rect(&outline).expect("bounds");
        let widest_a_cell_can_project = WORLD_PX as f32 / 100.0;
        assert!(
            bounds.width() < widest_a_cell_can_project,
            "the cell projected {} px of the {WORLD_PX} px world",
            bounds.width()
        );
    }

    /// Cells outside the viewport are not projected.
    #[test]
    fn only_cells_in_the_window_are_returned() {
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let baltic = observation(BALTIC, 100, 3);
        let wyoming = observation("8426b45ffffffff", 100, 3);
        let dataset = JamDataset::new(day, vec![baltic, wyoming]);

        let transform = MercTransform::for_test_centered(1_048_576.0, Latitude::new(55.0));
        let rect =
            egui::Rect::from_center_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        let cells = visible_cells(&dataset, &transform, rect, true);
        assert!(
            cells.len() <= 1,
            "a tight window around the Baltic cannot hold both cells"
        );
    }

    /// A cell centred in the viewport projects to a polygon that actually
    /// covers the middle of it. Guards the projection and the view framing
    /// together: an off-by-a-scale-factor puts the outline off-screen while
    /// every count still looks right.
    #[test]
    fn a_centred_cell_covers_the_middle_of_the_viewport() {
        let cell = CellIndex::from_str(BALTIC).expect("cell index");
        let center = LatLng::from(cell);
        let rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        let transform = MercTransform::for_test_view(
            TEST_VIEW_TOTAL_PX,
            Latitude::new(center.lat()),
            Longitude::new(center.lng()),
            rect.center(),
        );

        let outline = cell_outline(cell, &transform).expect("outline");
        let bounds = bounding_rect(&outline).expect("bounds");
        assert!(
            bounds.contains(rect.center()),
            "the cell should cover the viewport centre, got {bounds:?}"
        );
        assert!(
            bounds.width() < rect.width() && bounds.height() < rect.height(),
            "a 22 km cell should fit inside a 100 km view, got {bounds:?}"
        );
    }

    /// Every cell of a real ring projects, and the low-sample ones are the
    /// ones marked for hatching.
    #[test]
    fn a_ring_of_cells_projects_with_its_low_sample_cells_marked() {
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let center_cell = CellIndex::from_str(BALTIC).expect("cell index");
        let tallies = [
            (400, 0),
            (98, 2),
            (94, 6),
            (90, 10),
            (60, 40),
            (2, 2),
            (1, 1),
        ];
        let observations: Vec<HexObservation> = center_cell
            .grid_disk::<Vec<_>>(1)
            .into_iter()
            .zip(tallies)
            .map(|(cell, (good, bad))| HexObservation { cell, good, bad })
            .collect();
        let expected_low_sample = observations.iter().filter(|o| is_low_sample(o)).count();
        let dataset = JamDataset::new(day, observations);

        let center = LatLng::from(center_cell);
        let rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        let transform = MercTransform::for_test_view(
            TEST_VIEW_TOTAL_PX,
            Latitude::new(center.lat()),
            Longitude::new(center.lng()),
            rect.center(),
        );

        let cells = visible_cells(&dataset, &transform, rect, true);
        assert_eq!(
            cells.len(),
            dataset.len(),
            "every cell of the ring is in view"
        );
        assert_eq!(
            cells.iter().filter(|cell| cell.low_sample).count(),
            expected_low_sample
        );
        assert_eq!(expected_low_sample, 2, "two of the seven tallies are thin");
    }

    /// The ramp is continuous across its breakpoints and monotonically
    /// redder, so a cell just past a breakpoint does not jump a tier.
    #[rstest]
    #[case::clear(0.0)]
    #[case::below_the_low_breakpoint(0.01)]
    #[case::at_the_low_breakpoint(0.02)]
    #[case::between(0.05)]
    #[case::at_the_high_breakpoint(0.10)]
    #[case::heavy(0.5)]
    fn the_ramp_reddens_with_the_share(#[case] fraction: f32) {
        let color = gt_ui_theme::interference_color(fraction).dark();
        let clear = gt_ui_theme::interference_color(0.0).dark();
        assert!(
            color.r() >= clear.r(),
            "{fraction} should be no less red than a clear cell"
        );
    }

    #[test]
    fn the_reference_material_quotes_the_breakpoints_cells_are_coloured_at() {
        let low_percent = gt_ui_theme::INTERFERENCE_LOW_BREAKPOINT * 100.0;
        let high_percent = gt_ui_theme::INTERFERENCE_HIGH_BREAKPOINT * 100.0;
        let material = gt_jam::reference::AIRCRAFT_INTERFERENCE.to_string();

        for wording in [
            format!("more than {:.0}% of all aircraft", 100.0 - low_percent),
            format!("between {low_percent:.0}% and {high_percent:.0}% of aircraft"),
            format!("more than {high_percent:.0}% of aircraft"),
            format!("same {low_percent:.0} % and {high_percent:.0} % breakpoints"),
        ] {
            assert!(
                material.contains(&wording),
                "the material never says {wording:?}"
            );
        }
    }

    /// The guard writes [`MIN_AIRCRAFT_FOR_SOLID_FILL`] as a word because the
    /// material spells the count out.
    #[test]
    fn the_reference_material_names_the_aircraft_count_cells_are_hatched_below() {
        let spelled_out = match MIN_AIRCRAFT_FOR_SOLID_FILL {
            3 => "three",
            4 => "four",
            5 => "five",
            6 => "six",
            7 => "seven",
            other => panic!("{other} aircraft is outside the range this guard spells out"),
        };
        let wording = format!("Cells with fewer than {spelled_out} aircraft are hatched");
        let material = gt_jam::reference::AIRCRAFT_INTERFERENCE.to_string();

        assert!(
            material.contains(&wording),
            "the material never says {wording:?}"
        );
    }

    fn bounds(x_min: f64, x_max: f64, y_min: f64, y_max: f64) -> gt_types::MercBounds {
        gt_types::MercBounds {
            x_min,
            x_max,
            y_min,
            y_max,
        }
    }

    /// Mercator y grows southward, so the smaller y must come back as the
    /// northern - larger - latitude.
    #[test]
    fn the_window_undoes_the_north_south_inversion() {
        let (lat, _) = geographic_window(bounds(0.5, 0.6, 0.3, 0.4));
        assert!(lat.start() < lat.end(), "a range runs low to high");
        let (north, _) = mercator::denormalize(MercPoint { x: 0.5, y: 0.3 });
        let (south, _) = mercator::denormalize(MercPoint { x: 0.5, y: 0.4 });
        assert!(north > south, "the smaller y is further north");
        assert_eq!(lat, south..=north);
    }

    /// A viewport past the antimeridian must not select an empty sliver:
    /// its wrapped west lands east of its east. The assertion reads the window
    /// directly. End to end, `observations_within`'s own padding widens a wrong
    /// window enough to hide the fault.
    #[test]
    fn a_window_crossing_the_antimeridian_takes_every_longitude() {
        let (_, lon) = geographic_window(bounds(0.999, 1.001, 0.4, 0.5));
        assert_eq!(lon, FULL_LON_RANGE);
        assert!(lon.contains(&179.9) && lon.contains(&-179.9));
    }

    #[test]
    fn a_window_wider_than_the_world_takes_every_longitude() {
        let (_, lon) = geographic_window(bounds(-0.2, 1.2, 0.0, 1.0));
        assert_eq!(lon, FULL_LON_RANGE);
    }

    #[test]
    fn an_ordinary_window_keeps_its_longitudes() {
        let (_, lon) = geographic_window(bounds(0.54, 0.55, 0.31, 0.32));
        assert_ne!(lon, FULL_LON_RANGE);
        assert!(lon.start() < lon.end());
        assert!(*lon.start() > 14.0 && *lon.end() < 19.0, "{lon:?}");
    }

    /// A hatch line is cut to the cell, so nothing paints in the bounding
    /// box corners outside the hexagon.
    #[test]
    fn hatching_is_clipped_to_the_cell_outline() {
        let cell = CellIndex::from_str(BALTIC).expect("cell index");
        let center = LatLng::from(cell);
        let rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        let transform = MercTransform::for_test_view(
            TEST_VIEW_TOTAL_PX,
            Latitude::new(center.lat()),
            Longitude::new(center.lng()),
            rect.center(),
        );
        let outline = cell_outline(cell, &transform).expect("outline");
        let bounds = bounding_rect(&outline).expect("bounds");

        // A diagonal across the whole bounding box: its ends sit in corners
        // outside the hexagon, so clipping must shorten it.
        let corner_to_corner = [bounds.left_top(), bounds.right_bottom()];
        let clipped = clip_to_convex(corner_to_corner, &outline).expect("crosses the cell");
        let full_length = (corner_to_corner[1] - corner_to_corner[0]).length();
        let clipped_length = (clipped[1] - clipped[0]).length();
        assert!(
            clipped_length < full_length,
            "expected the diagonal to be cut, {clipped_length} of {full_length}"
        );
        for point in clipped {
            assert!(bounds.contains(point));
        }
    }

    #[test]
    fn a_hatch_line_outside_the_cell_is_dropped() {
        let cell = CellIndex::from_str(BALTIC).expect("cell index");
        let transform = MercTransform::for_test(1024.0);
        let outline = cell_outline(cell, &transform).expect("outline");
        let far_away = [egui::pos2(-9000.0, -9000.0), egui::pos2(-8900.0, -8900.0)];
        assert_eq!(clip_to_convex(far_away, &outline), None);
    }

    /// A cell's own centre is inside it. A point well outside its bounding
    /// box is not.
    #[test]
    fn a_cell_contains_its_own_centre() {
        let cell = CellIndex::from_str(BALTIC).expect("cell index");
        let center = LatLng::from(cell);
        let rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        let transform = MercTransform::for_test_view(
            TEST_VIEW_TOTAL_PX,
            Latitude::new(center.lat()),
            Longitude::new(center.lng()),
            rect.center(),
        );
        let outline = cell_outline(cell, &transform).expect("outline");

        assert!(contains_point(&outline, rect.center()));
        assert!(!contains_point(&outline, egui::pos2(-9000.0, -9000.0)));
    }

    /// The bounding box corners lie outside the hexagon, which is what
    /// separates a polygon hit test from a rectangle one.
    #[test]
    fn a_cells_bounding_box_corners_are_outside_it() {
        let cell = CellIndex::from_str(BALTIC).expect("cell index");
        let center = LatLng::from(cell);
        let rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        let transform = MercTransform::for_test_view(
            TEST_VIEW_TOTAL_PX,
            Latitude::new(center.lat()),
            Longitude::new(center.lng()),
            rect.center(),
        );
        let outline = cell_outline(cell, &transform).expect("outline");
        let bounds = bounding_rect(&outline).expect("bounds");

        for corner in [
            bounds.left_top(),
            bounds.right_top(),
            bounds.left_bottom(),
            bounds.right_bottom(),
        ] {
            assert!(
                !contains_point(&outline, corner),
                "{corner:?} is a bounding-box corner, outside the hexagon"
            );
        }
    }

    /// The pointer picks the cell it is inside, not merely a nearby one.
    #[test]
    fn the_pointer_picks_the_cell_it_is_in() {
        let day = NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
        let center_cell = CellIndex::from_str(BALTIC).expect("cell index");
        let observations: Vec<HexObservation> = center_cell
            .grid_disk::<Vec<_>>(1)
            .into_iter()
            .enumerate()
            .map(|(index, cell)| HexObservation {
                cell,
                good: 100,
                bad: u32::try_from(index).unwrap_or_default(),
            })
            .collect();
        let dataset = JamDataset::new(day, observations);

        let center = LatLng::from(center_cell);
        let rect =
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(CANVAS_PX, CANVAS_PX));
        let transform = MercTransform::for_test_view(
            TEST_VIEW_TOTAL_PX,
            Latitude::new(center.lat()),
            Longitude::new(center.lng()),
            rect.center(),
        );
        let cells = visible_cells(&dataset, &transform, rect, true);

        let hit = cell_at_pointer(&cells, rect.center()).expect("a cell under the centre");
        assert_eq!(hit.observation.cell, center_cell);
        assert!(cell_at_pointer(&cells, egui::pos2(-9000.0, -9000.0)).is_none());
    }
}
