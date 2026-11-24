//! Query-match halos: wide strokes beneath the track line marking the
//! stretches a query matched, rings for single-point matches, and the match
//! header shown in the hover tooltip.

use egui::epaint::{CircleShape, Mesh, PathShape, TessellationOptions, Tessellator, Vertex};
use egui::{Color32, Pos2, RichText, Shape, Stroke, Ui};
use gt_types::{LoadedFile, TrackRef};
use gt_ui_types::StaleRunNote;

use crate::match_reveal::HaloStyle;
use crate::polyline::PolylineSpans;

pub(crate) struct HaloClip {
    sample_centers: egui::Rect,
    pixels_per_point: f32,
    tessellator: Tessellator,
    mesh: Mesh,
}

impl HaloClip {
    pub(crate) fn new(
        rect: egui::Rect,
        pixels_per_point: f32,
        options: TessellationOptions,
    ) -> Self {
        // The font atlas dimensions apply to glyphs.
        let mut tessellator = Tessellator::new(pixels_per_point, options, [1, 1], Vec::new());
        tessellator.set_clip_rect(rect);
        // The WGPU scissor rounds its edges in physical pixels.
        let scissor = (rect * pixels_per_point).round();
        Self {
            sample_centers: egui::Rect::from_min_max(
                scissor.min.max(Pos2::ZERO) + PIXEL_CENTER_OFFSET,
                scissor.max - PIXEL_CENTER_OFFSET,
            ),
            pixels_per_point,
            tessellator,
            mesh: Mesh::default(),
        }
    }

    pub(crate) fn band_is_visible(&mut self, spans: &PolylineSpans<()>, halo: HaloStyle) -> bool {
        spans.iter().any(|span| {
            self.shape_is_visible(Shape::Path(
                halo.band_shape(span, gt_ui_theme::QUERY_MATCH_HOVER_HALO),
            ))
        })
    }

    pub(crate) fn ring_is_visible(&mut self, position: Pos2, halo: HaloStyle) -> bool {
        self.shape_is_visible(Shape::Circle(
            halo.ring_shape(position, gt_ui_theme::QUERY_MATCH_HOVER_HALO),
        ))
    }

    fn shape_is_visible(&mut self, shape: Shape) -> bool {
        self.mesh.indices.clear();
        self.mesh.vertices.clear();
        self.tessellator.tessellate_shape(shape, &mut self.mesh);
        self.mesh.triangles().any(|triangle| {
            let vertices =
                triangle.map(|index| self.mesh.vertices.get(usize::try_from(index).ok()?));
            let [Some(a), Some(b), Some(c)] = vertices else {
                return false;
            };
            [a, b, c].iter().any(|vertex| vertex.color.a() > 0)
                && self::triangle_covers_sample(
                    [a, b, c],
                    self.pixels_per_point,
                    self.sample_centers,
                )
        })
    }
}

fn triangle_covers_sample(
    mut vertices: [&Vertex; 3],
    pixels_per_point: f32,
    sample_centers: egui::Rect,
) -> bool {
    let mut points = vertices.map(|vertex| vertex.pos * pixels_per_point);
    let pixel_indices = egui::Rect::from_points(&points)
        .intersect(sample_centers)
        .translate(-PIXEL_CENTER_OFFSET);
    if pixel_indices.is_negative() || !pixel_indices.is_finite() {
        return false;
    }
    let first = pixel_indices.min.ceil();
    let last = pixel_indices.max.floor();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "pixel indices are rounded within the framebuffer bounds"
    )]
    let [left, top, right, bottom] = [first.x, first.y, last.x, last.y].map(|edge| edge as i32);
    let normal = |direction: egui::Vec2| egui::vec2(-direction.y, direction.x);
    let [a, b, c] = points;
    let winding = (c - a).dot(normal(b - a));
    if winding == 0.0 {
        return false;
    }
    if winding < 0.0 {
        points.swap(1, 2);
        vertices.swap(1, 2);
    }
    let [a, b, c] = points;
    let edges = [[b, c], [c, a], [a, b]].map(|[from, to]| {
        let direction = to - from;
        // A sample on a shared edge belongs to the triangle with the top or left edge.
        let inclusive = direction.y < 0.0 || (direction.y == 0.0 && direction.x > 0.0);
        (from, normal(direction), inclusive)
    });
    let [alpha_a, alpha_b, alpha_c] = vertices.map(|vertex| f32::from(vertex.color.a()));
    (top..=bottom).any(|row| {
        (left..=right).any(|column| {
            let sample = egui::pos2(column as f32, row as f32) + PIXEL_CENTER_OFFSET;
            let weights = edges.map(|(from, normal, inclusive)| {
                let weight = (sample - from).dot(normal);
                (weight > 0.0 || (weight == 0.0 && inclusive)).then_some(weight)
            });
            let [Some(a), Some(b), Some(c)] = weights else {
                return false;
            };
            a * alpha_a + b * alpha_b + c * alpha_c > 0.0
        })
    })
}

impl HaloStyle {
    fn band_shape<K>(self, span: &[(K, Pos2)], color: Color32) -> PathShape {
        let points = span.iter().map(|&(_, position)| position).collect();
        PathShape::line(points, Stroke::new(self.band_width, color))
    }

    fn ring_shape(self, position: Pos2, color: Color32) -> CircleShape {
        CircleShape::stroke(
            position,
            self.ring_radius,
            Stroke::new(self.ring_stroke_width, color),
        )
    }
}

