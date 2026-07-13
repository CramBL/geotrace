//! Snapped-track polylines: the road-matched reference geometry, drawn per
//! track in the track's color, dashed and translucent - visually subordinate
//! to the recorded track it annotates.
//!
//! The geometry arrives pre-projected to normalized Mercator with per-vertex
//! edge attribution (segments are immutable once a run completes; see
//! [`gt_ui_types::SnappedTracks`]). Breaks between segments render as gaps -
//! route discontinuities and unsnapped runs - and the recorded track
//! underneath is never painted over or hidden.
//!
//! Dashing emits one shape per dash period of *screen-space* path length,
//! which grows linearly with zoom regardless of the segment's vertex count.
//! Segments therefore go through the [`visible_path`] culling machinery like
//! the recorded trackline: off-screen stretches never generate dashes and
//! sub-pixel detail is merged, bounding the per-frame shape count by what is
//! actually on screen.
//!
//! Hovering the snapped line shows the matched edge's attributes (name,
//! road class, speed limit, surface). The hit-test runs over the original
//! vertices - their indices address the edge spans - with a cheap same-side
//! rejection against a small rect around the cursor, so its cost stays
//! proportional to the geometry near the pointer.

use egui::{Pos2, Rect, Response, RichText, Stroke, Ui};
use gt_ui_types::{SnappedEdgeInfo, SnappedTracks};
use walkers::{MapMemory, Plugin, Projector};

use crate::polyline::{CULL_MARGIN_PX, VisiblePath, segment_outside, visible_path};
use crate::track_renderer::draw_dashed_line;
use crate::transform::MercTransform;

/// Stroke width. Thinner than the recorded trackline (3.0) so the reference
/// geometry reads as an annotation, not a second track.
const SNAPPED_STROKE_WIDTH: f32 = 2.0;

/// Alpha the track color is reduced to for the snapped track.
const SNAPPED_ALPHA: f32 = 0.55;

/// Dash and gap lengths in screen pixels. Deliberately shorter than the
/// ghost-fix dashing (8/5) so the two dashed styles stay distinguishable.
const SNAPPED_DASH_PX: f32 = 5.0;
const SNAPPED_GAP_PX: f32 = 4.0;

/// Hover hit radius around the snapped line, screen pixels. Tighter than
/// the recorded data's 20 px point hit-test: a line is a precise target,
/// and the recorded elements should win the contested band around them.
const SNAPPED_HOVER_RADIUS_PX: f32 = 10.0;

/// The nearest snapped-line hit under the cursor.
struct HoverHit {
    distance_sq: f32,
    track: gt_types::TrackRef,
    segment: usize,
    /// Index of the hit line's leading vertex, addressing the edge spans.
    vertex: usize,
}

pub struct SnappedTrackRenderer<'a> {
    snapped: &'a SnappedTracks,
    /// Hover is disabled while the recorded data owns the pointer (an
    /// active hover on recorded elements) - the primary data wins.
    hover_enabled: bool,
}

impl<'a> SnappedTrackRenderer<'a> {
    pub fn new(snapped: &'a SnappedTracks, hover_enabled: bool) -> Self {
        Self {
            snapped,
            hover_enabled,
        }
    }
}

impl Plugin for SnappedTrackRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform = MercTransform::new(projector, map_memory, ui.max_rect().center());
        let cull_rect = ui.max_rect().expand(CULL_MARGIN_PX);
        let pointer = (self.hover_enabled && response.hovered())
            .then(|| response.hover_pos())
            .flatten();
        let cursor_rect = pointer
            .map(|p| Rect::from_center_size(p, egui::Vec2::splat(2.0 * SNAPPED_HOVER_RADIUS_PX)));

        // Reused across segments so projection and key-stripping cost one
        // allocation per frame, not one per segment or span.
        let mut projected: Vec<Pos2> = Vec::new();
        let mut span_points: Vec<Pos2> = Vec::new();
        // Order-independent: only a strictly nearer hit replaces `best`, so
        // the HashMap's iteration order never affects the result.
        let mut best: Option<HoverHit> = None;

        for (&track_ref, geometry) in &self.snapped.by_track {
            let color =
                gt_ui_theme::track_color(track_ref.fi.as_usize(), track_ref.index.as_usize())
                    .gamma_multiply(SNAPPED_ALPHA);
            let stroke = Stroke::new(SNAPPED_STROKE_WIDTH, color);
            for (segment_index, segment) in geometry.segments.iter().enumerate() {
                projected.clear();
                projected.extend(segment.points.iter().map(|&merc| transform.to_screen(merc)));

                if let (Some(cursor), Some(cursor_rect)) = (pointer, cursor_rect) {
                    hit_test(
                        &projected,
                        cursor,
                        cursor_rect,
                        track_ref,
                        segment_index,
                        &mut best,
                    );
                }

                // Segments carry no per-point styling, so the key is unit.
                match visible_path(projected.iter().map(|&p| ((), p)), cull_rect) {
                    VisiblePath::OffScreen => {}
                    // A segment collapsed below one pixel (extreme zoom-out)
                    // stays discoverable as a dot, like the recorded track.
                    VisiblePath::Dot((), pos) => {
                        ui.painter().circle_filled(pos, stroke.width, stroke.color);
                    }
                    VisiblePath::Spans(spans) => {
                        for span in spans.iter() {
                            span_points.clear();
                            span_points.extend(span.iter().map(|&((), pos)| pos));
                            draw_dashed_line(
                                ui.painter(),
                                &span_points,
                                stroke,
                                SNAPPED_DASH_PX,
                                SNAPPED_GAP_PX,
                            );
                        }
                    }
                }
            }
        }

        if let Some(hit) = &best {
            let edge = self.snapped.by_track.get(&hit.track).and_then(|geometry| {
                let info = geometry.segments.get(hit.segment)?.edge_at(hit.vertex)?;
                geometry.edges.get(info)
            });
            if let Some(edge) = edge {
                response.show_tooltip_ui(|ui| edge_tooltip_rows(ui, edge));
            }
        }
    }
}

