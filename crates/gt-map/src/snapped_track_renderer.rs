//! Snapped-track polylines: the road-matched reference geometry, drawn per
//! track in the track's color, dashed and translucent - visually subordinate
//! to the recorded track it annotates.
//!
//! The geometry arrives pre-projected to normalized Mercator (segments are
//! immutable once a run completes; see `gt_ui_types::SnappedTracks`). Breaks
//! between segments render as gaps - route discontinuities and unsnapped
//! runs - and the recorded track underneath is never painted over or hidden.
//!
//! Dashing emits one shape per dash period of *screen-space* path length,
//! which grows linearly with zoom regardless of the segment's vertex count.
//! Segments therefore go through the [`visible_path`] culling machinery like
//! the recorded trackline: off-screen stretches never generate dashes and
//! sub-pixel detail is merged, bounding the per-frame shape count by what is
//! actually on screen.

use egui::{Pos2, Response, Stroke, Ui};
use gt_ui_types::SnappedTracks;
use walkers::{MapMemory, Plugin, Projector};

use crate::polyline::{CULL_MARGIN_PX, VisiblePath, visible_path};
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

pub struct SnappedTrackRenderer<'a> {
    snapped: &'a SnappedTracks,
}

impl<'a> SnappedTrackRenderer<'a> {
    pub fn new(snapped: &'a SnappedTracks) -> Self {
        Self { snapped }
    }
}

impl Plugin for SnappedTrackRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform = MercTransform::new(projector, map_memory, ui.max_rect().center());
        let cull_rect = ui.max_rect().expand(CULL_MARGIN_PX);
        // Reused across spans so stripping the (unit) styling keys costs one
        // allocation per frame, not one per span.
        let mut span_points: Vec<Pos2> = Vec::new();
        for (track_ref, segments) in &self.snapped.segments_by_track {
            let color =
                gt_ui_theme::track_color(track_ref.fi.as_usize(), track_ref.index.as_usize())
                    .gamma_multiply(SNAPPED_ALPHA);
            let stroke = Stroke::new(SNAPPED_STROKE_WIDTH, color);
            // `.iter()` is load-bearing: `segments` is `&Arc<Vec<_>>`, which
            // only reaches the Vec's iterator through Deref method resolution.
            for segment in segments.iter() {
                // Segments carry no per-point styling, so the key is unit.
                let points = segment.iter().map(|&merc| ((), transform.to_screen(merc)));
                match visible_path(points, cull_rect) {
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
    }
}
