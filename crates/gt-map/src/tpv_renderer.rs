use std::iter;
use std::ops::RangeInclusive;

use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, Pos2, Stroke, Ui, Vec2};
use gt_types::{
    DataCategory, FileIdx, FixRef, LoadedFile, LoadedTrack, MapFixKind, NavPoint, PlacedPoint,
    PlacedPoints, PointIdx, SatelliteQueryScope, SkyGeometry, SolutionState, TrackIdx, TrackRef,
};
use gt_ui_types::{HighlightScope, MapElementRef, MapHighlight};
use uom::si::angle::radian;
use uom::si::f64::Angle;
use uom::si::length::meter;

use crate::icon_mesh::{IconId, IconInstance, IconMeshBatch, IconMeshLibrary};
use crate::text_badge::{BadgePlateHeight, TextBadge};
use crate::transform::MapScale;

pub(crate) use pinned_point::show_sticky_tpv_content;
pub(crate) use point_metrics::{FixPlacement, show_hover_table};
pub(crate) use satellite_tables::constellation_swatch;
pub(crate) use sky_context::SkySection;

mod navigation_details;
mod pinned_point;
mod point_metrics;
mod point_summary;
mod satellite_tables;
mod sky_context;

/// What the icon pass culls a fix against: `map_rect` grown by
/// [`ICON_VIEW_MARGIN_PX`], so an icon whose shape crosses the edge draws.
/// [`crate::viewport::collect_visible_points`] queries the fixes of this rect.
pub(crate) fn icon_cull_rect(map_rect: egui::Rect) -> egui::Rect {
    map_rect.expand(ICON_VIEW_MARGIN_PX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChevronFix {
    CoordinateOutOfRange,
    Estimated,
    NavigationLoss,
    Synthesized { color: Color32 },
    UnreportedPosition { color: Color32 },
}

impl ChevronFix {
    pub(crate) fn for_fix(fix: &NavPoint) -> Option<Self> {
        match fix.map_fix_kind() {
            MapFixKind::Recorded => None,
            MapFixKind::UnreportedPosition => Some(Self::UnreportedPosition {
                color: tpv_point_color(fix),
            }),
            MapFixKind::CoordinateOutOfRange => Some(Self::CoordinateOutOfRange),
            MapFixKind::NavigationLoss => Some(Self::NavigationLoss),
            MapFixKind::Estimated => Some(Self::Estimated),
            MapFixKind::Synthesized => Some(Self::Synthesized {
                color: tpv_point_color(fix),
            }),
        }
    }

    fn tint(self, dark_mode: bool) -> Color32 {
        match self {
            Self::NavigationLoss => FIX_LOST_RED,
            Self::Estimated => FIX_ESTIMATED_PURPLE,
            Self::Synthesized { color } | Self::UnreportedPosition { color } => color,
            Self::CoordinateOutOfRange => gt_ui_theme::warning_amber(dark_mode),
        }
    }
}

fn is_arrow_highlighted(highlight: &MapHighlight, point_ref: MapElementRef) -> bool {
    if highlight.is_sticky(point_ref) {
        return true;
    }
    match highlight.hover {
        Some(HighlightScope::Point(r)) => r == point_ref,
        Some(HighlightScope::Track(track)) => track == point_ref.track(),
        Some(HighlightScope::TrackCategory { track, category }) => {
            track == point_ref.track() && category == DataCategory::Tpv
        }
        _ => false,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "render context requires all parameters; a context struct would not add clarity"
)]
pub(crate) fn draw_track_icons(
    ui: &Ui,
    view_rect: egui::Rect,
    fi: FileIdx,
    ti: TrackIdx,
    track: &LoadedTrack,
    real_fix_indices: Option<&Vec<usize>>,
    chevron_points: &[(usize, ChevronFix)],
    style: &TpvDrawStyle,
    fade: TrackIconFade,
    transform: &crate::transform::MercTransform,
    highlight: &MapHighlight,
    fix_is_present: &impl Fn(usize) -> bool,
    icon_meshes: Option<&IconMeshLibrary>,
) {
    // One batch for the whole track's icons. `draw_navigation_arrow` barriers
    // the batch before it paints a highlighted arrow, so stacking matches
    // immediate painting exactly. See [IconMeshBatch].
    let mut batch = IconMeshBatch::gpu_when_available(ui, icon_meshes);
    // A track with no geometry is drawn nowhere, so it has no icons.
    let Some(placed) = track.placed_points() else {
        return;
    };
    // Real fixes: indices come from the global R-tree viewport query.
    if let Some(indices) = real_fix_indices {
        draw_accuracy_circles(ui, placed, indices, style, fade, transform, fix_is_present);
        for fix in visible_real_fixes(placed, indices, style, fade, transform, fix_is_present) {
            let point_style = TpvDrawStyle {
                icon_alpha: fix.icon_alpha,
                ..*style
            };
            let point_ref = MapElementRef::Fix(FixRef::new(
                TrackRef::new(fi, ti),
                PointIdx::new(fix.point_index),
            ));
            draw_tpv_point(
                ui,
                fix.screen_pos,
                &PointKind::Real {
                    color: tpv_point_color(fix.point.fix),
                    heading: fix.heading,
                },
                is_arrow_highlighted(highlight, point_ref),
                &point_style,
                &mut batch,
            );
        }
    }

    // The caller builds `chevron_points` from the same viewport query as
    // `real_fix_indices`, applying the time window and the query's hiding
    // there.
    for (pi, chevron, point) in chevron_points
        .iter()
        .filter_map(|&(pi, chevron)| Some((pi, chevron, placed.get(pi)?)))
    {
        let screen_pos = transform.to_screen(point.merc());
        if !view_rect.contains(screen_pos) {
            continue;
        }
        let icon_alpha = fix_icon_alpha(
            fade,
            placed,
            pi,
            screen_pos,
            style.base_arrow_size,
            transform,
        );
        if icon_alpha <= 0.0 {
            continue;
        }
        let point_style = TpvDrawStyle {
            icon_alpha,
            ..*style
        };
        // Direction for the chevron:
        // - If the GPS reported a heading (fix-lost but device maintained estimate),
        //   use it - it is more accurate than deriving from neighbour positions.
        // - Otherwise derive from neighbouring Mercator positions (see
        //   [`chevron_direction`]).
        let direction = if let Some(h) = point.fix.tpv.heading() {
            let angle_rad = h.get::<radian>() - std::f64::consts::FRAC_PI_2;
            egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32)
        } else {
            let merc_prev = pi
                .checked_sub(1)
                .and_then(|i| placed.get(i))
                .map_or(point.merc(), |p| p.merc());
            let merc_next = placed.get(pi + 1).map_or(point.merc(), |p| p.merc());
            chevron_direction(merc_prev, merc_next)
        };
        let point_ref = MapElementRef::Fix(FixRef::new(TrackRef::new(fi, ti), PointIdx::new(pi)));
        draw_tpv_point(
            ui,
            screen_pos,
            &PointKind::Chevron {
                direction,
                fix: chevron,
            },
            is_arrow_highlighted(highlight, point_ref),
            &point_style,
            &mut batch,
        );
    }
    batch.paint(ui.painter());
}

struct VisibleRealFix<'a> {
    point_index: usize,
    point: PlacedPoint<'a>,
    heading: Option<Angle>,
    screen_pos: Pos2,
    icon_alpha: f32,
}

