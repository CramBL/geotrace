//! The unified per-track map layers: trackline, fix-quality line, and
//! per-fix icons.
//!
//! Layer order is explicit here: every trackline first (under everything),
//! then per track the quality line and the icons. The line geometry (LOD
//! selection, time filter, projection, culling) is computed once per track
//! and shared by both line layers through [`LinePointKey`], where the two
//! previous plugins each walked the points themselves.

use std::collections::HashMap;

use egui::{Color32, Response, Stroke, Ui};
use gt_filter::GlobalFilter;
use gt_types::{DataCategory, FileIdx, LoadedFile, LoadedTrack, TrackIdx, TrackRef};
use gt_ui_types::{DrawLayerMask, HighlightScope, MapHighlight, QueryMatches, SkyGlyphVariant};
use walkers::{MapMemory, Plugin, Projector};

use crate::polyline::{CULL_MARGIN_PX, VisiblePath, visible_path};
use crate::query_match_renderer;
use crate::sat_labels::{self, SelectedLabels};
use crate::sky_glyph_renderer::{self, SelectedGlyphs};
use crate::tpv_renderer::{
    self, QUALITY_LINE_WIDTH, TpvDrawStyle, TrackIconFade, bucket_alpha, fix_icon_alpha,
    line_alpha_bucket, quality_line_color, split_spans_by,
};
use crate::track_renderer::{
    self, blink_stroke, draw_track_with_ghost, skip_trackline, track_stroke,
};
use crate::transform::{MapScale, MercTransform, lod_points};
use crate::viewport::{TrackEntry, TrackPlan};

/// Minimum animated progress at which the overlay and three-phase rendering
/// are active.  Below this the overlay is invisible and the normal path runs.
const FADE_VISIBLE_THRESHOLD: f32 = 0.01;

/// Margin around the viewport inside which per-fix icons are still drawn,
/// so icons whose shape extends past the edge are not clipped visibly.
const ICON_VIEW_MARGIN_PX: f32 = 50.0;

/// Zoom-level step the decimation cell size snaps to. Rounding to the nearest
/// bucket leaves the bucketed zoom at most a quarter-level off the true value
/// (2^0.25 ≈ 1.19x scale drift at a bucket's edges, none at its centre), while
/// a smooth zoom still crosses only a few boundaries.
const ZOOM_DECIMATION_BUCKET: f64 = 0.5;

/// Zoom snapped to a coarse bucket, used only to size the collision-grid cell
/// that thins satellite labels and sky glyphs.
///
/// That grid keys its cells in Mercator space, so a cell size that slides
/// continuously with zoom re-partitions the world on every frame: during a
/// smooth zoom the winning point per cell keeps changing, and labels and
/// glyphs flicker as they are dropped and re-added. Snapping the cell's zoom
/// to a bucket holds the partition - and thus the selected set - steady until
/// zoom crosses a boundary. Rendering still uses the real zoom, so positions
/// and scale stay smooth.
///
/// Both inputs to the cell size must use this bucketed zoom: the label spacing
/// ([`tpv_renderer::label_cell_px`]) also varies with zoom, so pairing it with
/// the real scale would leave the cell sliding.
fn decimation_zoom(zoom: f64) -> f64 {
    (zoom / ZOOM_DECIMATION_BUCKET).round() * ZOOM_DECIMATION_BUCKET
}

/// Mercator cell size for a decimation pass whose points sit `spacing_px`
/// apart on screen, computed at the bucketed zoom (see [`decimation_zoom`]).
fn decimation_cell_merc(spacing_px: f32, zoom: f64) -> f64 {
    f64::from(spacing_px) / MapScale::from_zoom(decimation_zoom(zoom)).px_per_merc()
}

/// Per-point styling key for the unified line passes: the trackline dashes
/// ghost stretches. The quality line colors by fix quality and crossfade
/// bucket. One key drives both layers, so each track's points are
/// LOD-selected, projected, and culled exactly once per frame.
///
/// The combined key merges sub-pixel points only when all components
/// match, where the two previous passes each merged on their own narrower
/// key. The painted pixels are the same through occasional extra collinear
/// vertices - and in one edge case a different primitive: a sub-pixel
/// cluster with mixed quality keeps >= 2 points and paints as a short span
/// where the old trackline-only reduction collapsed to a dot (pinned by
/// `sub_pixel_quality_transition_yields_spans_not_dot`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct LinePointKey {
    ghost: bool,
    quality: Color32,
    bucket: u8,
    /// Which `draw` layers cover this point - drives the per-layer halo passes.
    matched: DrawLayerMask,
    /// Whether the query pipeline hides this point. Splits the line at hidden
    /// points.
    hidden: bool,
    /// Whether the match hovered in the query results table covers this point -
    /// drives the hover halo pass.
    hover_matched: bool,
}

