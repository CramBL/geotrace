use egui::{Color32, Stroke};
use gt_types::{DataCategory, FileIdx, TrackIdx, TrackRef};
use gt_ui_theme::{HIGHLIGHT_BLUE, track_color};
use gt_ui_types::{HighlightScope, MapHighlight};

/// Dashing of the stretches drawn through ghost fixes.
pub(crate) const GHOST_FIX_DASH: DashPattern = DashPattern {
    dash_px: 8.0,
    gap_px: 5.0,
};

/// Alpha multiplier for elements on non-focused tracks while hover is active.
///
/// Used by marker renderers, which draw at this alpha on top of the fade
/// overlay. The overlay's own opacity is [`FOCUS_SCRIM_MAX_ALPHA_LIGHT`] /
/// [`FOCUS_SCRIM_MAX_ALPHA_DARK`], tuned independently.
pub(crate) const HOVER_FADE_ALPHA: f32 = 0.15;

/// The focus scrim always dims by darkening, in both themes: a translucent
/// near-black rect over the whole viewport when a track is focused, so the
/// non-focused map and tracks recede and the focused track (painted on top)
/// stands out.
// A slightly blue-shifted near-black, so the dimmed map keeps the cool cast of
// the app's dark surfaces. The exact channels are not load-bearing.
pub(crate) const FOCUS_SCRIM_COLOR: egui::Color32 = egui::Color32::from_rgb(15, 17, 20);

/// Peak opacity of the focus scrim, in light and dark themes respectively.
///
/// The scrim unavoidably covers the map tiles as well as the non-focused
/// tracks it means to dim, so both stay gentle: enough to push the non-focused
/// geometry back while keeping the map legible. Light mode is lower, since a
/// dark scrim reads as heavier over a light map than over a dark one at equal
/// opacity.
pub(crate) const FOCUS_SCRIM_MAX_ALPHA_LIGHT: f32 = 0.22;
pub(crate) const FOCUS_SCRIM_MAX_ALPHA_DARK: f32 = 0.3;

/// Stroke for a track's plain line: thicker highlight blue when the track
/// is hovered or sticky-selected, its palette color at full opacity otherwise.
///
/// Dimming of non-focused tracks is handled by the fade overlay in
/// [`crate::track_layers`], not by modifying the stroke color here.
pub(crate) fn track_stroke(highlight: &MapHighlight, fi: FileIdx, ti: TrackIdx) -> Stroke {
    if is_trip_highlighted(highlight, fi, ti) {
        Stroke::new(4.0_f32, HIGHLIGHT_BLUE)
    } else {
        Stroke::new(3.0_f32, track_color(fi.as_usize(), ti.as_usize()))
    }
}

fn is_trip_highlighted(highlight: &MapHighlight, fi: FileIdx, ti: TrackIdx) -> bool {
    let track = TrackRef::new(fi, ti);
    if highlight.sticky.is_some_and(|r| r.track == track) {
        return true;
    }
    match highlight.hover {
        Some(HighlightScope::File { file_index }) => file_index == fi,
        Some(HighlightScope::Track(t)) => t == track,
        Some(HighlightScope::TrackCategory { track: t, category }) => {
            t == track && matches!(category, DataCategory::Track | DataCategory::Tpv)
        }
        Some(HighlightScope::Point(_)) | None => false,
    }
}

/// Returns the alpha multiplier to use when painting this track's elements.
///
/// Returns `1.0` when no hover is active or when this track is in focus.
/// Returns [`HOVER_FADE_ALPHA`] for every other track while a hover is active,
/// so the focused track stands out and all others are almost hidden.
///
/// Two hover sources are considered:
/// - `highlight.hover`: a map pointer hover (any [`HighlightScope`]).
/// - `highlight.plot_hover_point`: the plot cursor snapping to a TPV point.
pub(crate) fn track_fade_alpha(highlight: &MapHighlight, fi: FileIdx, ti: TrackIdx) -> f32 {
    if !highlight.fading_enabled || !hover_is_active(highlight) {
        return 1.0;
    }
    if is_track_in_focus(highlight, fi, ti) {
        return 1.0;
    }
    HOVER_FADE_ALPHA
}

