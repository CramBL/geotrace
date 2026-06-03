use egui::{Color32, Response, Stroke, Ui};
use gt_filter::{GlobalFilter, track_passes_filter};
use gt_types::{DataCategory, FileIdx, LoadedFile, MercBounds, TrackIdx, TrackRef};
use gt_ui_theme::{HIGHLIGHT_BLUE, track_color};
use gt_ui_types::{HighlightScope, MapHighlight, TrackDataVisibility};
use walkers::{MapMemory, Plugin, Projector};

pub struct TrackRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    /// First file index that is considered "newly loaded"; files[new_file_boundary..]
    /// receive a blinking overlay while `blink_alpha > 0`.
    new_file_boundary: usize,
    /// Current blink intensity in [0.0, 1.0]. Zero means no overlay.
    blink_alpha: f32,
}

impl<'a> TrackRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        new_file_boundary: usize,
        blink_alpha: f32,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            new_file_boundary,
            blink_alpha,
        }
    }

    fn track_stroke(&self, fi: FileIdx, ti: TrackIdx) -> Stroke {
        if self.is_trip_highlighted(fi, ti) {
            Stroke::new(4.0, HIGHLIGHT_BLUE)
        } else {
            Stroke::new(3.0, track_color(fi.as_usize(), ti.as_usize()))
        }
    }

    fn is_trip_highlighted(&self, fi: FileIdx, ti: TrackIdx) -> bool {
        let track = TrackRef::new(fi, ti);
        if self.highlight.sticky.is_some_and(|r| r.track == track) {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::File { file_index }) => file_index == fi,
            Some(HighlightScope::Track(t)) => t == track,
            Some(HighlightScope::TrackCategory { track: t, category }) => {
                t == track && matches!(category, DataCategory::Track | DataCategory::Tpv)
            }
            Some(HighlightScope::Point(_)) | None => false,
        }
    }
}

impl Plugin for TrackRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        // Build the per-frame coordinate transform once; all per-point calls are
        // then two f64 multiplies + two f64 adds with no large-value cancellation.
        let transform = crate::MercTransform::new(projector, map_memory, ui.max_rect().center());

        // Viewport bounds in Mercator space — used to skip tracks that are
        // entirely outside the visible area without iterating any points.
        let view_rect = ui.max_rect();
        let vp_bounds = MercBounds {
            x_min: transform.merc_x_from_screen(view_rect.min.x),
            x_max: transform.merc_x_from_screen(view_rect.max.x),
            y_min: transform.merc_y_from_screen(view_rect.min.y),
            y_max: transform.merc_y_from_screen(view_rect.max.y),
        };

        for (fi, file) in self.files.iter().enumerate() {
            let fi = FileIdx::new(fi);
            let Some(file_vis) = fi.get(&self.visibility.files) else {
                continue;
            };
            if !file_vis.enabled {
                continue;
            }
            for (ti, track) in file.tracks.iter().enumerate() {
                let ti = TrackIdx::new(ti);
                let Some(trip_vis) = ti.get(&file_vis.tracks) else {
                    continue;
                };
                if !trip_vis.enabled || !trip_vis.track_visible {
                    continue;
                }
                if !track_passes_filter(&track.metadata, self.filter) {
                    continue;
                }
                // Per-track viewport cull: if the track's Mercator bounding box
                // does not intersect the viewport, skip it entirely.
                if !track.metadata.merc_bounds.intersects(vp_bounds) {
                    continue;
                }
                let stroke = self.track_stroke(fi, ti);

                // Collect (is_ghost, screen_pos) for each visible point.
                // Ghost fixes (heading == None) are dead-reckoned post-last-fix
                // positions rendered as dashed segments.
                let pts: Vec<(bool, egui::Pos2)> = track
                    .points
                    .iter()
                    .filter(|p| {
                        gt_filter::point_passes_time_filter(p.tpv.time().utc(), self.filter)
                    })
                    .map(|p| (p.tpv.heading().is_none(), transform.to_screen(p.merc)))
                    .collect();

                if pts.len() < 2 {
                    continue;
                }

                // Blink overlay uses the full path without ghost distinction.
                let need_blink = self.blink_alpha > 0.0 && fi.as_usize() >= self.new_file_boundary;
                let blink_path: Option<Vec<egui::Pos2>> =
                    need_blink.then(|| pts.iter().map(|(_, pos)| *pos).collect());

                draw_track_with_ghost(ui.painter(), &pts, stroke);

                // Blink overlay: draw a bright pulsing stroke on top of
                // newly loaded tracks for the first 3 seconds after load.
                if let Some(bp) = blink_path {
                    #[expect(
                        clippy::cast_sign_loss,
                        reason = "blink_alpha is clamped to [0,1] in NavMap::draw so product is non-negative"
                    )]
                    let blink_a = (self.blink_alpha * 200.0) as u8;
                    let blink_color = Color32::from_rgba_unmultiplied(255, 230, 80, blink_a);
                    let blink_stroke = Stroke::new(6.0, blink_color);
                    ui.painter().add(egui::Shape::line(bp, blink_stroke));
                }
            }
        }
    }
}