/// One visible track's prepared geometry and paint decisions for this
/// frame, produced by [`TrackLayers::prepare_track_geometries`] and
/// consumed by the paint methods.
struct TrackGeometry<'a> {
    fi: FileIdx,
    ti: TrackIdx,
    track: &'a LoadedTrack,
    entry: TrackEntry,
    paint_trackline: bool,
    need_blink: bool,
    path: VisiblePath<LinePointKey>,
    /// Original indices of the LOD level's ghost fixes (hollow chevrons),
    /// collected during the geometry walk.
    ghost_points: Vec<usize>,
}

impl TrackGeometry<'_> {
    fn paints_quality_line(&self) -> bool {
        matches!(
            self.entry.fade,
            Some(TrackIconFade::PerFix | TrackIconFade::AllHidden)
        )
    }

    /// The fade to draw icons with. `None` when no icons draw this frame.
    fn icon_fade(&self) -> Option<TrackIconFade> {
        self.entry
            .fade
            .filter(|&fade| fade != TrackIconFade::AllHidden)
    }
}

#[derive(bon::Builder)]
pub struct TrackLayers<'a> {
    files: &'a [LoadedFile],
    plan: &'a TrackPlan,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    /// Indices of the real fixes inside the viewport, grouped per track by
    /// the collection pass.
    tpv_by_track: HashMap<TrackRef, Vec<usize>>,
    /// First file index that is considered "newly loaded";
    /// files[new_file_boundary..] receive a blinking overlay while
    /// `blink_alpha > 0`.
    new_file_boundary: usize,
    /// Current blink intensity in [0.0, 1.0]. Zero means no overlay.
    blink_alpha: f32,
    /// Animated hover-fade progress in [0.0, 1.0].
    /// Driven by [`crate::HoverFadeState`]: 0 = no overlay, 1 = full overlay.
    hover_fade_alpha: f32,
    /// Matches of the last query run, drawn as halos beneath the tracklines.
    query_matches: Option<&'a QueryMatches>,
    /// Whether the query-highlights display category is visible. Gates the
    /// halo passes only - the query's keep/hide point removal is a query
    /// semantic, not ink, and is never masked.
    display_query_highlights: bool,
    /// Which sky-glyph variant to draw for report-bearing points.
    sky_glyph_variant: SkyGlyphVariant,
}

impl<'a> TrackLayers<'a> {}

impl Plugin for TrackLayers<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        // Build the per-frame coordinate transform once. All per-point calls
        // are then two f64 multiplies + two f64 adds with no large-value
        // cancellation.
        let transform = MercTransform::new(projector, map_memory, ui.max_rect().center());
        let style = tpv_renderer::frame_style(map_memory.zoom());
        let max_rect = ui.max_rect();

        let geometries = self.prepare_track_geometries(max_rect, &style, &transform);
        let sat_labels =
            self.select_sat_labels(&geometries, max_rect, &transform, map_memory.zoom());
        let sky_glyphs =
            self.select_sky_glyphs(&geometries, max_rect, &transform, map_memory.zoom());

        let hover_active =
            self.highlight.fading_enabled && track_renderer::hover_is_active(self.highlight);
        let fade = self.hover_fade_alpha;

        if fade > FADE_VISIBLE_THRESHOLD || hover_active {
            if hover_active {
                // Three-phase rendering: non-focused tracks → animated overlay →
                // focused track on top.  A single overlay rect covers accumulated
                // non-focused track geometry uniformly, preventing the
                // overlap-accumulation artifact where N independently-faded
                // tracks at alpha 1/N become prominent together.  The overlay
                // also covers satellite-count labels so they vanish for free.
                let focused: Vec<bool> = geometries
                    .iter()
                    .map(|geo| track_renderer::is_track_in_focus(self.highlight, geo.fi, geo.ti))
                    .collect();

                self.paint_match_halos(ui, &geometries, &style, |i| {
                    !focused.get(i).copied().unwrap_or(false)
                });
                self.paint_tracklines(ui, &geometries, |i| {
                    !focused.get(i).copied().unwrap_or(false)
                });
                self.paint_tpv_layers(
                    ui,
                    &geometries,
                    &sat_labels,
                    &sky_glyphs,
                    &style,
                    &transform,
                    max_rect,
                    |i| !focused.get(i).copied().unwrap_or(false),
                );
                paint_fade_overlay(ui, max_rect, fade);
                self.paint_match_halos(ui, &geometries, &style, |i| {
                    focused.get(i).copied().unwrap_or(false)
                });
                self.paint_tracklines(ui, &geometries, |i| {
                    focused.get(i).copied().unwrap_or(false)
                });
                self.paint_tpv_layers(
                    ui,
                    &geometries,
                    &sat_labels,
                    &sky_glyphs,
                    &style,
                    &transform,
                    max_rect,
                    |i| focused.get(i).copied().unwrap_or(false),
                );
            } else {
                // No current hover target (fade-out): all tracks under the
                // fading overlay, no focused track in Phase 3.
                self.paint_match_halos(ui, &geometries, &style, |_| true);
                self.paint_tracklines(ui, &geometries, |_| true);
                self.paint_tpv_layers(
                    ui,
                    &geometries,
                    &sat_labels,
                    &sky_glyphs,
                    &style,
                    &transform,
                    max_rect,
                    |_| true,
                );
                paint_fade_overlay(ui, max_rect, fade);
            }
        } else {
            self.paint_match_halos(ui, &geometries, &style, |_| true);
            self.paint_tracklines(ui, &geometries, |_| true);
            self.paint_tpv_layers(
                ui,
                &geometries,
                &sat_labels,
                &sky_glyphs,
                &style,
                &transform,
                max_rect,
                |_| true,
            );
        }

        self.show_hover_overlays(ui, &style, &transform);
    }
}