fn visible_real_fixes<'a>(
    placed: PlacedPoints<'a>,
    real_fix_indices: &'a [usize],
    style: &'a TpvDrawStyle,
    fade: TrackIconFade,
    transform: &'a crate::transform::MercTransform,
    fix_is_present: &'a dyn Fn(usize) -> bool,
) -> impl Iterator<Item = VisibleRealFix<'a>> + 'a {
    real_fix_indices.iter().filter_map(move |&point_index| {
        let point = placed.get(point_index)?;
        if !fix_is_present(point_index) {
            return None;
        }
        // Dead-reckoned fixes and fixes with a coordinate out of range are
        // drawn by the chevron loop.
        if ChevronFix::for_fix(point.fix).is_some() {
            return None;
        }
        let heading = point.fix.tpv.heading();
        let screen_pos = transform.to_screen(point.merc());
        let icon_alpha = fix_icon_alpha(
            fade,
            placed,
            point_index,
            screen_pos,
            style.base_arrow_size,
            transform,
        );
        (icon_alpha > 0.0).then_some(VisibleRealFix {
            point_index,
            point,
            heading,
            screen_pos,
            icon_alpha,
        })
    })
}

/// Paint the horizontal-accuracy circle of every visible real fix that has
/// one, below the whole track's icons.
///
/// A painter primitive between the icons flushes the icon batch, which emits
/// one mesh per flush.
fn draw_accuracy_circles(
    ui: &Ui,
    placed: PlacedPoints<'_>,
    real_fix_indices: &[usize],
    style: &TpvDrawStyle,
    fade: TrackIconFade,
    transform: &crate::transform::MercTransform,
    fix_is_present: &impl Fn(usize) -> bool,
) {
    let min_visible_radius = (style.base_arrow_size * ACCURACY_CIRCLE_MIN_VISIBLE_FACTOR)
        .max(MIN_ACCURACY_CIRCLE_RADIUS_PX);
    for fix in visible_real_fixes(
        placed,
        real_fix_indices,
        style,
        fade,
        transform,
        fix_is_present,
    ) {
        let Some(eph_m) = fix.point.fix.tpv.eph_m() else {
            continue;
        };
        let (latitude, _) = fix.point.resolved_position();
        let radius = (f64::from(eph_m) * transform.pixels_per_meter(latitude)) as f32;
        if radius < min_visible_radius {
            continue;
        }
        ui.painter().circle(
            fix.screen_pos,
            radius,
            ACCURACY_CIRCLE_FILL.gamma_multiply(fix.icon_alpha),
            Stroke::new(
                ACCURACY_CIRCLE_STROKE_WIDTH_PX,
                ACCURACY_CIRCLE_STROKE.gamma_multiply(fix.icon_alpha),
            ),
        );
    }
}

