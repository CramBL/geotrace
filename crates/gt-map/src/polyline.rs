//! Screen-space polyline reduction shared by the track and TPV renderers.
//!
//! egui packs every frame into a single vertex buffer, so unbounded per-point
//! tessellation of large recordings can exceed wgpu's maximum buffer size and
//! abort the process. The helpers here bound the tessellated vertex count by
//! what is actually visible: off-screen stretches are culled and sub-pixel
//! detail is merged.

/// Screen-space margin added around the viewport when culling polyline
/// segments, so strokes and feathering right at the edge are not visibly
/// clipped.
pub(crate) const CULL_MARGIN_PX: f32 = 8.0;

/// Minimum squared screen-space distance between kept polyline points.
/// Consecutive points with the same key closer than this are merged, bounding
/// the tessellated vertex count by the on-screen path length.
pub(crate) const MIN_POINT_DIST_SQ: f32 = 1.0;

/// Maximum screen-space error allowed when substituting a precomputed track
/// LOD level for the full point list. Kept below the sub-pixel merge
/// threshold ([`MIN_POINT_DIST_SQ`]) so LOD-fed rendering stays visually
/// identical to runtime decimation of the full recording.
pub(crate) const MAX_LOD_ERROR_PX: f32 = 0.75;

/// The on-screen drawable form of a path, as computed by [`visible_path`].
///
/// Matching on this is what guarantees a track can never silently vanish
/// while part of it is on screen: the collapsed case is a variant the
/// compiler forces every caller to handle.
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(crate) enum VisiblePath<K> {
    /// No part of the path lies inside the cull rect.
    OffScreen,
    /// The path's entire on-screen extent packs below one pixel (extreme
    /// zoom-out, or a single fix). Draw a dot at this point so the path
    /// stays discoverable.
    Dot(K, egui::Pos2),
    /// Drawable polyline spans. Every span has at least two points.
    Spans(PolylineSpans<K>),
}

/// Drawable polyline spans stored as contiguous slices of one flat buffer.
///
/// `span_ends` holds the exclusive end index of each span, so a whole frame's
/// track geometry costs a single heap allocation (the boundary list stays
/// inline for the common case of a handful of spans).
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(crate) struct PolylineSpans<K> {
    points: Vec<(K, egui::Pos2)>,
    span_ends: smallvec::SmallVec<[usize; 8]>,
}

impl<K> PolylineSpans<K> {
    /// Iterate the spans as slices of the flat buffer.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &[(K, egui::Pos2)]> {
        let mut start = 0;
        self.span_ends.iter().filter_map(move |&end| {
            let span = self.points.get(start..end);
            start = end;
            span
        })
    }

    /// Build from per-span point lists. Test-only convenience for asserting
    /// expected span structure.
    #[cfg(test)]
    pub(crate) fn from_nested(nested: Vec<Vec<(K, egui::Pos2)>>) -> Self {
        let mut spans = Self {
            points: Vec::new(),
            span_ends: smallvec::SmallVec::new(),
        };
        for span in nested {
            spans.points.extend(span);
            spans.span_ends.push(spans.points.len());
        }
        spans
    }
}

/// Reduce a track's screen-space points to their on-screen drawable form.
///
/// Each point carries a key `K` (e.g. ghost/real flag, fix-quality color)
/// that downstream drawing uses to style individual edges.
///
/// Two reductions keep the tessellated vertex count proportional to what is
/// actually visible:
///
/// - Segments that provably cannot intersect `cull_rect` (both endpoints
///   beyond the same rect edge) end the current span. The polyline resumes
///   when it re-enters. Partially visible segments are kept whole, so no
///   endpoint is ever moved and visible geometry is exact.
/// - Consecutive points with equal keys closer than [`MIN_POINT_DIST_SQ`]
///   are merged - at sub-pixel distance the difference is invisible. A key
///   transition is always kept so styled regions stay anchored.
///
/// Spans with fewer than two points (nothing to draw) are dropped. When that
/// leaves nothing but at least one point is on screen, the result is
/// [`VisiblePath::Dot`] instead.
pub(crate) fn visible_path<K: Copy + PartialEq>(
    points: impl Iterator<Item = (K, egui::Pos2)>,
    cull_rect: egui::Rect,
) -> VisiblePath<K> {
    let mut spans = PolylineSpans::<K> {
        points: Vec::new(),
        span_ends: smallvec::SmallVec::new(),
    };
    // Start of the span currently being built, as an index into the flat
    // buffer. Everything before it belongs to completed spans.
    let mut span_start = 0;
    let mut prev: Option<(K, egui::Pos2)> = None;
    let mut first_on_screen: Option<(K, egui::Pos2)> = None;

    for cur in points {
        if first_on_screen.is_none() && cull_rect.contains(cur.1) {
            first_on_screen = Some(cur);
        }
        if let Some(prev_pt) = prev {
            if segment_outside(prev_pt.1, cur.1, cull_rect) {
                if spans.points.len() - span_start >= 2 {
                    spans.span_ends.push(spans.points.len());
                } else {
                    spans.points.truncate(span_start);
                }
                span_start = spans.points.len();
            } else {
                if spans.points.len() == span_start {
                    spans.points.push(prev_pt);
                }
                if let Some(&(last_key, last_pos)) = spans.points.last()
                    && (cur.0 != last_key || (cur.1 - last_pos).length_sq() >= MIN_POINT_DIST_SQ)
                {
                    spans.points.push(cur);
                }
            }
        }
        prev = Some(cur);
    }
    if spans.points.len() - span_start >= 2 {
        spans.span_ends.push(spans.points.len());
    } else {
        spans.points.truncate(span_start);
    }
    if spans.span_ends.is_empty() {
        return match first_on_screen {
            Some((key, pos)) => VisiblePath::Dot(key, pos),
            None => VisiblePath::OffScreen,
        };
    }
    VisiblePath::Spans(spans)
}

