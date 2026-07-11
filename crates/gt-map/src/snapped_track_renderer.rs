//! Snapped-track polylines: the road-matched reference geometry, drawn per
//! track in the track's color, dashed and translucent - visually subordinate
//! to the recorded track it annotates.
//!
//! The geometry arrives pre-projected to normalized Mercator (segments are
//! immutable once a run completes; see `gt_ui_types::SnappedTracks`). Breaks
//! between segments render as gaps - route discontinuities and unsnapped
//! runs - and the recorded track underneath is never painted over or hidden.

use egui::{Pos2, Response, Stroke, Ui};
use gt_ui_types::SnappedTracks;
use walkers::{MapMemory, Plugin, Projector};

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
        for (track_ref, segments) in &self.snapped.segments_by_track {
            let color =
                gt_ui_theme::track_color(track_ref.fi.as_usize(), track_ref.index.as_usize())
                    .gamma_multiply(SNAPPED_ALPHA);
            let stroke = Stroke::new(SNAPPED_STROKE_WIDTH, color);
            // `.iter()` is load-bearing: `segments` is `&Arc<Vec<_>>`, which
            // only reaches the Vec's iterator through Deref method resolution.
            for segment in segments.iter() {
                // Segments are bounded by the run's sent-point count (thinned
                // to at most 1 point/s), so projecting every vertex per frame
                // stays cheap without the trackline's culling machinery.
                let points: Vec<Pos2> = segment
                    .iter()
                    .map(|&merc| transform.to_screen(merc))
                    .collect();
                draw_dashed_line(
                    ui.painter(),
                    &points,
                    stroke,
                    SNAPPED_DASH_PX,
                    SNAPPED_GAP_PX,
                );
            }
        }
    }
}