/// Zoom-derived visual parameters computed once per frame and shared
/// across all tracks: icon sizes scale with zoom (see [`base_arrow_size`])
/// and the outline alpha fades out below zoom 14 so dense clusters don't
/// blend into a white mass.
pub(crate) fn frame_style(zoom: f64) -> TpvDrawStyle {
    TpvDrawStyle {
        base_arrow_size: base_arrow_size(zoom),
        outline_alpha: ((zoom - 10.0) / 4.0).clamp(0.0, 1.0) as f32,
        icon_alpha: 1.0,
    }
}

/// The satellite-label collision-cell size in pixels: larger when zoomed out
/// so the label count doesn't explode at dense clusters (180 px at low zoom,
/// 60 px at high). Shared by [`frame_style`] and the map's zoom-debounced
/// label decimation.
pub(crate) fn label_cell_px(zoom: f64) -> f32 {
    60.0 + (1.0 - zoom_size_factor(zoom)) * 120.0
}

/// Cross-highlight: when the track plot cursor is active, draw a ring around
/// the pre-computed closest point, plus a sky disc for its report so
/// scrubbing the plot walks the sky along the track. The app layer computes
/// the point via `find_closest_tpv` and stores it in
/// `MapHighlight::plot_hover_point` - no O(n) scan needed here.
pub(crate) fn draw_plot_hover_overlay(
    ui: &Ui,
    files: &[LoadedFile],
    highlight: &MapHighlight,
    style: &TpvDrawStyle,
    transform: &crate::transform::MercTransform,
) {
    if let Some((fi, ti, pi)) = highlight.plot_hover_point
        && let Some(point) = fi
            .get(files)
            .and_then(|f| ti.get(&f.tracks))
            .and_then(|t| t.placed_points()?.get(pi.as_usize()))
    {
        let pos = transform.to_screen(point.merc());
        let painter = ui.painter();
        painter.circle_stroke(
            pos,
            style.base_arrow_size + 6.0,
            egui::Stroke::new(
                2.0_f32,
                egui::Color32::from_rgba_unmultiplied(100, 200, 255, 230),
            ),
        );
        painter.circle_stroke(
            pos,
            style.base_arrow_size + 3.0,
            egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_unmultiplied(100, 200, 255, 120),
            ),
        );
        // The detailed disc at the scrubbed point, independent of the sky
        // glyphs overlay's own variant or visibility.
        if point.fix.supports_sky_geometry(SkyGeometry::Position) {
            crate::sky_glyph_renderer::draw_hover_disc(ui, pos, point.fix, glyph_size_scale(style));
        }
    }
}

/// Color for the "fix used" count in the satellite badge, legible on the
/// current theme. Pass `ui.visuals().dark_mode`.
pub(crate) fn fix_count_color(count: u32, dark_mode: bool) -> Color32 {
    gt_ui_theme::fix_count_tier(count).color(dark_mode)
}

/// Color for the "total seen" count in the satellite badge, legible on the
/// current theme. Pass `ui.visuals().dark_mode`.
pub(crate) fn seen_count_color(count: u32, dark_mode: bool) -> Color32 {
    gt_ui_theme::seen_count_tier(count).color(dark_mode)
}

/// Zoom-derived visual parameters computed once per frame and shared across
/// all points in all tracks.
#[derive(Clone, Copy)]
pub(crate) struct TpvDrawStyle {
    pub(crate) outline_alpha: f32,
    pub(crate) base_arrow_size: f32,
    /// Opacity of the fix icon currently being drawn, in (0.0, 1.0].
    /// Determined per fix from its local on-screen spacing (see
    /// [`fix_icon_alpha`]). Below 1.0 the icon is crossfading into the
    /// continuous quality line, and fully transparent icons are skipped
    /// before drawing.
    pub(crate) icon_alpha: f32,
}