/// Track the nearest in-radius hit of the cursor against a projected
/// segment. Same-side rejection against the small cursor rect keeps the
/// per-frame cost proportional to the geometry near the pointer.
fn hit_test(
    projected: &[Pos2],
    cursor: Pos2,
    cursor_rect: Rect,
    track: gt_types::TrackRef,
    segment: usize,
    best: &mut Option<HoverHit>,
) {
    let radius_sq = SNAPPED_HOVER_RADIUS_PX * SNAPPED_HOVER_RADIUS_PX;
    for (vertex, window) in projected.windows(2).enumerate() {
        let [a, b] = window else { continue };
        if segment_outside(*a, *b, cursor_rect) {
            continue;
        }
        let distance_sq = point_segment_distance_sq(cursor, *a, *b);
        if distance_sq > radius_sq {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|hit| distance_sq < hit.distance_sq)
        {
            *best = Some(HoverHit {
                distance_sq,
                track,
                segment,
                vertex,
            });
        }
    }
}

/// Squared distance from `p` to the line segment `a`-`b`, screen pixels.
fn point_segment_distance_sq(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length_sq = ab.length_sq();
    if length_sq <= f32::EPSILON {
        return (p - a).length_sq();
    }
    let t = ((p - a).dot(ab) / length_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length_sq()
}

/// The hover rows: the matched edge's name plus its attribute grid. Absent
/// attributes are omitted - the tooltip states what is known, not what the
/// map data lacks.
fn edge_tooltip_rows(ui: &mut Ui, edge: &SnappedEdgeInfo) {
    ui.label(RichText::new(edge.name.as_deref().unwrap_or("Unnamed road")).strong());
    egui::Grid::new("snapped_edge_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            let mut row = |label: &str, value: String| {
                ui.label(RichText::new(label).weak());
                ui.label(value);
                ui.end_row();
            };
            if let Some(road_class) = &edge.road_class {
                row("Road class", road_class.clone());
            }
            if let Some(limit) = edge.speed_limit_kmh {
                row("Speed limit", format!("{limit} km/h"));
            }
            if let Some(surface) = &edge.surface {
                row("Surface", surface.clone());
            }
        });
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::point_segment_distance_sq;

    /// Interior projection, endpoint clamping, and the degenerate
    /// zero-length segment.
    #[rstest::rstest]
    #[case::perpendicular_to_interior(pos2(5.0, 3.0), pos2(0.0, 0.0), pos2(10.0, 0.0), 9.0)]
    #[case::clamps_to_start(pos2(-4.0, 3.0), pos2(0.0, 0.0), pos2(10.0, 0.0), 25.0)]
    #[case::clamps_to_end(pos2(14.0, 3.0), pos2(0.0, 0.0), pos2(10.0, 0.0), 25.0)]
    #[case::zero_length_segment(pos2(3.0, 4.0), pos2(0.0, 0.0), pos2(0.0, 0.0), 25.0)]
    fn distance_to_segment(
        #[case] p: egui::Pos2,
        #[case] a: egui::Pos2,
        #[case] b: egui::Pos2,
        #[case] expected_sq: f32,
    ) {
        let got = point_segment_distance_sq(p, a, b);
        assert!(
            (got - expected_sq).abs() < 1e-4,
            "expected {expected_sq}, got {got}"
        );
    }
}