impl TrackLayers<'_> {
    /// The single geometry walk for every visible track: LOD selection,
    /// time filter, projection, culling, and the per-point styling key,
    /// plus the ghost-fix indices for the chevron pass. The quality color
    /// is keyed even when only the trackline draws - it is a cheap match,
    /// and constant key components never split spans.
    fn prepare_track_geometries(
        &self,
        max_rect: egui::Rect,
        style: &TpvDrawStyle,
        transform: &MercTransform,
    ) -> Vec<TrackGeometry<'_>> {
        let cull_rect = max_rect.expand(CULL_MARGIN_PX);
        // Viewport bounds in Mercator space - used to skip tracks that are
        // entirely outside the visible area without iterating any points.
        let vp_bounds = transform.viewport_merc_bounds(max_rect);

        let mut geometries: Vec<TrackGeometry> = Vec::new();
        for (fi, file) in self.files.iter().enumerate() {
            let fi = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let ti = TrackIdx::new(ti);
                let Some(entry) = self.plan.entry(TrackRef::new(fi, ti)) else {
                    continue;
                };
                if entry.draws_nothing() {
                    continue;
                }
                if !track.metadata.merc_bounds.intersects(vp_bounds) {
                    continue;
                }
                // Blink overlay: a bright pulsing stroke on top of newly
                // loaded tracks for the first 3 seconds after load.
                let need_blink = self.blink_alpha > 0.0 && fi.as_usize() >= self.new_file_boundary;
                let paint_trackline = entry.trackline && !skip_trackline(entry.fade, need_blink);
                let paint_icons = matches!(
                    entry.fade,
                    Some(TrackIconFade::PerFix | TrackIconFade::AllVisible)
                );
                let paint_quality = matches!(
                    entry.fade,
                    Some(TrackIconFade::PerFix | TrackIconFade::AllHidden)
                );
                if !paint_trackline
                    && !paint_quality
                    && !paint_icons
                    && !entry.sat_labels
                    && !entry.sky_glyphs
                {
                    continue;
                }

                let track_ref = TrackRef::new(fi, ti);
                let mut ghost_points: Vec<usize> = Vec::new();
                let fade = entry.fade;
                let hover_match = self
                    .highlight
                    .hover_match
                    .filter(|hm| hm.track == track_ref);
                let pts = lod_points(track, transform)
                    .filter(|(_, p)| {
                        gt_filter::point_passes_time_filter(p.tpv.time().utc(), self.filter)
                    })
                    .map(|(pi, p)| {
                        let screen_pos = transform.to_screen(p.merc);
                        if paint_icons && p.is_ghost_fix() {
                            ghost_points.push(pi);
                        }
                        let bucket = match fade {
                            None | Some(TrackIconFade::AllVisible) => 0,
                            Some(fade) => line_alpha_bucket(
                                1.0 - fix_icon_alpha(
                                    fade,
                                    track,
                                    pi,
                                    screen_pos,
                                    style.base_arrow_size,
                                    transform,
                                ),
                            ),
                        };
                        let (matched, hidden) = self
                            .query_matches
                            .map_or((DrawLayerMask::default(), false), |m| {
                                (m.draw_mask(track_ref, pi), m.is_hidden(track_ref, pi))
                            });
                        let key = LinePointKey {
                            ghost: p.tpv.heading().is_none(),
                            quality: quality_line_color(p),
                            bucket,
                            matched,
                            hidden,
                            hover_matched: hover_match.is_some_and(|hm| hm.contains(pi)),
                        };
                        (key, screen_pos)
                    });
                let path = visible_path(pts, cull_rect);
                geometries.push(TrackGeometry {
                    fi,
                    ti,
                    track,
                    entry,
                    paint_trackline,
                    need_blink,
                    path,
                    ghost_points,
                });
            }
        }
        geometries
    }

    /// Paint every track's query-match halos beneath its lines, for the
    /// entries that pass the `filter(index)` predicate. Runs in the same
    /// phases as the tracklines so the fade overlay dims halos consistently.
    /// The match hovered in the results table paints last, in the highlight
    /// blue, so it reads above the draw layers it may overlap.
    fn paint_match_halos<F>(
        &self,
        ui: &Ui,
        geometries: &[TrackGeometry],
        style: &TpvDrawStyle,
        filter: F,
    ) where
        F: Fn(usize) -> bool,
    {
        // The display mask gates only the halo ink. Query keep/hide
        // semantics (the hidden point ranges) apply regardless.
        if !self.display_query_highlights {
            return;
        }
        let draws = self
            .query_matches
            .map(|m| (m.draws.as_slice(), m.stale))
            .filter(|(draws, _)| !draws.is_empty());
        let hover_match = self.highlight.hover_match;
        if draws.is_none() && hover_match.is_none() {
            return;
        }
        let ring_radius = style.base_arrow_size;
        for (i, geo) in geometries.iter().enumerate() {
            if !filter(i) {
                continue;
            }
            let track_ref = TrackRef::new(geo.fi, geo.ti);
            // A layer per draw query, painted in its own color; overlapping
            // halos stack because each is a separate pass. Capped at the
            // mask width, matching `QueryMatches::draw_mask`.
            for (layer_idx, layer) in draws
                .map(|(draws, _)| draws)
                .unwrap_or_default()
                .iter()
                .take(DrawLayerMask::MAX_LAYERS)
                .enumerate()
            {
                if layer.ranges_for(track_ref).is_empty() {
                    continue;
                }
                let stale = draws.is_some_and(|(_, stale)| stale);
                let color = gt_ui_theme::query_halo_color(layer.color, stale);
                Self::paint_halo_path(ui, &geo.path, ring_radius, color, |key| {
                    key.matched.contains(layer_idx)
                });
            }
            if hover_match.is_some_and(|hm| hm.track == track_ref) {
                Self::paint_halo_path(
                    ui,
                    &geo.path,
                    ring_radius,
                    gt_ui_theme::QUERY_MATCH_HOVER_HALO,
                    |key| key.hover_matched,
                );
            }
        }
    }

    /// Paint one halo pass over a track's prepared path: bands along the
    /// points `covered` selects, a ring where the pass reduces to one point.
    fn paint_halo_path(
        ui: &Ui,
        path: &VisiblePath<LinePointKey>,
        ring_radius: f32,
        color: egui::Color32,
        covered: impl Fn(&LinePointKey) -> bool,
    ) {
        match path {
            VisiblePath::OffScreen => {}
            VisiblePath::Dot(key, pos) => {
                if covered(key) {
                    query_match_renderer::draw_match_ring(ui, *pos, ring_radius, color);
                }
            }
            VisiblePath::Spans(spans) => {
                for span in spans.iter() {
                    query_match_renderer::paint_match_halo_span(
                        ui,
                        span,
                        &covered,
                        ring_radius,
                        color,
                    );
                }
            }
        }
    }

    /// Paint every track's plain line (and blink overlay) for the entries
    /// that pass the `filter(index)` predicate.
    fn paint_tracklines<F>(&self, ui: &Ui, geometries: &[TrackGeometry], filter: F)
    where
        F: Fn(usize) -> bool,
    {
        for (i, geo) in geometries.iter().enumerate() {
            if !filter(i) || !geo.paint_trackline {
                continue;
            }
            let stroke = track_stroke(self.highlight, geo.fi, geo.ti);
            let blink = geo.need_blink.then(|| blink_stroke(self.blink_alpha));
            paint_trackline_path(ui, &geo.path, stroke, blink);
        }
    }

    /// Resolve which satellite-label anchors get a label this frame, for
    /// every track whose TPV layer is on. Labels are collision-resolved
    /// across all tracks at once ([`sat_labels::select_sat_labels`]); the
    /// per-point conditions mirror the icon pass (time filter, query
    /// hiding).
    fn select_sat_labels(
        &self,
        geometries: &[TrackGeometry],
        max_rect: egui::Rect,
        transform: &MercTransform,
        zoom: f64,
    ) -> SelectedLabels {
        let viewport = transform.viewport_merc_bounds(max_rect);
        // The label spacing also varies with zoom, so bucket it too (see
        // [`decimation_zoom`]).
        let cell_merc =
            decimation_cell_merc(tpv_renderer::label_cell_px(decimation_zoom(zoom)), zoom);
        sat_labels::select_sat_labels(
            geometries
                .iter()
                .enumerate()
                .filter(|(_, geo)| geo.entry.sat_labels)
                .map(|(i, geo)| (i, TrackRef::new(geo.fi, geo.ti), geo.track)),
            geometries.len(),
            viewport,
            cell_merc,
            |track_ref, pi, point| {
                gt_filter::point_passes_time_filter(point.tpv.time().utc(), self.filter)
                    && !self
                        .query_matches
                        .is_some_and(|m| m.is_hidden(track_ref, pi))
            },
        )
    }

    /// Resolve which report-bearing points get a sky ring this frame, for
    /// every glyph-enabled track, decimated across all tracks at once
    /// ([`sky_glyph_renderer::select_glyphs`]). Empty below
    /// [`sky_glyph_renderer::MIN_ZOOM`], where per-point rings would be
    /// noise. Per-point conditions mirror the icon and label passes.
    fn select_sky_glyphs(
        &self,
        geometries: &[TrackGeometry],
        max_rect: egui::Rect,
        transform: &MercTransform,
        zoom: f64,
    ) -> SelectedGlyphs {
        if zoom < sky_glyph_renderer::MIN_ZOOM {
            return vec![Vec::new(); geometries.len()];
        }
        let viewport = transform.viewport_merc_bounds(max_rect);
        let min_spacing_px = sky_glyph_renderer::min_spacing_px(self.sky_glyph_variant);
        let cell_merc = decimation_cell_merc(min_spacing_px, zoom);
        sky_glyph_renderer::select_glyphs(
            geometries
                .iter()
                .enumerate()
                .filter(|(_, geo)| geo.entry.sky_glyphs)
                .map(|(i, geo)| (i, TrackRef::new(geo.fi, geo.ti), geo.track)),
            geometries.len(),
            viewport,
            cell_merc,
            |track_ref, pi, point| {
                gt_filter::point_passes_time_filter(point.tpv.time().utc(), self.filter)
                    && !self
                        .query_matches
                        .is_some_and(|m| m.is_hidden(track_ref, pi))
            },
        )
    }

    /// Paint the TPV layer per track: the sky rings underneath, the
    /// fix-quality line, the fix icons on top, then the selected satellite
    /// labels, for the entries that pass the `filter(index)` predicate.
    #[expect(
        clippy::too_many_arguments,
        reason = "per-frame paint context; a wrapper struct would not add clarity"
    )]
    fn paint_tpv_layers<F>(
        &self,
        ui: &Ui,
        geometries: &[TrackGeometry],
        sat_labels: &SelectedLabels,
        sky_glyphs: &SelectedGlyphs,
        style: &TpvDrawStyle,
        transform: &MercTransform,
        max_rect: egui::Rect,
        filter: F,
    ) where
        F: Fn(usize) -> bool,
    {
        let icon_view_rect = max_rect.expand(ICON_VIEW_MARGIN_PX);
        for (i, geo) in geometries.iter().enumerate() {
            if !filter(i) {
                continue;
            }
            // Glyphs first, so the quality line, icons, and labels stay
            // legible on top of the subtle background context.
            if let Some(glyph_indices) = sky_glyphs.get(i) {
                sky_glyph_renderer::draw_glyphs(
                    ui,
                    geo.track,
                    glyph_indices,
                    transform,
                    self.sky_glyph_variant,
                    tpv_renderer::glyph_size_scale(style),
                );
            }
            if geo.paints_quality_line() {
                paint_quality_path(ui, &geo.path);
            }
            if let Some(fade) = geo.icon_fade() {
                let track_ref = TrackRef::new(geo.fi, geo.ti);
                let tpv = self.tpv_by_track.get(&track_ref);
                // In keep/hide, drop the icons of hidden points too, so the
                // arrows match the (broken) line.
                let (filtered_tpv, filtered_ghost);
                let (tpv, ghost) = match self.query_matches {
                    Some(matches) if !matches.hidden_ranges(track_ref).is_empty() => {
                        let shown = |pi: &usize| !matches.is_hidden(track_ref, *pi);
                        filtered_tpv = tpv.map(|v| v.iter().copied().filter(shown).collect());
                        filtered_ghost = geo
                            .ghost_points
                            .iter()
                            .copied()
                            .filter(shown)
                            .collect::<Vec<_>>();
                        (filtered_tpv.as_ref(), filtered_ghost.as_slice())
                    }
                    _ => (tpv, geo.ghost_points.as_slice()),
                };
                tpv_renderer::draw_track_icons(
                    ui,
                    icon_view_rect,
                    geo.fi,
                    geo.ti,
                    geo.track,
                    tpv,
                    ghost,
                    style,
                    fade,
                    transform,
                    self.highlight,
                    self.filter,
                );
            }
            // Labels last so their backplates sit on top of the icons.
            if let Some(label_indices) = sat_labels.get(i) {
                tpv_renderer::draw_sat_labels(ui, geo.track, label_indices, style, transform);
            }
        }
    }

    /// Show the hover artifacts that sit on top of all layers: the TPV
    /// tooltip for the hovered point (set by NavMap the previous frame) and
    /// the plot-cursor cross-highlight ring.
    fn show_hover_overlays(&self, ui: &Ui, style: &TpvDrawStyle, transform: &MercTransform) {
        // Suppressed when the sticky popup is already showing this exact
        // point and when any popup is open.
        if let Some(HighlightScope::Point(r)) = self.highlight.hover
            && r.category == DataCategory::Tpv
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && !self.highlight.suppress_hover_labels
        {
            // Hovering a matched point adds the match context above the
            // standard point table.
            let match_header = self.query_matches.and_then(|matches| {
                let range = matches
                    .header_range(r.track, r.point_index.as_usize())?
                    .clone();
                Some(move |ui: &mut Ui| {
                    query_match_renderer::match_header_ui(
                        ui,
                        self.files,
                        r.track,
                        &range,
                        matches.stale,
                    );
                })
            });
            tpv_renderer::show_tooltip(ui, self.files, r, match_header);
        }

        tpv_renderer::draw_plot_hover_overlay(ui, self.files, self.highlight, style, transform);
    }
}