/// Zoom interpolation factor for icon sizing: 0.0 at
/// [`ICON_MIN_SIZE_ZOOM`] and below, 1.0 at [`ICON_MAX_SIZE_ZOOM`] and
/// above, linear in between.
fn zoom_size_factor(zoom: f64) -> f32 {
    ((zoom - ICON_MIN_SIZE_ZOOM) / (ICON_MAX_SIZE_ZOOM - ICON_MIN_SIZE_ZOOM)).clamp(0.0, 1.0) as f32
}

/// Zoom-scaled base size of the per-fix icons. Shared with viewport
/// collection so fade is classified with the renderer's exact size.
pub(crate) fn base_arrow_size(zoom: f64) -> f32 {
    MIN_ARROW_SIZE_PX + zoom_size_factor(zoom) * (MAX_ARROW_SIZE_PX - MIN_ARROW_SIZE_PX)
}

/// Zoom-adaptive size scale for overlay glyphs: 1.0 where the fix icons
/// reach full size, shrinking to the icons' minimum fraction below that, so
/// glyphs shrink in step with the heading arrows as the track shrinks.
pub(crate) fn glyph_size_scale(style: &TpvDrawStyle) -> f32 {
    style.base_arrow_size / MAX_ARROW_SIZE_PX
}

/// How a track's fix icons fade this frame, determined O(1) from the track's
/// precomputed segment-length range before any per-point work.
///
/// At a given map scale every fix's local spacing lies between the track's
/// shortest and longest segment in pixels, so when the whole range falls on
/// one side of the fade band the per-fix computation can be skipped: a dense
/// zoomed-out track costs nothing per point, and a well-spaced zoomed-in
/// track skips the quality line entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackIconFade {
    /// Even the longest segment is below the fade-out spacing: every icon
    /// is fully transparent and the quality line stands in alone.
    AllHidden,
    /// Even the shortest segment is at or above the fade-in spacing: every
    /// icon is fully opaque and the quality line is not needed.
    AllVisible,
    /// Spacing crosses the fade band (e.g. parked phases on an otherwise
    /// moving track): opacity is determined per fix from its local spacing.
    PerFix,
}

/// Classify how `track`'s icons fade at the current map scale.
///
/// A track without segments (a lone fix) has nothing to overlap with and is
/// always [`TrackIconFade::AllVisible`] - a spacing of zero would hide it at
/// every zoom. So is a track with no geometry, whose icons are never drawn.
pub(crate) fn classify_icon_fade(
    track: &LoadedTrack,
    scale: MapScale,
    icon_size_px: f32,
) -> TrackIconFade {
    let Some(geometry) = track.geometry.measured() else {
        return TrackIconFade::AllVisible;
    };
    let Some(range) = geometry.segment_length_range else {
        return TrackIconFade::AllVisible;
    };
    let ppm = scale.pixels_per_meter(geometry.bounding_box.lat.center());
    let min_px = (range.min.get::<meter>() * ppm) as f32;
    let max_px = (range.max.get::<meter>() * ppm) as f32;
    let (lo, hi) = fade_band(icon_size_px);
    if max_px < lo {
        TrackIconFade::AllHidden
    } else if min_px >= hi {
        TrackIconFade::AllVisible
    } else {
        TrackIconFade::PerFix
    }
}

/// The fade band for a given icon size: local spacings at or below the
/// first bound render fully transparent icons, at or above the second fully
/// opaque ones. Proportional to the icon size, floored in absolute pixels
/// (see [`ICON_FADE_LO_MIN_SPACING_PX`]).
fn fade_band(icon_size_px: f32) -> (f32, f32) {
    (
        (icon_size_px * ICON_FADE_LO_SPACING_FACTOR).max(ICON_FADE_LO_MIN_SPACING_PX),
        (icon_size_px * ICON_FADE_HI_SPACING_FACTOR).max(ICON_FADE_HI_MIN_SPACING_PX),
    )
}

