use std::ops::RangeInclusive;

use egui::{Color32, Stroke};
use gt_types::{DataCategory, FileIdx, TrackIdx, TrackRef};
use gt_ui_theme::HIGHLIGHT_BLUE;
use gt_ui_types::{HighlightScope, MapHighlight};

/// Stroke for a track's plain line: thicker highlight blue when the track
/// is hovered or sticky-selected, its palette color at full opacity otherwise.
///
/// Dimming of non-focused tracks is handled by the fade overlay in
/// [`crate::track_layers`], not by modifying the stroke color here.
pub(crate) fn track_stroke(highlight: &MapHighlight, fi: FileIdx, ti: TrackIdx) -> Stroke {
    if is_track_highlighted(highlight, fi, ti) {
        Stroke::new(4.0_f32, HIGHLIGHT_BLUE)
    } else {
        Stroke::new(
            3.0_f32,
            gt_ui_theme::track_color(fi.as_usize(), ti.as_usize()),
        )
    }
}

/// Whether the map draws `ti` as the highlighted track: the pointer is on it
/// or on its recording, or it is the sticky selection's track.
pub(crate) fn is_track_highlighted(highlight: &MapHighlight, fi: FileIdx, ti: TrackIdx) -> bool {
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

/// Apply a hover-fade by scaling the color's alpha channel, so the element
/// fades to transparent against the map tiles.
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

/// Which portions of the trackline are visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TracklineVisibility {
    pub(crate) solid: bool,
    pub(crate) ghost: bool,
}

/// Strokes for the solid and ghost portions of a trackline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TracklineStrokes {
    pub(crate) solid: Stroke,
    pub(crate) ghost: Stroke,
}