/// Maximal runs of consecutive shown (non-hidden) points within one span.
///
/// In `draw` mode nothing is hidden, so this yields the whole span as one
/// run and rendering is unchanged. In `keep`/`hide` the line breaks at every
/// hidden point rather than bridging the gap.
fn shown_runs(
    span: &[(LinePointKey, egui::Pos2)],
) -> impl Iterator<Item = &[(LinePointKey, egui::Pos2)]> {
    span.split(|(key, _)| key.hidden)
        .filter(|run| !run.is_empty())
}

/// Paint a track's plain (track-colored) line from its prepared geometry,
/// with the optional blink overlay on top.
fn paint_trackline_path(
    ui: &Ui,
    path: &VisiblePath<LinePointKey>,
    stroke: Stroke,
    blink: Option<Stroke>,
) {
    match path {
        VisiblePath::OffScreen => {}
        VisiblePath::Dot(key, pos) => {
            if key.hidden {
                return;
            }
            ui.painter().circle_filled(*pos, stroke.width, stroke.color);
            if let Some(blink) = blink {
                ui.painter().circle_filled(*pos, blink.width, blink.color);
            }
        }
        VisiblePath::Spans(spans) => {
            for span in spans.iter() {
                for run in shown_runs(span) {
                    draw_track_with_ghost(ui.painter(), run, stroke, |key| key.ghost);
                    if let Some(blink) = blink {
                        let bp: Vec<egui::Pos2> = run.iter().map(|&(_, pos)| pos).collect();
                        ui.painter().add(egui::Shape::line(bp, blink));
                    }
                }
            }
        }
    }
}