/// Local on-screen spacing of the fix at `pi`: the larger of the screen
/// distances to its temporal neighbours. `None` when the fix has no
/// neighbours at all (lone fix).
///
/// Taking the larger distance keeps cluster boundaries visible: the
/// departure arrow of a parked phase has a far next-neighbour even though
/// its previous neighbour sits on top of it. Endpoints of the track have
/// only one neighbour, so a track that starts or ends parked fades its
/// first/last arrow with the rest of the cluster.
fn local_fix_spacing_px(
    points: PlacedPoints<'_>,
    pi: usize,
    screen_pos: Pos2,
    transform: &crate::transform::MercTransform,
) -> Option<f32> {
    let dist_to = |i: usize| {
        points
            .get(i)
            .map(|p| (transform.to_screen(p.merc()) - screen_pos).length())
    };
    let prev = pi.checked_sub(1).and_then(dist_to);
    let next = dist_to(pi + 1);
    match (prev, next) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(d), None) | (None, Some(d)) => Some(d),
        (None, None) => None,
    }
}

/// Opacity of a fix icon for a given local on-screen spacing: 1.0 at or
/// above the [`fade_band`]'s upper bound, 0.0 at or below its lower bound,
/// linear in between.
fn icon_fade_alpha(spacing_px: f32, icon_size_px: f32) -> f32 {
    let (lo, hi) = fade_band(icon_size_px);
    if hi <= lo {
        // Unreachable while the HI floor and factor exceed their LO
        // counterparts. Kept so a future constant change degrades to opaque
        // icons.
        return 1.0;
    }
    ((spacing_px - lo) / (hi - lo)).clamp(0.0, 1.0)
}

pub(crate) fn fix_icon_alpha(
    fade: TrackIconFade,
    points: PlacedPoints<'_>,
    pi: usize,
    screen_pos: Pos2,
    icon_size_px: f32,
    transform: &crate::transform::MercTransform,
) -> f32 {
    match fade {
        TrackIconFade::AllHidden => 0.0,
        TrackIconFade::AllVisible => 1.0,
        TrackIconFade::PerFix => local_fix_spacing_px(points, pi, screen_pos, transform)
            .map_or(1.0, |spacing| icon_fade_alpha(spacing, icon_size_px)),
    }
}

/// Quantize a quality-line alpha into one of [`QUALITY_LINE_ALPHA_STEPS`]+1
/// discrete levels (0 = invisible, max = fully opaque).
pub(crate) fn line_alpha_bucket(line_alpha: f32) -> u8 {
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "clamped to [0, 1] so the product is in [0, QUALITY_LINE_ALPHA_STEPS]"
    )]
    let bucket = (line_alpha.clamp(0.0, 1.0) * f32::from(QUALITY_LINE_ALPHA_STEPS)).round() as u8;
    bucket
}

/// The drawable alpha for a quantized quality-line bucket.
pub(crate) fn bucket_alpha(bucket: u8) -> f32 {
    f32::from(bucket) / f32::from(QUALITY_LINE_ALPHA_STEPS)
}

/// The index range of every sub-span of a key-styled polyline span: one
/// range per maximal run of points with the same projected key. The caller
/// builds each shape from `span[range]` in one exact-size allocation.
///
/// Each edge takes the navigation state of its starting point. Two adjacent
/// shapes share their boundary point and the line stays continuous: a run's
/// last index is the next run's first index.
pub(crate) fn sub_span_ranges<K: Copy, P: PartialEq>(
    span: &[(K, Pos2)],
    project: impl Fn(K) -> P,
) -> impl Iterator<Item = (P, RangeInclusive<usize>)> {
    let mut start = 0_usize;
    iter::from_fn(move || {
        let (key, _) = *span.get(start)?;
        let key = project(key);
        let mut end = start + 1;
        while end + 1 < span.len() && span.get(end).is_some_and(|&(next, _)| project(next) == key) {
            end += 1;
        }
        if end >= span.len() {
            return None;
        }
        let range = start..=end;
        start = end;
        Some((key, range))
    })
}