/// Draw a track polyline with separate strokes and visibility for solid and ghost-fix edges.
///
/// An edge is ghost when either endpoint is a ghost fix: the dashed region
/// extends one segment on each side of each ghost point, marking the
/// transition into dead reckoning.
pub(crate) fn draw_track_with_ghost<K: Copy>(
    painter: &egui::Painter,
    pts: &[(K, egui::Pos2)],
    strokes: TracklineStrokes,
    visibility: TracklineVisibility,
    is_ghost: impl Fn(K) -> bool,
) {
    if pts.len() < 2 {
        return;
    }

    let paint_solid_run = |run: RangeInclusive<usize>| {
        if visibility.solid
            && let Some(run_points) = pts.get(run)
        {
            painter.add(egui::Shape::line(
                run_points.iter().map(|&(_, pos)| pos).collect(),
                strokes.solid,
            ));
        }
    };

    let mut solid_run: Option<RangeInclusive<usize>> = None;
    let mut ghost_span: Vec<egui::Pos2> = Vec::new();

    for (edge, w) in pts.windows(2).enumerate() {
        let [(key_a, pos_a), (key_b, pos_b)] = w else {
            continue;
        };
        let (pos_a, pos_b) = (*pos_a, *pos_b);
        let edge_is_ghost = is_ghost(*key_a) || is_ghost(*key_b);

        if edge_is_ghost {
            if let Some(run) = solid_run.take() {
                paint_solid_run(run);
            }
            if visibility.ghost {
                if ghost_span.is_empty() {
                    ghost_span.push(pos_a);
                }
                ghost_span.push(pos_b);
            }
        } else {
            if visibility.ghost && ghost_span.len() >= 2 {
                draw_dashed_line(painter, &ghost_span, strokes.ghost, GHOST_FIX_DASH);
            }
            ghost_span.clear();
            let run_start = solid_run.as_ref().map_or(edge, |run| *run.start());
            solid_run = Some(run_start..=edge + 1);
        }
    }

    if let Some(run) = solid_run {
        paint_solid_run(run);
    }
    if visibility.ghost && ghost_span.len() >= 2 {
        draw_dashed_line(painter, &ghost_span, strokes.ghost, GHOST_FIX_DASH);
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

/// True when the quality line covers the entire solid trackline and no blink overlay is active.
/// Ghost stretches are not covered by the quality line and paint regardless.
pub(crate) fn skip_solid_trackline(
    fade: Option<crate::tpv_renderer::TrackIconFade>,
    need_blink: bool,
) -> bool {
    fade == Some(crate::tpv_renderer::TrackIconFade::AllHidden) && !need_blink
}

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
// the app's dark surfaces. Nothing depends on the exact channels.
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

#[cfg(test)]
mod tests {
    use egui::{Color32, Pos2, Stroke};

    use crate::tpv_renderer::TrackIconFade;

    #[test]
    fn trackline_is_replaced_only_when_the_quality_line_covers_it() {
        // Fully faded icons with the TPV layer on: the quality line paints
        // over the trackline, so the solid pass is skipped.
        assert!(super::skip_solid_trackline(
            Some(TrackIconFade::AllHidden),
            false
        ));
        // TPV layer hidden: no quality line exists, the solid trackline must stay.
        assert!(!super::skip_solid_trackline(None, false));
        // Icons partially or fully visible: the quality line is transparent
        // or absent along opaque stretches, the solid trackline must stay.
        assert!(!super::skip_solid_trackline(
            Some(TrackIconFade::PerFix),
            false
        ));
        assert!(!super::skip_solid_trackline(
            Some(TrackIconFade::AllVisible),
            false
        ));
        // A blinking (newly loaded) track draws its overlay in this pass.
        assert!(!super::skip_solid_trackline(
            Some(TrackIconFade::AllHidden),
            true
        ));
    }

    #[test]
    fn draw_track_with_ghost_respects_solid_and_ghost_visibility() {
        let pts = vec![
            (false, Pos2::new(0.0, 0.0)),
            (false, Pos2::new(10.0, 0.0)),
            (true, Pos2::new(20.0, 0.0)),
            (true, Pos2::new(30.0, 0.0)),
            (false, Pos2::new(40.0, 0.0)),
            (false, Pos2::new(50.0, 0.0)),
        ];
        let solid_stroke = Stroke::new(2.0, Color32::WHITE);
        let ghost_stroke = Stroke::new(2.0, crate::tpv_renderer::FIX_LOST_RED);
        let strokes = super::TracklineStrokes {
            solid: solid_stroke,
            ghost: ghost_stroke,
        };

        let paint_shapes = |visibility| {
            let mut harness = crate::test_util::harness_builder().ui(|ui| {
                super::draw_track_with_ghost(ui.painter(), &pts, strokes, visibility, |is_ghost| {
                    is_ghost
                });
            });
            harness.run();
            harness
                .inner
                .output()
                .shapes
                .iter()
                .filter(|clipped| !matches!(clipped.shape, egui::Shape::Rect(_)))
                .map(|clipped| clipped.shape.clone())
                .collect::<Vec<_>>()
        };

        let has_stroke_color = |shapes: &[egui::Shape], color: Color32| {
            shapes.iter().any(|shape| match shape {
                egui::Shape::LineSegment { stroke: s, .. } => s.color == color,
                egui::Shape::Path(path) => {
                    path.stroke.color == egui::epaint::ColorMode::Solid(color)
                }
                _ => false,
            })
        };

        let solid_only = paint_shapes(super::TracklineVisibility {
            solid: true,
            ghost: false,
        });
        assert_eq!(solid_only.len(), 2);
        assert!(has_stroke_color(&solid_only, solid_stroke.color));
        assert!(!has_stroke_color(&solid_only, ghost_stroke.color));

        let both = paint_shapes(super::TracklineVisibility {
            solid: true,
            ghost: true,
        });
        assert_eq!(both.len(), 5);
        assert!(has_stroke_color(&both, solid_stroke.color));
        assert!(has_stroke_color(&both, ghost_stroke.color));

        let ghost_only = paint_shapes(super::TracklineVisibility {
            solid: false,
            ghost: true,
        });
        assert_eq!(ghost_only.len(), 3);
        assert!(has_stroke_color(&ghost_only, ghost_stroke.color));
        assert!(!has_stroke_color(&ghost_only, solid_stroke.color));

        let neither = paint_shapes(super::TracklineVisibility {
            solid: false,
            ghost: false,
        });
        assert_eq!(neither.len(), 0);
    }
}