/// Draw a flat semi-transparent rectangle over the map viewport to dim all
/// track geometry drawn before this call.
///
/// `progress` is the animated fade value in [0.0, 1.0] produced by
/// [`crate::HoverFadeState`].  At `progress = 1.0` the overlay reaches its
/// theme's peak opacity ([`track_renderer::FOCUS_SCRIM_MAX_ALPHA_LIGHT`] /
/// [`track_renderer::FOCUS_SCRIM_MAX_ALPHA_DARK`]).
/// Using a single rect rather than per-track alpha prevents the accumulation
/// artifact where N overlapping faded tracks at alpha `1/N` each would sum to
/// full visibility at busy intersections.
fn paint_fade_overlay(ui: &Ui, max_rect: egui::Rect, progress: f32) {
    let alpha = focus_scrim_alpha(ui.visuals().dark_mode, progress);
    let bg = track_renderer::FOCUS_SCRIM_COLOR;
    ui.painter().rect_filled(
        max_rect,
        0.0,
        egui::Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), alpha),
    );
}

/// The scrim's alpha for a theme and animation `progress`, scaling the theme's
/// peak opacity by `progress` clamped to [0.0, 1.0].
fn focus_scrim_alpha(dark_mode: bool, progress: f32) -> u8 {
    let max_alpha = if dark_mode {
        track_renderer::FOCUS_SCRIM_MAX_ALPHA_DARK
    } else {
        track_renderer::FOCUS_SCRIM_MAX_ALPHA_LIGHT
    } * 255.0;
    #[expect(
        clippy::cast_sign_loss,
        reason = "max_alpha and progress.clamp(0,1) are both non-negative"
    )]
    let alpha = (max_alpha * progress.clamp(0.0, 1.0)) as u8;
    alpha
}