/// Renders the directional icon (arrow or chevron) of a single on-screen GPS
/// point. The accuracy circles are a separate pass, see
/// [`draw_accuracy_circles`], and the satellite-count labels another, see
/// [`draw_sat_labels`].
fn draw_tpv_point(
    ui: &Ui,
    screen_pos: Pos2,
    point_kind: &PointKind,
    highlighted: bool,
    style: &TpvDrawStyle,
    batch: &mut IconMeshBatch<'_>,
) {
    match point_kind {
        PointKind::Real {
            color,
            heading: Some(heading),
        } => {
            draw_navigation_arrow(
                ui,
                batch,
                screen_pos,
                *heading,
                color.gamma_multiply(style.icon_alpha),
                highlighted,
                style.outline_alpha * style.icon_alpha,
                style.base_arrow_size,
            );
        }
        PointKind::Real {
            color,
            heading: None,
        } => {
            let color = color.gamma_multiply(style.icon_alpha);
            let size = style.base_arrow_size * ORDINARY_DOT_SIZE_FACTOR;
            if highlighted {
                batch.barrier(ui.painter());
                ui.painter().circle(
                    screen_pos,
                    size + DOT_HIGHLIGHT_GROWTH_PX,
                    color,
                    Stroke::new(DOT_HIGHLIGHT_STROKE_PX, gt_ui_theme::HIGHLIGHT_BLUE),
                );
            } else {
                let rim_alpha = style.outline_alpha * style.icon_alpha;
                batch.push(IconInstance {
                    icon: IconId::NavDot,
                    center: screen_pos,
                    half_extents: Vec2::splat(size),
                    direction: None,
                    tints: [
                        color,
                        Color32::WHITE.gamma_multiply(rim_alpha * rim_alpha),
                        Color32::WHITE,
                    ],
                });
            }
        }
        PointKind::Chevron { direction, fix } => {
            let tint = if highlighted {
                gt_ui_theme::HIGHLIGHT_BLUE
            } else {
                fix.tint(ui.visuals().dark_mode)
            };
            draw_chevron(
                batch,
                screen_pos,
                *direction,
                tint,
                highlighted,
                style.base_arrow_size,
                style.icon_alpha,
            );
        }
    }
}

/// Draw the satellite-count labels for a track's selected anchor points.
///
/// Which anchors get a label this frame is determined by
/// [`crate::sat_labels::select_sat_labels`]. This pass just renders them.
/// Labels draw at full opacity regardless of the icon crossfade - the
/// anchor selection already bounds their density, and a faded-out cluster
/// is exactly where the surviving label carries the information.
pub(crate) fn draw_sat_labels(
    ui: &Ui,
    track: &LoadedTrack,
    label_indices: &[usize],
    style: &TpvDrawStyle,
    transform: &crate::transform::MercTransform,
) {
    let Some(placed) = track.placed_points() else {
        return;
    };
    for point in label_indices.iter().filter_map(|&pi| placed.get(pi)) {
        let Some(used) = point.fix.resolved_used_count(SatelliteQueryScope::Global) else {
            continue;
        };
        let screen_pos = transform.to_screen(point.merc());
        let label = match point.fix.resolved_seen_count(SatelliteQueryScope::Global) {
            Some(seen) => format!("{}/{}", used.count, seen.count),
            None => format!("{} used", used.count),
        };
        let text_pos = screen_pos + egui::vec2(style.base_arrow_size + 3.0, -style.base_arrow_size);
        TextBadge {
            text: label,
            font: egui::FontId::proportional(SAT_LABEL_FONT_PX),
            text_color: Color32::WHITE,
            fill: SAT_LABEL_PLATE_FILL,
            padding_pt: SAT_LABEL_PADDING_PT,
            corner_radius_pt: SAT_LABEL_CORNER_RADIUS_PT,
            plate_height: BadgePlateHeight::LaidOutText,
        }
        .draw(ui, text_pos, egui::Align2::LEFT_BOTTOM);
    }
}

pub(crate) fn tpv_point_color(point: &NavPoint) -> Color32 {
    match point.resolved_navigation_state().state {
        SolutionState::Fix => FIX_BLUE,
        SolutionState::NoFix => FIX_LOST_RED,
        SolutionState::EstimatedOnly => FIX_ESTIMATED_PURPLE,
        SolutionState::Unknown => FIX_UNKNOWN_GRAY,
    }
}

enum PointKind {
    Chevron {
        direction: Vec2,
        fix: ChevronFix,
    },
    Real {
        color: Color32,
        heading: Option<Angle>,
    },
}

/// Compute the travel direction for a chevron from its neighbouring Mercator positions.
///
/// Mercator y increases southward, so dx/dy map directly to egui screen space without
/// a Y-flip. Falls back to [`Vec2::DOWN`] when both neighbours coincide (isolated point).
fn chevron_direction(prev: gt_types::MercPoint, next: gt_types::MercPoint) -> Vec2 {
    let raw = egui::vec2((next.x - prev.x) as f32, (next.y - prev.y) as f32);
    if raw.length_sq() > 1e-12 {
        raw.normalized()
    } else {
        Vec2::DOWN
    }
}

/// Push a hollow chevron, for a fix the receiver did not measure, into the
/// track's icon batch.
///
/// The chevron tip points in `direction` (the inferred travel direction).
fn draw_chevron(
    batch: &mut IconMeshBatch<'_>,
    center: Pos2,
    direction: Vec2,
    tint: Color32,
    highlighted: bool,
    base_size: f32,
    icon_alpha: f32,
) {
    let size = if highlighted {
        base_size + 3.0
    } else {
        base_size
    };
    let tint = tint.gamma_multiply(icon_alpha);
    batch.push(IconInstance {
        icon: IconId::GhostFix,
        center,
        half_extents: Vec2::splat(size),
        direction: Some(direction),
        tints: [tint; 3],
    });
}