/// Paint one draw layer's halos from a track's prepared span geometry, in the
/// layer's `color`.
///
/// `span` is one culling span of the track's polyline. `matched` projects
/// each point's key to whether this layer covers it, so no intermediate
/// buffer is built per frame. Consecutive matched points form the halo
/// stretches. Runs of one point (a single-point match, or a longer match
/// reduced to one visible point by LOD selection) draw as a ring so they stay
/// visible.
pub(crate) fn paint_match_halo_span<K>(
    ui: &Ui,
    span: &[(K, Pos2)],
    matched: impl Fn(&K) -> bool,
    halo: HaloStyle,
    color: Color32,
) {
    for run in matched_runs(span, &matched) {
        match run {
            [] => {}
            [(_, pos)] => draw_match_ring(ui, *pos, halo, color),
            _ => {
                ui.painter().add(Shape::Path(halo.band_shape(run, color)));
            }
        }
    }
}

/// Ring around an isolated matched point (also used when a whole track
/// collapses to a sub-pixel dot).
pub(crate) fn draw_match_ring(ui: &Ui, pos: Pos2, halo: HaloStyle, color: Color32) {
    ui.painter().add(Shape::Circle(halo.ring_shape(pos, color)));
}

/// Maximal runs of consecutive matched points within one span.
///
/// A halo edge exists only when BOTH endpoints matched, so the halo never
/// overshoots the match by an edge.
fn matched_runs<'a, K>(
    span: &'a [(K, Pos2)],
    matched: &'a impl Fn(&K) -> bool,
) -> impl Iterator<Item = &'a [(K, Pos2)]> {
    span.split(move |(key, _)| !matched(key))
        .filter(|run| !run.is_empty())
}

/// The tooltip header for a hovered match: point count and duration, plus
/// the stale note. The point table below it is the standard hover table.
pub(crate) fn match_header_ui(
    ui: &mut Ui,
    files: &[LoadedFile],
    track_ref: TrackRef,
    range: &std::ops::Range<usize>,
    stale: bool,
) {
    let Some(track) = track_ref.resolve(files) else {
        return;
    };
    let count = range.len();
    let duration = gt_fmt::match_duration_seconds(track, range);
    let em_dash = gt_ui_theme::EM_DASH;
    let heading = match duration {
        Some(secs) => format!(
            "Match {em_dash} {count} points over {}",
            gt_fmt::format_match_duration(secs)
        ),
        None => format!("Match {em_dash} {count} points"),
    };
    ui.strong(heading);
    if stale {
        ui.label(
            RichText::new(StaleRunNote::RunAgain.to_string())
                .weak()
                .italics(),
        );
    }
    ui.separator();
}

const PIXEL_CENTER_OFFSET: egui::Vec2 = egui::vec2(0.5, 0.5);

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn shared_sample_uses_one_triangle_and_its_edge_alpha() {
        let pixels_per_point = 1.25;
        let clip = HaloClip::new(
            egui::Rect::from_min_max(egui::pos2(2.25, 0.0), egui::pos2(3.15, 0.8)),
            pixels_per_point,
            TessellationOptions::default(),
        );
        let [top_left, top_right, bottom_left, bottom_right] =
            [(3.0, 0.0), (4.0, 0.0), (3.0, 1.0), (4.0, 1.0)].map(|(x, y)| {
                Vertex::untextured(egui::pos2(x, y) / pixels_per_point, Color32::WHITE)
            });
        let sample_is_covered = |vertices| {
            self::triangle_covers_sample(vertices, pixels_per_point, clip.sample_centers)
        };
        assert_eq!(
            clip.sample_centers,
            egui::Rect::from_min_max(egui::pos2(3.5, 0.5), egui::pos2(3.5, 0.5)),
        );
        assert!(!sample_is_covered([&top_left, &top_right, &bottom_left]));
        assert!(sample_is_covered([&bottom_right, &bottom_left, &top_right]));
        assert!(sample_is_covered([&bottom_right, &top_right, &bottom_left]));
        let transparent_top = Vertex::untextured(top_right.pos, Color32::TRANSPARENT);
        let transparent_bottom = Vertex::untextured(bottom_left.pos, Color32::TRANSPARENT);
        assert!(!sample_is_covered([
            &bottom_right,
            &transparent_bottom,
            &transparent_top,
        ]));
    }

    fn span(flags: &[bool]) -> Vec<(bool, Pos2)> {
        flags
            .iter()
            .enumerate()
            .map(|(i, &matched)| (matched, egui::pos2(i as f32, 0.0)))
            .collect()
    }

    fn is_matched(key: &bool) -> bool {
        *key
    }

    /// A halo covers a maximal run of matched points, and breaks at every
    /// point the run did not match.
    #[rstest]
    #[case::runs_split_at_the_unmatched_points(
        &[false, true, true, false, true, false, true, true, true],
        vec![2, 1, 3]
    )]
    #[case::everything_matched(&[true, true, true], vec![3])]
    #[case::nothing_matched(&[false, false], vec![])]
    fn matched_runs_break_at_the_unmatched_points(
        #[case] matched: &[bool],
        #[case] expected: Vec<usize>,
    ) {
        let span = span(matched);
        let runs: Vec<usize> = matched_runs(&span, &is_matched).map(<[_]>::len).collect();
        assert_eq!(runs, expected);
    }

    /// The header states how long the match ran, over the shared duration
    /// helper. The fixture's points are spaced exactly one second apart.
    #[test]
    fn match_duration_covers_first_to_last_point() {
        let track = gt_test_utils::loaded_track_with_points(gt_test_utils::nav_test_data());
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(0..1)), None);
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(0..3)), Some(2));
        assert_eq!(
            gt_fmt::match_duration_seconds(&track, &(150..300)),
            Some(149)
        );
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(0..10_000)), None);
        assert_eq!(gt_fmt::match_duration_seconds(&track, &(5..5)), None);
    }
}