/// Paint a track's fix-quality line from its prepared geometry: each edge
/// colored by the fix quality at its starting point and faded by the
/// crossfade bucket, with fully transparent stretches skipped.
fn paint_quality_path(ui: &Ui, path: &VisiblePath<LinePointKey>) {
    let painter = ui.painter();
    match path {
        VisiblePath::OffScreen => {}
        VisiblePath::Dot(key, pos) => {
            if key.hidden {
                return;
            }
            if key.bucket > 0 {
                painter.circle_filled(
                    *pos,
                    QUALITY_LINE_WIDTH,
                    key.quality.gamma_multiply(bucket_alpha(key.bucket)),
                );
            }
        }
        VisiblePath::Spans(spans) => {
            for span in spans.iter() {
                // Restrict to shown runs first (keep/hide), then color each
                // run by fix quality as before.
                for run in shown_runs(span) {
                    for ((quality, bucket), sub_span) in
                        split_spans_by(run, |key| (key.quality, key.bucket))
                    {
                        if bucket == 0 {
                            continue;
                        }
                        painter.add(egui::Shape::line(
                            sub_span,
                            Stroke::new(
                                QUALITY_LINE_WIDTH,
                                quality.gamma_multiply(bucket_alpha(bucket)),
                            ),
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Rect, pos2};

    use gt_test_utils::TestHarness;
    use gt_ui_types::DrawLayerMask;

    use super::{
        LinePointKey, ZOOM_DECIMATION_BUCKET, decimation_zoom, focus_scrim_alpha,
        paint_fade_overlay, shown_runs,
    };
    use crate::polyline::{VisiblePath, visible_path};

    /// The decimation zoom is a step function: a fine zoom sweep the width of a
    /// bucket lands on a single value, so the collision-grid cell - and thus
    /// the selected labels and glyphs - hold steady instead of churning
    /// frame-to-frame during a smooth zoom.
    #[test]
    fn decimation_zoom_holds_steady_within_a_bucket() {
        let start = 12.0;
        let sweep: Vec<(f64, f64)> = (0u16..50)
            .map(|i| start + f64::from(i) / 50.0 * ZOOM_DECIMATION_BUCKET)
            .map(|real| (real, decimation_zoom(real)))
            .collect();
        // Every step across one bucket width maps to at most two distinct
        // bucketed zooms (the one boundary the sweep may cross), never a fresh
        // value each frame.
        let mut distinct: Vec<f64> = sweep.iter().map(|&(_, bucketed)| bucketed).collect();
        distinct.dedup();
        assert!(
            distinct.len() <= 2,
            "sweep churned across {} buckets: {distinct:?}",
            distinct.len()
        );
        // The bucketed zoom never drifts far from the real zoom, so on-screen
        // spacing stays close to the target.
        for &(real, bucketed) in &sweep {
            assert!((bucketed - real).abs() <= ZOOM_DECIMATION_BUCKET / 2.0);
        }
    }

    /// Snapshot: the focus scrim at full progress dims the scene by darkening
    /// it, in both themes. A regression guard for the light-mode wash-out (the
    /// scrim used to brighten a light map). Paints a backdrop and a few marks,
    /// then the scrim over them.
    #[rstest::rstest]
    #[case::light("focus_scrim_light", false)]
    #[case::dark("focus_scrim_dark", true)]
    fn focus_scrim_darkens_the_scene(#[case] name: &str, #[case] dark_mode: bool) {
        let mut harness = TestHarness::builder()
            .size(egui::vec2(200.0, 120.0))
            .theme(dark_mode)
            .ui(|ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();
                // A light backdrop stands in for the map tiles (light in both
                // themes), the surface the scrim used to wash out in light mode.
                painter.rect_filled(rect, 0.0, Color32::from_gray(225));
                for (i, color) in [Color32::RED, Color32::GREEN, Color32::from_rgb(0, 120, 255)]
                    .into_iter()
                    .enumerate()
                {
                    let x = rect.left() + 40.0 + i as f32 * 60.0;
                    painter.circle_filled(egui::pos2(x, rect.center().y), 16.0, color);
                }
                paint_fade_overlay(ui, rect, 1.0);
            });
        harness.run();
        harness.snapshot(name);
    }

    /// A span of points with the given `hidden` flags, at increasing x.
    fn span(hidden: &[bool]) -> Vec<(LinePointKey, egui::Pos2)> {
        hidden
            .iter()
            .enumerate()
            .map(|(i, &hidden)| {
                let key = LinePointKey {
                    ghost: false,
                    quality: Color32::BLUE,
                    bucket: 0,
                    matched: DrawLayerMask::default(),
                    hidden,
                    hover_matched: false,
                };
                (key, pos2(i as f32, 0.0))
            })
            .collect()
    }

    #[test]
    fn shown_runs_of_all_visible_is_one_run() {
        // Draw mode: nothing hidden, so the whole span is a single run and
        // rendering is byte-identical to before the keep/hide feature.
        let s = span(&[false, false, false]);
        let runs: Vec<usize> = shown_runs(&s).map(<[_]>::len).collect();
        assert_eq!(runs, vec![3]);
    }

    #[test]
    fn shown_runs_break_at_hidden_points() {
        // Hidden points split the line; leading/trailing/adjacent hidden
        // points yield no empty runs, and an isolated shown point is a
        // 1-element run (which draws no edge but keeps its icon).
        let s = span(&[true, false, false, true, false, true]);
        let runs: Vec<usize> = shown_runs(&s).map(<[_]>::len).collect();
        assert_eq!(runs, vec![2, 1]);
    }

    #[test]
    fn shown_runs_of_all_hidden_is_empty() {
        let s = span(&[true, true]);
        assert_eq!(shown_runs(&s).count(), 0);
    }

    /// A parked cluster with mixed fix quality at sub-pixel distance: the
    /// quality transition forces point retention, so the unified key paints
    /// a short span where the old trackline-only reduction (keyed on ghost
    /// alone) collapsed to a dot. Pins this intentional divergence of the
    /// unified geometry walk.
    #[test]
    fn sub_pixel_quality_transition_yields_spans_not_dot() {
        let rect = Rect {
            min: pos2(0.0, 0.0),
            max: pos2(100.0, 100.0),
        };
        let key = |quality| LinePointKey {
            ghost: false,
            quality,
            bucket: 3,
            matched: DrawLayerMask::default(),
            hidden: false,
            hover_matched: false,
        };
        let pts = vec![
            (key(Color32::BLUE), pos2(10.0, 10.0)),
            (key(Color32::YELLOW), pos2(10.2, 10.0)),
        ];
        let path = visible_path(pts.into_iter(), rect);
        assert!(matches!(path, VisiblePath::Spans(_)));
    }

    #[test]
    fn focus_scrim_dims_gently_and_is_lighter_in_light_mode() {
        // The scrim darkens in both themes (a dark rect); it used to brighten
        // in light mode, washing the map out. Both stay well below opaque, and
        // light mode is gentler since a dark scrim reads heavier over a light
        // map at equal opacity.
        let light = focus_scrim_alpha(false, 1.0);
        let dark = focus_scrim_alpha(true, 1.0);
        assert!(light < 128, "light scrim {light} should stay legible");
        assert!(dark < 128, "dark scrim {dark} should stay legible");
        assert!(
            light < dark,
            "light scrim {light} should be gentler than dark {dark}"
        );
    }

    #[test]
    fn focus_scrim_scales_with_progress() {
        assert_eq!(focus_scrim_alpha(false, 0.0), 0);
        // Clamped above 1.0 so the animation overshooting cannot exceed the peak.
        assert_eq!(focus_scrim_alpha(true, 2.0), focus_scrim_alpha(true, 1.0));
    }
}