/// Draw the car-GPS style navigation arrow for a real fix.
///
/// The hot path pushes one two-tint-slot mesh instance into the track's
/// icon batch (fill tinted with the navigation-state color, rim white with the
/// fade alpha).
/// Highlighted arrows (at most the hovered and the sticky point per frame)
/// keep the painter implementation: their thicker blue outline has a
/// different stroke width than the baked 1.5 px rim, and at that count the
/// per-frame tessellation cost is irrelevant. That painter path barriers
/// the batch itself, and is also what draws when the embedded meshes failed
/// to decode, so arrows never disappear.
#[expect(
    clippy::too_many_arguments,
    reason = "render context requires all parameters; a context struct would not add clarity"
)]
fn draw_navigation_arrow(
    ui: &Ui,
    batch: &mut IconMeshBatch<'_>,
    center: Pos2,
    heading: Angle,
    color: Color32,
    highlighted: bool,
    outline_alpha: f32,
    base_size: f32,
) {
    let angle_rad = heading.get::<radian>() - std::f64::consts::FRAC_PI_2;
    let dir = egui::vec2(angle_rad.cos() as f32, angle_rad.sin() as f32);

    if !highlighted {
        // The painter faded the rim by scaling both its alpha and its stroke
        // width with `outline_alpha`. The baked rim has a fixed width, so
        // squaring the alpha matches that width x alpha ink.
        let rim_alpha = outline_alpha * outline_alpha;
        batch.push(IconInstance {
            icon: IconId::NavArrow,
            center,
            half_extents: Vec2::splat(base_size),
            direction: Some(dir),
            tints: [
                color,
                Color32::WHITE.gamma_multiply(rim_alpha),
                Color32::WHITE,
            ],
        });
        return;
    }

    // Highlighted arrows keep the painter implementation (thicker blue rim
    // with its own stroke width). Flush so it stacks above earlier icons.
    batch.barrier(ui.painter());

    let perp = egui::vec2(-dir.y, dir.x);

    let size = if highlighted {
        base_size + 3.0
    } else {
        base_size
    };
    let stroke_color = if highlighted {
        Color32::from_rgb(100, 200, 255)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, alpha_u8(outline_alpha))
    };
    let stroke_width = if highlighted {
        2.0
    } else {
        1.5 * outline_alpha
    };

    // A car-GPS / Google-Maps style navigation arrow.
    // Vertices (dir points up = forward direction of travel):
    //
    //           *          tip  (+size forward)
    //          / \
    //         /   \
    //        /     \
    //       /   *   \        notch (0.1·size back - concave, pulled toward tip)
    //      /   / \   \
    //     *   /   \   *      wings (0.4·size back, ±0.5·size wide)
    //
    // The outer edges (/ \) run from the tip all the way down to the wings.
    // The inner edges (/ \) run from each wing up to the notch, creating the
    // concave dip at the rear centre.
    //
    // Because the shape is non-convex, the fill is drawn as two convex
    // triangles (tip-right-notch and tip-notch-left) and the outline as a
    // single closed PathShape.
    let tip = center + dir * size;
    let right = center - dir * (size * 0.4) + perp * (size * 0.5);
    let notch = center - dir * (size * 0.1);
    let left = center - dir * (size * 0.4) - perp * (size * 0.5);

    // Fill - two convex triangles avoid non-convex fill artefacts.
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, right, notch],
        color,
        Stroke::NONE,
    ));
    ui.painter().add(egui::Shape::convex_polygon(
        vec![tip, notch, left],
        color,
        Stroke::NONE,
    ));

    // Outline - closed path drawn on top of the fill.
    if stroke_width > 0.0 {
        ui.painter().add(egui::Shape::Path(PathShape::closed_line(
            vec![tip, right, notch, left],
            PathStroke::new(stroke_width, stroke_color),
        )));
    }
}

#[inline]
fn alpha_u8(alpha: f32) -> u8 {
    #[expect(
        clippy::cast_sign_loss,
        reason = "value is clamped to [0.0,1.0] so the product is always non-negative"
    )]
    let v = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    v
}