/// True when the segment a-b provably cannot intersect `rect`: both endpoints
/// lie beyond the same edge. Conservative - segments passing diagonally near
/// a corner are kept even when they miss the rect.
pub(crate) fn segment_outside(a: egui::Pos2, b: egui::Pos2, rect: egui::Rect) -> bool {
    (a.x < rect.min.x && b.x < rect.min.x)
        || (a.x > rect.max.x && b.x > rect.max.x)
        || (a.y < rect.min.y && b.y < rect.min.y)
        || (a.y > rect.max.y && b.y > rect.max.y)
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, pos2};

    use super::{PolylineSpans, VisiblePath, segment_outside, visible_path};

    fn spans<K: Copy + PartialEq>(nested: Vec<Vec<(K, Pos2)>>) -> VisiblePath<K> {
        VisiblePath::Spans(PolylineSpans::from_nested(nested))
    }

    const RECT: Rect = Rect {
        min: pos2(0.0, 0.0),
        max: pos2(100.0, 100.0),
    };

    fn real(x: f32, y: f32) -> (bool, Pos2) {
        (false, pos2(x, y))
    }

    fn ghost(x: f32, y: f32) -> (bool, Pos2) {
        (true, pos2(x, y))
    }

    #[test]
    fn spaced_points_inside_view_form_a_single_full_span() {
        let pts = vec![real(10.0, 10.0), real(20.0, 10.0), real(30.0, 10.0)];
        let path = visible_path(pts.clone().into_iter(), RECT);
        assert_eq!(path, spans(vec![pts]));
    }

    #[test]
    fn sub_pixel_points_are_merged() {
        let pts = vec![
            real(10.0, 10.0),
            real(10.2, 10.0),
            real(10.4, 10.0),
            real(20.0, 10.0),
        ];
        let path = visible_path(pts.into_iter(), RECT);
        assert_eq!(path, spans(vec![vec![real(10.0, 10.0), real(20.0, 10.0)]]));
    }

    #[test]
    fn key_transition_is_kept_even_at_sub_pixel_distance() {
        let pts = vec![real(10.0, 10.0), ghost(10.2, 10.0), real(10.4, 10.0)];
        let path = visible_path(pts.clone().into_iter(), RECT);
        assert_eq!(path, spans(vec![pts]));
    }

    #[test]
    fn color_keys_split_spans_on_quality_change() {
        use egui::Color32;
        let blue = Color32::BLUE;
        let yellow = Color32::YELLOW;
        let pts = vec![
            (blue, pos2(10.0, 10.0)),
            (blue, pos2(20.0, 10.0)),
            (yellow, pos2(30.0, 10.0)),
            (yellow, pos2(40.0, 10.0)),
        ];
        let path = visible_path(pts.clone().into_iter(), RECT);
        assert_eq!(path, spans(vec![pts]));
    }

    #[test]
    fn fully_off_screen_path_is_off_screen() {
        // All points are above the rect, so every segment is trivially
        // outside - even though the path spans the rect horizontally.
        let pts = vec![real(-50.0, -50.0), real(150.0, -60.0), real(-50.0, -70.0)];
        let path = visible_path(pts.into_iter(), RECT);
        assert_eq!(path, VisiblePath::OffScreen);
    }

    #[test]
    fn partially_visible_segments_keep_exact_endpoints() {
        // In -> far out -> back in: the crossing segments stay (one endpoint
        // is inside), only the fully-outside middle segment breaks the span.
        let pts = vec![
            real(50.0, 50.0),
            real(50.0, -500.0),
            real(60.0, -500.0),
            real(60.0, 50.0),
        ];
        let path = visible_path(pts.into_iter(), RECT);
        assert_eq!(
            path,
            spans(vec![
                vec![real(50.0, 50.0), real(50.0, -500.0)],
                vec![real(60.0, -500.0), real(60.0, 50.0)],
            ])
        );
    }

    #[test]
    fn single_visible_point_becomes_a_dot() {
        assert_eq!(
            visible_path(std::iter::empty::<(bool, Pos2)>(), RECT),
            VisiblePath::OffScreen
        );
        // A lone fix inside the view must stay discoverable (drawn as a dot
        // by the caller). A lone fix outside the view yields nothing.
        assert_eq!(
            visible_path(std::iter::once(real(10.0, 10.0)), RECT),
            VisiblePath::Dot(false, pos2(10.0, 10.0))
        );
        assert_eq!(
            visible_path(std::iter::once(real(-10.0, 10.0)), RECT),
            VisiblePath::OffScreen
        );
    }

    #[test]
    fn collapsed_sub_pixel_path_becomes_a_dot() {
        // A whole track merging below one pixel (extreme zoom-out) must not
        // vanish: the first visible point comes back as a dot.
        let pts = vec![real(10.0, 10.0), real(10.2, 10.0), real(10.4, 10.0)];
        let path = visible_path(pts.into_iter(), RECT);
        assert_eq!(path, VisiblePath::Dot(false, pos2(10.0, 10.0)));
    }

    #[test]
    fn nan_coordinates_are_dropped_by_the_merge_step() {
        // Every comparison against NaN is false: the same-side cull never
        // rejects a NaN segment, and the keep-condition of the sub-pixel
        // merge (distance >= threshold) never holds either, so a same-key
        // NaN point is silently merged away before reaching the painter.
        // Upstream projection should not produce NaN.
        let pts = vec![
            real(10.0, 10.0),
            (false, pos2(f32::NAN, 10.0)),
            real(20.0, 10.0),
        ];
        let path = visible_path(pts.into_iter(), RECT);
        assert_eq!(path, spans(vec![vec![real(10.0, 10.0), real(20.0, 10.0)]]));
    }

    proptest::proptest! {
        /// `visible_path` must never panic, and a `Spans` result must only
        /// contain drawable spans (at least two points each, at least one
        /// span) - for arbitrary point streams including coordinates far
        /// outside the viewport, infinities, and degenerate (zero-size or
        /// inverted) cull rects. The collapsed and off-screen cases are
        /// their own `VisiblePath` variants, so they need no length invariant.
        #[test]
        fn visible_path_never_panics_and_spans_are_drawable(
            pts in proptest::collection::vec(
                (proptest::bool::ANY, proptest::num::f32::ANY, proptest::num::f32::ANY),
                0..50,
            ),
            rect_min in (proptest::num::f32::ANY, proptest::num::f32::ANY),
            rect_max in (proptest::num::f32::ANY, proptest::num::f32::ANY),
        ) {
            // Constructed directly (not via `from_two_pos`) so inverted and
            // NaN rects are exercised too.
            let rect = Rect {
                min: pos2(rect_min.0, rect_min.1),
                max: pos2(rect_max.0, rect_max.1),
            };
            let points = pts.iter().map(|&(key, x, y)| (key, pos2(x, y)));
            if let VisiblePath::Spans(spans) = visible_path(points, rect) {
                proptest::prop_assert!(spans.iter().next().is_some());
                proptest::prop_assert!(spans.iter().all(|span| span.len() >= 2));
            }
        }
    }

    #[test]
    fn segment_outside_rejects_only_same_side_pairs() {
        // Both beyond the same edge: rejected.
        assert!(segment_outside(pos2(-10.0, 50.0), pos2(-5.0, 60.0), RECT));
        assert!(segment_outside(pos2(50.0, 110.0), pos2(60.0, 200.0), RECT));
        // Crossing the rect: kept.
        assert!(!segment_outside(pos2(-10.0, 50.0), pos2(110.0, 50.0), RECT));
        // Inside: kept.
        assert!(!segment_outside(pos2(10.0, 10.0), pos2(20.0, 20.0), RECT));
        // Diagonal near a corner (conservatively kept even though it misses).
        assert!(!segment_outside(pos2(-10.0, 50.0), pos2(50.0, -10.0), RECT));
    }
}