/// Apply a hover-fade by scaling the color's alpha channel rather than its RGB
/// values, so the element fades to transparent against the map tiles instead
/// of darkening toward black.
///
/// `fade` is expected to be in `[0.0, 1.0]`. Values outside that range are
/// clamped.
pub(crate) fn apply_fade_alpha(color: Color32, fade: f32) -> Color32 {
    #[expect(
        clippy::cast_sign_loss,
        reason = "fade is clamped to [0, 1] so the product is non-negative"
    )]
    let a = ((color.a() as f32) * fade.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

/// Returns `true` when any hover source is currently active, meaning non-focused
/// tracks should be dimmed.
///
/// For map hover (`highlight.hover`) any active scope qualifies.
/// For plot hover only [`MapHighlight::plot_hover_snapped`] qualifies. The
/// cursor must be within snap-distance of an actual data point so that moving
/// the cursor into the plot area does not immediately trigger the overlay.
pub(crate) fn hover_is_active(highlight: &MapHighlight) -> bool {
    highlight.hover.is_some() || highlight.plot_hover_snapped
}

/// Returns `true` when the given track is the focus of the current hover.
pub(crate) fn is_track_in_focus(highlight: &MapHighlight, fi: FileIdx, ti: TrackIdx) -> bool {
    let track = TrackRef::new(fi, ti);
    let from_map_hover = match highlight.hover {
        Some(HighlightScope::File { file_index }) => file_index == fi,
        Some(HighlightScope::Track(t)) | Some(HighlightScope::TrackCategory { track: t, .. }) => {
            t == track
        }
        Some(HighlightScope::Point(r)) => r.track == track,
        None => false,
    };
    from_map_hover || highlight.snapped_plot_hover_track() == Some(track)
}

/// Returns the single [`TrackRef`] currently in focus, or `None` when no
/// specific track has focus (hover inactive, or a file-level scope).
///
/// Used by [`crate::NavMap`] to detect when the focused track changes and to
/// drive the hysteresis/animation logic in `HoverFadeState`.
pub(crate) fn focused_track_from_highlight(highlight: &MapHighlight) -> Option<TrackRef> {
    match highlight.hover {
        Some(HighlightScope::Track(t)) | Some(HighlightScope::TrackCategory { track: t, .. }) => {
            Some(t)
        }
        Some(HighlightScope::Point(r)) => Some(r.track),
        Some(HighlightScope::File { .. }) | None => highlight.snapped_plot_hover_track(),
    }
}

/// The bright pulsing overlay stroke for newly loaded tracks.
pub(crate) fn blink_stroke(blink_alpha: f32) -> Stroke {
    #[expect(
        clippy::cast_sign_loss,
        reason = "blink_alpha is clamped to [0,1] in NavMap::draw so product is non-negative"
    )]
    let blink_a = (blink_alpha * 200.0) as u8;
    Stroke::new(
        6.0_f32,
        Color32::from_rgba_unmultiplied(255, 230, 80, blink_a),
    )
}

/// Draw a track polyline where ghost-fix edges (either endpoint has `heading == None`)
/// are rendered as dashed lines and real edges as solid lines.
///
/// An edge is ghost when either endpoint is a ghost fix, so the dashed region
/// extends one segment on each side of every ghost point - ensuring the
/// visual uncertainty is clear even at the real→ghost boundary.
pub(crate) fn draw_track_with_ghost<K: Copy>(
    painter: &egui::Painter,
    pts: &[(K, egui::Pos2)],
    stroke: Stroke,
    is_ghost: impl Fn(K) -> bool,
) {
    if pts.len() < 2 {
        return;
    }

    let mut solid_span: Vec<egui::Pos2> = Vec::new();
    let mut ghost_span: Vec<egui::Pos2> = Vec::new();

    for w in pts.windows(2) {
        let [(key_a, pos_a), (key_b, pos_b)] = w else {
            continue;
        };
        let (pos_a, pos_b) = (*pos_a, *pos_b);
        let edge_is_ghost = is_ghost(*key_a) || is_ghost(*key_b);

        if edge_is_ghost {
            if solid_span.len() >= 2 {
                painter.add(egui::Shape::line(std::mem::take(&mut solid_span), stroke));
            } else {
                solid_span.clear();
            }
            if ghost_span.is_empty() {
                ghost_span.push(pos_a);
            }
            ghost_span.push(pos_b);
        } else {
            if ghost_span.len() >= 2 {
                draw_dashed_line(painter, &ghost_span, stroke, GHOST_FIX_DASH);
            }
            ghost_span.clear();
            if solid_span.is_empty() {
                solid_span.push(pos_a);
            }
            solid_span.push(pos_b);
        }
    }

    if solid_span.len() >= 2 {
        painter.add(egui::Shape::line(solid_span, stroke));
    }
    if ghost_span.len() >= 2 {
        draw_dashed_line(painter, &ghost_span, stroke, GHOST_FIX_DASH);
    }
}

/// Dash and gap lengths of a dashed line, in screen pixels.
#[derive(Clone, Copy)]
pub(crate) struct DashPattern {
    pub(crate) dash_px: f32,
    pub(crate) gap_px: f32,
}

pub(crate) fn draw_dashed_line(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    stroke: Stroke,
    DashPattern { dash_px, gap_px }: DashPattern,
) {
    if points.len() < 2 {
        return;
    }
    let period = dash_px + gap_px;
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
            let in_dash = phase < dash_px;
            let phase_end = if in_dash { dash_px } else { period };
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
                    phase = dash_px;
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

/// True when this track's trackline should not paint at all: the fix icons
/// are fully faded, so the quality line paints exactly over this track's
/// geometry and the plain trackline (highlight stroke included) would be
/// entirely occluded. `fade` is `None` when the TPV layer is hidden - then
/// no quality line exists and the trackline must stay. The blink overlay
/// draws on top of everything and still needs the pass.
pub(crate) fn skip_trackline(
    fade: Option<crate::tpv_renderer::TrackIconFade>,
    need_blink: bool,
) -> bool {
    fade == Some(crate::tpv_renderer::TrackIconFade::AllHidden) && !need_blink
}

#[cfg(test)]
mod tests {
    use super::skip_trackline;
    use crate::tpv_renderer::TrackIconFade;

    #[test]
    fn trackline_is_replaced_only_when_the_quality_line_covers_it() {
        // Fully faded icons with the TPV layer on: the quality line paints
        // over the trackline, so the pass is skipped.
        assert!(skip_trackline(Some(TrackIconFade::AllHidden), false));
        // TPV layer hidden: no quality line exists, the trackline must stay.
        assert!(!skip_trackline(None, false));
        // Icons partially or fully visible: the quality line is transparent
        // or absent along opaque stretches, the trackline must stay.
        assert!(!skip_trackline(Some(TrackIconFade::PerFix), false));
        assert!(!skip_trackline(Some(TrackIconFade::AllVisible), false));
        // A blinking (newly loaded) track draws its overlay in this pass.
        assert!(!skip_trackline(Some(TrackIconFade::AllHidden), true));
    }
}