/// Draw a track polyline where ghost-fix edges (either endpoint has `heading == None`)
/// are rendered as dashed lines and real edges as solid lines.
///
/// An edge is ghost when either endpoint is a ghost fix, so the dashed region
/// extends one segment on each side of every ghost point — ensuring the
/// visual uncertainty is clear even at the real→ghost boundary.
fn draw_track_with_ghost(painter: &egui::Painter, pts: &[(bool, egui::Pos2)], stroke: Stroke) {
    if pts.len() < 2 {
        return;
    }

    let mut solid_run: Vec<egui::Pos2> = Vec::new();
    let mut ghost_run: Vec<egui::Pos2> = Vec::new();

    for w in pts.windows(2) {
        let [(ghost_a, pos_a), (ghost_b, pos_b)] = w else {
            continue;
        };
        let (ghost_a, pos_a, ghost_b, pos_b) = (*ghost_a, *pos_a, *ghost_b, *pos_b);
        let edge_is_ghost = ghost_a || ghost_b;

        if edge_is_ghost {
            if solid_run.len() >= 2 {
                painter.add(egui::Shape::line(std::mem::take(&mut solid_run), stroke));
            } else {
                solid_run.clear();
            }
            if ghost_run.is_empty() {
                ghost_run.push(pos_a);
            }
            ghost_run.push(pos_b);
        } else {
            if ghost_run.len() >= 2 {
                draw_dashed_line(painter, &ghost_run, stroke, 8.0, 5.0);
            }
            ghost_run.clear();
            if solid_run.is_empty() {
                solid_run.push(pos_a);
            }
            solid_run.push(pos_b);
        }
    }

    if solid_run.len() >= 2 {
        painter.add(egui::Shape::line(solid_run, stroke));
    }
    if ghost_run.len() >= 2 {
        draw_dashed_line(painter, &ghost_run, stroke, 8.0, 5.0);
    }
}

/// Draw a polyline as a dashed line with the given dash and gap lengths in screen pixels.
fn draw_dashed_line(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    stroke: Stroke,
    dash: f32,
    gap: f32,
) {
    if points.len() < 2 {
        return;
    }
    let period = dash + gap;
    let mut phase: f32 = 0.0;
    let mut dash_start: Option<egui::Pos2> = None;

    for w in points.windows(2) {
        let [a, b] = w else { continue };
        let (a, b) = (*a, *b);
        let seg_len = (b - a).length();
        if seg_len < f32::EPSILON {
            continue;
        }
        let dir = (b - a) / seg_len;
        let mut pos = a;
        let mut remaining = seg_len;

        while remaining > f32::EPSILON {
            let in_dash = phase < dash;
            let phase_end = if in_dash { dash } else { period };
            let step = (phase_end - phase).min(remaining);
            let next_pos = pos + dir * step;

            if in_dash {
                if dash_start.is_none() {
                    dash_start = Some(pos);
                }
            } else if let Some(start) = dash_start.take() {
                painter.line_segment([start, pos], stroke);
            }

            pos = next_pos;
            remaining -= step;
            phase += step;

            // Transition: end of dash → start of gap, or end of gap → start of dash.
            if phase + f32::EPSILON >= phase_end {
                if in_dash {
                    if let Some(start) = dash_start.take() {
                        painter.line_segment([start, pos], stroke);
                    }
                    phase = dash;
                } else {
                    phase = 0.0;
                }
            }
        }
    }

    // Flush any final in-progress dash.
    if let Some(start) = dash_start
        && let Some(&last) = points.last()
    {
        painter.line_segment([start, last], stroke);
    }
}