/// Local on-screen fix spacing, in units of the icon size, at which a fix
/// icon is fully opaque (`HI`) respectively fully transparent (`LO`).
/// Between the two, the icon crossfades with the continuous quality line.
///
/// Icons are deliberately kept while they merely overlap - partially
/// overlapping arrows are still readable. Fading starts only below half an
/// icon size of spacing and completes when neighbouring arrows share almost
/// all of their pixels, at which point skipping them also keeps the
/// tessellated vertex count bounded by screen content.
///
/// The spacing is measured per fix (see [`local_fix_spacing_px`]), not per
/// track, so a parked phase fades out without dragging down the rest of the
/// track.
const ICON_FADE_HI_SPACING_FACTOR: f32 = 0.5;
const ICON_FADE_LO_SPACING_FACTOR: f32 = 0.2;

/// Absolute floors for the fade band, in pixels. At low zoom the proportional
/// thresholds collapse below one pixel of spacing, where packed icons are
/// already unreadable and better replaced by the quality line. Fading completes
/// below [`ICON_FADE_LO_MIN_SPACING_PX`] and starts below
/// [`ICON_FADE_HI_MIN_SPACING_PX`]. The HI floor exceeding the LO floor keeps
/// the band non-empty for any icon size.
const ICON_FADE_LO_MIN_SPACING_PX: f32 = 2.0;
const ICON_FADE_HI_MIN_SPACING_PX: f32 = 5.0;

/// How far past the map rect the icon of a fix reaches into it. A fix drawn
/// this far outside paints part of its icon inside the rect.
const ICON_VIEW_MARGIN_PX: f32 = 50.0;

/// Number of discrete opacity steps for the quality line's crossfade.
/// Per-point line alphas are quantized to this many levels so that long
/// stretches share one key and stay mergeable into single polyline spans.
const QUALITY_LINE_ALPHA_STEPS: u8 = 3;

/// Stroke width of the continuous navigation-state line that replaces the fix
/// icons when they fade out - slightly thicker than the 3 px trackline
/// underneath so the state colors stay readable on top of it.
pub(crate) const QUALITY_LINE_WIDTH: f32 = 5.0;

/// Accuracy circles with a smaller pixel radius than this are skipped - they
/// would be invisible at that size.
const MIN_ACCURACY_CIRCLE_RADIUS_PX: f32 = 2.0;

/// An accuracy circle smaller than this fraction of the directional icon's
/// size is entirely covered by the icon, so drawing it is wasted geometry.
const ACCURACY_CIRCLE_MIN_VISIBLE_FACTOR: f32 = 0.5;

const ACCURACY_CIRCLE_FILL: Color32 = Color32::from_rgba_unmultiplied_const(30, 120, 255, 20);
const ACCURACY_CIRCLE_STROKE: Color32 = Color32::from_rgba_unmultiplied_const(30, 120, 255, 60);
const ACCURACY_CIRCLE_STROKE_WIDTH_PX: f32 = 1.0;

const ORDINARY_DOT_SIZE_FACTOR: f32 = 0.6;
const DOT_HIGHLIGHT_GROWTH_PX: f32 = 3.0;
const DOT_HIGHLIGHT_STROKE_PX: f32 = 2.0;
const FIX_UNKNOWN_GRAY: Color32 = Color32::from_rgb(145, 155, 170);
const FIX_ESTIMATED_PURPLE: Color32 = Color32::from_rgb(165, 105, 210);

const FIX_BLUE: Color32 = Color32::from_rgb(66, 133, 244);
pub(crate) const FIX_LOST_RED: Color32 = Color32::from_rgb(219, 68, 55);

/// Zoom range over which the fix icons scale from dot size up to their
/// full design size: dots at low zoom keep dense clusters from blending
/// into a solid mass.
const ICON_MIN_SIZE_ZOOM: f64 = 12.0;
const ICON_MAX_SIZE_ZOOM: f64 = 18.0;

/// Fix-icon size at [`ICON_MIN_SIZE_ZOOM`] and below respectively
/// [`ICON_MAX_SIZE_ZOOM`] and above.
const MIN_ARROW_SIZE_PX: f32 = 3.0;
const MAX_ARROW_SIZE_PX: f32 = 12.0;

/// Height of a satellite-count label.
const SAT_LABEL_FONT_PX: f32 = 12.0;

const SAT_LABEL_PADDING_PT: f32 = 2.0;

const SAT_LABEL_CORNER_RADIUS_PT: f32 = 2.0;

/// Fill behind a satellite-count label, dark enough for white digits over the
/// track line and over the tiles.
const SAT_LABEL_PLATE_FILL: Color32 = Color32::from_rgba_unmultiplied_const(0, 0, 0, 160);

#[cfg(test)]
mod tests;

#[cfg(test)]
mod arrow_snapshot_tests;
