//! The unified per-track map layers: trackline, fix-quality line, and
//! per-fix icons.
//!
//! Layer order is explicit here: every trackline first (under everything),
//! then per track the quality line and the icons. The line geometry (LOD
//! selection, time filter, projection, culling) is computed once per track
//! and shared by both line layers through [`LinePointKey`].

use egui::{Color32, Response, Stroke, Ui};
use gt_filter::GlobalFilter;
use gt_types::{FileIdx, LoadedFile, LoadedTrack, TrackIdx, TrackRef};
use gt_ui_types::{DrawLayerMask, MapHighlight, QueryMatches, SkyGlyphVariant, TrackMatchView};
use rustc_hash::FxHashMap;
use walkers::{MapMemory, Plugin, Projector};

use crate::collision_grid;
use crate::icon_mesh::IconMeshLibrary;
use crate::match_reveal::HaloStyle;
use crate::polyline::{CULL_MARGIN_PX, VisiblePath, visible_path};
use crate::query_match_renderer;
use crate::sat_labels::{self, LabelSelection};
use crate::sky_glyph_renderer::{self, GlyphSelection};
use crate::tpv_renderer::{
    self, ChevronFix, QUALITY_LINE_WIDTH, TpvDrawStyle, TrackIconFade, bucket_alpha,
    fix_icon_alpha, line_alpha_bucket, quality_line_color,
};
use crate::track_renderer::{
    self, blink_stroke, draw_track_with_ghost, skip_trackline, track_stroke,
};
use crate::transform::{GeometryCull, MercTransform, lod_points};
use crate::viewport::{TrackEntry, TrackPlan};

/// Minimum animated progress at which the overlay and three-phase rendering
/// are active.  Below this the overlay is invisible and the normal path runs.
const FADE_VISIBLE_THRESHOLD: f32 = 0.01;

/// Per-point styling key for the unified line passes: the trackline dashes
/// ghost stretches. The quality line colors by fix quality and crossfade
/// bucket. One key drives both layers, so each track's points are
/// LOD-selected, projected, and culled exactly once per frame.
///
/// The combined key merges sub-pixel points only when every component matches.
/// A sub-pixel cluster with mixed quality therefore keeps >= 2 points and
/// paints as a short span (pinned by
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
    /// The point indices of the LOD level this frame's geometry walk covered,
    /// in fix order. `None` where it covered the track's full point list.
    /// [`TrackGeometry::chevrons_of`] keeps a viewport fix only when this
    /// level holds it, so one decimation governs the line and the chevrons.
    walked_level_indices: Option<&'a [u32]>,
}

impl TrackGeometry<'_> {
    /// The chevrons the icon pass draws for this track, in fix order: those
    /// fixes of `viewport_fixes` the walked LOD level keeps, whose time the
    /// filter's window holds, that the query does not hide, and that the map
    /// draws hollow ([`ChevronFix::for_fix`]).
    ///
    /// `viewport_fixes` are the R-tree's hits for this track, in the order
    /// its nodes hold them.
    fn chevrons_of(
        &self,
        viewport_fixes: &[usize],
        filter: &GlobalFilter,
        query_view: &TrackMatchView<'_>,
    ) -> Vec<(usize, ChevronFix)> {
        let Some(placed) = self.track.placed_points() else {
            return Vec::new();
        };
        let mut chevrons: Vec<(usize, ChevronFix)> = viewport_fixes
            .iter()
            .filter(|&&pi| self.level_holds_the_fix_at(pi) && !query_view.is_hidden(pi))
            .filter_map(|&pi| {
                let point = placed.get(pi)?;
                if !gt_filter::point_passes_time_filter(point.fix.tpv.time().utc(), filter) {
                    return None;
                }
                Some((pi, ChevronFix::for_fix(point.fix)?))
            })
            .collect();
        chevrons.sort_unstable_by_key(|&(pi, _)| pi);
        chevrons
    }

    /// Whether the LOD level the walk covered holds the fix at `pi`. Every
    /// fix passes where the walk covered the full point list. The level's
    /// indices are in fix order.
    fn level_holds_the_fix_at(&self, pi: usize) -> bool {
        match self.walked_level_indices {
            Some(indices) => {
                u32::try_from(pi).is_ok_and(|index| indices.binary_search(&index).is_ok())
            }
            None => true,
        }
    }

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
    /// the collection pass. Borrowed from the reused [`crate::viewport::VisiblePoints`]
    /// scratch, so lookups may return an empty list for a no-longer-visible track.
    /// `None` in a frame that collected no fix, where no track's icons draw
    /// and no lookup happens.
    #[builder(required)]
    tpv_by_track: Option<&'a FxHashMap<TrackRef, Vec<usize>>>,
    /// First file index that is considered "newly loaded".
    /// `files[new_file_boundary..]` receive a blinking overlay while
    /// `blink_alpha > 0`.
    new_file_boundary: usize,
    /// Current blink intensity in [0.0, 1.0]. Zero means no overlay.
    blink_alpha: f32,
    /// Animated hover-fade progress in [0.0, 1.0].
    /// Driven by [`crate::HoverFadeState`]: 0 = no overlay, 1 = full overlay.
    hover_fade_alpha: f32,
    /// How far into a new run's reveal the match halos are, from
    /// [`crate::match_reveal::MatchRevealState`]: 1 = fully inflated at the
    /// start of the reveal, 0 = settled.
    match_reveal: f32,
    /// Matches of the last query run, drawn as halos beneath the tracklines.
    query_matches: Option<&'a QueryMatches>,
    /// Whether the query-highlights display category is visible. Gates the
    /// halo passes only - the query's keep/hide point removal is a query
    /// semantic, not ink, and is never masked.
    display_query_highlights: bool,
    /// Which sky-glyph variant to draw for report-bearing points.
    sky_glyph_variant: SkyGlyphVariant,
    /// Pre-tessellated icon meshes for the ghost chevrons.
    icon_meshes: Option<&'a IconMeshLibrary>,
    /// Reused decimation scratch for the satellite-label selection, filled
    /// during [`Plugin::run`] and borrowed by the paint passes.
    sat_label_scratch: &'a mut LabelSelection,
    /// Reused decimation scratch for the sky-glyph selection.
    sky_glyph_scratch: &'a mut GlyphSelection,
}

impl Plugin for TrackLayers<'_> {
    fn run(
        mut self: Box<Self>,
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
        // The `select_*` calls fill the scratches through `&mut self`. The
        // `.selected()` reads then borrow them immutably for the paint passes.
        // Split so the mutable fill fully ends before the shared reads begin.
        self.select_sat_labels(&geometries, max_rect, &transform, map_memory.zoom());
        self.select_sky_glyphs(&geometries, max_rect, &transform, map_memory.zoom());
        let sat_labels = self.sat_label_scratch.selected();
        let sky_glyphs = self.sky_glyph_scratch.selected();

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
                    sat_labels,
                    sky_glyphs,
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
                    sat_labels,
                    sky_glyphs,
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
                    sat_labels,
                    sky_glyphs,
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
                sat_labels,
                sky_glyphs,
                &style,
                &transform,
                max_rect,
                |_| true,
            );
        }

        tpv_renderer::draw_plot_hover_overlay(ui, self.files, self.highlight, &style, &transform);
    }
}

impl<'a> TrackLayers<'a> {
    /// The single geometry walk for every visible track: LOD selection,
    /// time filter, projection, culling, and the per-point styling key. The
    /// quality color is keyed even when only the trackline draws - it is a
    /// cheap match, and constant key components never split spans.
    fn prepare_track_geometries(
        &self,
        max_rect: egui::Rect,
        style: &TpvDrawStyle,
        transform: &MercTransform,
    ) -> Vec<TrackGeometry<'a>> {
        let cull_rect = max_rect.expand(CULL_MARGIN_PX);
        let cull = GeometryCull::new(transform, cull_rect, self.filter);
        // Viewport bounds in Mercator space - used to skip tracks that are
        // entirely outside the visible area without iterating any points.
        let vp_bounds = transform.viewport_merc_bounds(max_rect);

        // Bind the `'a` file slice out of `&self` so the returned geometry
        // borrows `files`, not this `&self` call - the selection passes then
        // take `&mut self` while the geometry is still alive.
        let files: &'a [LoadedFile] = self.files;
        let mut geometries: Vec<TrackGeometry<'a>> = Vec::new();
        for (fi, file) in files.iter().enumerate() {
            let fi = FileIdx::new(fi);
            for (ti, track) in file.tracks.iter().enumerate() {
                let ti = TrackIdx::new(ti);
                let Some(entry) = self.plan.entry(TrackRef::new(fi, ti)) else {
                    continue;
                };
                if entry.draws_nothing() {
                    continue;
                }
                // A track with no geometry is nowhere on the map: nothing of
                // it is drawn, and the culling test has no box to take.
                let Some(geometry) = track.geometry.measured() else {
                    continue;
                };
                if !geometry.merc_bounds.intersects(vp_bounds) {
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
                let fade = entry.fade;
                let hover_match = self
                    .highlight
                    .hover_match
                    .filter(|hm| hm.track == track_ref);
                let query_view = TrackMatchView::for_track(self.query_matches, track_ref);
                let Some(placed) = track.placed_points() else {
                    continue;
                };
                let walk = lod_points(track, placed, transform, cull);
                let walked_level_indices = walk.walked_level_indices();
                let pts = walk.map(|(pi, p)| {
                    let screen_pos = transform.to_screen(p.merc());
                    let bucket = match fade {
                        None | Some(TrackIconFade::AllVisible) => 0,
                        Some(fade) => line_alpha_bucket(
                            1.0 - fix_icon_alpha(
                                fade,
                                placed,
                                pi,
                                screen_pos,
                                style.base_arrow_size,
                                transform,
                            ),
                        ),
                    };
                    let key = LinePointKey {
                        ghost: p.fix.tpv.heading().is_none(),
                        quality: quality_line_color(p.fix),
                        bucket,
                        matched: query_view.draw_mask(pi),
                        hidden: query_view.is_hidden(pi),
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
                    walked_level_indices,
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
        let halo = HaloStyle::new(style, self.match_reveal);
        for (i, geo) in geometries.iter().enumerate() {
            if !filter(i) {
                continue;
            }
            let track_ref = TrackRef::new(geo.fi, geo.ti);
            // A layer per draw query, painted in its own color. Overlapping
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
                let color = halo.revealed_color(gt_ui_theme::query_halo_color(layer.color, stale));
                Self::paint_halo_path(ui, &geo.path, halo, color, |key| {
                    key.matched.contains(layer_idx)
                });
            }
            // The hovered match paints settled: the reveal belongs to the
            // draw layers of the run.
            if hover_match.is_some_and(|hm| hm.track == track_ref) {
                Self::paint_halo_path(
                    ui,
                    &geo.path,
                    HaloStyle::new(style, 0.0),
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
        halo: HaloStyle,
        color: egui::Color32,
        covered: impl Fn(&LinePointKey) -> bool,
    ) {
        match path {
            VisiblePath::OffScreen => {}
            VisiblePath::Dot(key, pos) => {
                if covered(key) {
                    query_match_renderer::draw_match_ring(ui, *pos, halo, color);
                }
            }
            VisiblePath::Spans(spans) => {
                for span in spans.iter() {
                    query_match_renderer::paint_match_halo_span(ui, span, &covered, halo, color);
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
    /// across all tracks at once ([`sat_labels::select_sat_labels`]). The
    /// per-point conditions mirror the icon pass (time filter, query
    /// hiding).
    fn select_sat_labels(
        &mut self,
        geometries: &[TrackGeometry],
        max_rect: egui::Rect,
        transform: &MercTransform,
        zoom: f64,
    ) {
        let viewport = transform.viewport_merc_bounds(max_rect);
        // The label spacing also varies with zoom, so bucket it too (see
        // [`decimation_zoom`]).
        let cell_merc = collision_grid::decimation_cell_merc(
            tpv_renderer::label_cell_px(collision_grid::decimation_zoom(zoom)),
            zoom,
        );
        // Copy the shared borrows out so the closure captures them, not `self`,
        // leaving `self.sat_label_scratch` free to borrow mutably.
        let filter = self.filter;
        let query_matches = self.query_matches;
        sat_labels::select_sat_labels(
            &mut *self.sat_label_scratch,
            geometries
                .iter()
                .enumerate()
                .filter(|(_, geo)| geo.entry.sat_labels)
                .map(|(i, geo)| {
                    let track_ref = TrackRef::new(geo.fi, geo.ti);
                    let query_view = TrackMatchView::for_track(query_matches, track_ref);
                    (i, track_ref, geo.track, query_view)
                }),
            geometries.len(),
            viewport,
            cell_merc,
            move |query_view, pi, point| {
                gt_filter::point_passes_time_filter(point.tpv.time().utc(), filter)
                    && !query_view.is_hidden(pi)
            },
        );
    }

    /// Resolve which report-bearing points get a sky ring this frame, for
    /// every glyph-enabled track, decimated across all tracks at once
    /// ([`sky_glyph_renderer::select_glyphs`]). Empty below
    /// [`sky_glyph_renderer::MIN_ZOOM`], where per-point rings would be
    /// noise. Per-point conditions mirror the icon and label passes.
    fn select_sky_glyphs(
        &mut self,
        geometries: &[TrackGeometry],
        max_rect: egui::Rect,
        transform: &MercTransform,
        zoom: f64,
    ) {
        let viewport = transform.viewport_merc_bounds(max_rect);
        let filter = self.filter;
        let query_matches = self.query_matches;
        let variant = self.sky_glyph_variant;
        if zoom < sky_glyph_renderer::MIN_ZOOM {
            // No rings at this zoom. Still refresh the scratch to the right
            // number of empty buckets so the paint pass reads a matching
            // length, without walking any points.
            sky_glyph_renderer::select_glyphs(
                &mut *self.sky_glyph_scratch,
                std::iter::empty::<(usize, TrackRef, &LoadedTrack, TrackMatchView<'_>)>(),
                geometries.len(),
                viewport,
                // No candidates are pushed, so the cell size is never read.
                1.0,
                |_, _, _| false,
            );
            return;
        }
        let min_spacing_px = sky_glyph_renderer::min_spacing_px(variant);
        let cell_merc = collision_grid::decimation_cell_merc(min_spacing_px, zoom);
        sky_glyph_renderer::select_glyphs(
            &mut *self.sky_glyph_scratch,
            geometries
                .iter()
                .enumerate()
                .filter(|(_, geo)| geo.entry.sky_glyphs)
                .map(|(i, geo)| {
                    let track_ref = TrackRef::new(geo.fi, geo.ti);
                    let query_view = TrackMatchView::for_track(query_matches, track_ref);
                    (i, track_ref, geo.track, query_view)
                }),
            geometries.len(),
            viewport,
            cell_merc,
            move |query_view, pi, point| {
                gt_filter::point_passes_time_filter(point.tpv.time().utc(), filter)
                    && !query_view.is_hidden(pi)
            },
        );
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
        sat_labels: &[Vec<usize>],
        sky_glyphs: &[Vec<usize>],
        style: &TpvDrawStyle,
        transform: &MercTransform,
        max_rect: egui::Rect,
        filter: F,
    ) where
        F: Fn(usize) -> bool,
    {
        let icon_view_rect = tpv_renderer::icon_cull_rect(max_rect);
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
                let tpv = self
                    .tpv_by_track
                    .and_then(|by_track| by_track.get(&track_ref));
                // In keep/hide, drop the icons of hidden points too, so the
                // arrows match the (broken) line.
                let query_view = TrackMatchView::for_track(self.query_matches, track_ref);
                let chevrons =
                    geo.chevrons_of(tpv.map_or(&[], Vec::as_slice), self.filter, &query_view);
                let filtered_tpv;
                let tpv = if query_view.hides_any_point() {
                    filtered_tpv = tpv.map(|v| {
                        v.iter()
                            .copied()
                            .filter(|&pi| !query_view.is_hidden(pi))
                            .collect()
                    });
                    filtered_tpv.as_ref()
                } else {
                    tpv
                };
                tpv_renderer::draw_track_icons(
                    ui,
                    icon_view_rect,
                    geo.fi,
                    geo.ti,
                    geo.track,
                    tpv,
                    &chevrons,
                    style,
                    fade,
                    transform,
                    self.highlight,
                    self.filter,
                    self.icon_meshes,
                );
            }
            // Labels last so their backplates sit on top of the icons.
            if let Some(label_indices) = sat_labels.get(i) {
                tpv_renderer::draw_sat_labels(ui, geo.track, label_indices, style, transform);
            }
        }
    }
}

/// Maximal runs of consecutive shown (non-hidden) points within one span.
///
/// In `draw` mode nothing is hidden, so this yields the whole span as one run.
/// In `keep`/`hide` the line breaks at every hidden point.
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
/// A single rect prevents the accumulation artifact where N overlapping
/// faded tracks at alpha `1/N` each would sum to full visibility at busy
/// intersections.
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
                        tpv_renderer::sub_span_ranges(run, |key| (key.quality, key.bucket))
                    {
                        if bucket == 0 {
                            continue;
                        }
                        let Some(sub_span_points) = run.get(sub_span) else {
                            continue;
                        };
                        painter.add(egui::Shape::line(
                            sub_span_points.iter().map(|&(_, pos)| pos).collect(),
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

    use gt_ui_types::DrawLayerMask;

    use super::{LinePointKey, focus_scrim_alpha, paint_fade_overlay, shown_runs};
    use crate::polyline::{VisiblePath, visible_path};

    /// Snapshot: the focus scrim at full progress dims the scene by darkening
    /// it, in both themes. A regression guard for the light-mode wash-out (the
    /// scrim used to brighten a light map). Paints a backdrop and a few marks,
    /// then the scrim over them.
    #[rstest::rstest]
    #[case::light("focus_scrim_light", false)]
    #[case::dark("focus_scrim_dark", true)]
    fn focus_scrim_darkens_the_scene(#[case] name: &str, #[case] dark_mode: bool) {
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(200.0, 120.0))
            .theme(dark_mode)
            .ui(|ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();
                // A light backdrop replaces the map tiles (light in both
                // themes), the surface the scrim is drawn over.
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

    /// A span of points at increasing x, one per flag in `hidden`.
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
        // Draw mode: nothing hidden, so the whole span is a single run.
        let s = span(&[false, false, false]);
        let runs: Vec<usize> = shown_runs(&s).map(<[_]>::len).collect();
        assert_eq!(runs, vec![3]);
    }

    #[test]
    fn shown_runs_break_at_hidden_points() {
        // Hidden points split the line. Leading, trailing and adjacent
        // hidden points are skipped, and an isolated shown point is a
        // 1-element run (icon only, no edge).
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
        // The scrim darkens in both themes (a dark rect). Both stay well
        // below opaque, and light mode is gentler since a dark scrim reads
        // heavier over a light map at equal opacity.
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

    /// The chevrons the icon pass draws come from the fixes the viewport query
    /// found. These cases hold that source and the geometry walk to the same set
    /// of chevrons over the map rect.
    mod chevrons {
        use std::iter;
        use std::ops::Range;

        use chrono::{DateTime, TimeDelta, Utc};
        use gt_filter::GlobalFilter;
        use gt_types::{
            FileIdx, GpsTime, Latitude, LoadedTrack, Longitude, NavPoint, TimePositionVelocity,
            TrackIdx, TrackRef,
        };
        use gt_ui_types::{QueryMatches, TrackMatchView};
        use rstest::rstest;
        use uom::si::angle::degree;
        use uom::si::f64::Angle;

        use super::super::TrackGeometry;
        use crate::polyline::{CULL_MARGIN_PX, VisiblePath};
        use crate::tpv_renderer::{self, ChevronFix, TrackIconFade};
        use crate::transform::{GeometryCull, MercTransform, lod_points};
        use crate::viewport::TrackEntry;

        /// The map rect every case frames the fixture in.
        const MAP_RECT: egui::Rect = egui::Rect {
            min: egui::pos2(0.0, 0.0),
            max: egui::pos2(800.0, 600.0),
        };

        fn the_track() -> TrackRef {
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
        }

        const FIRST_FIX_TIME: DateTime<Utc> = DateTime::<Utc>::UNIX_EPOCH;

        const FIX_COUNT: usize = 2_700;

        /// The fixes the receiver dead-reckoned, which the map draws as chevrons.
        const DEAD_RECKONED: Range<usize> = 100..2_600;

        /// The dead-reckoned fixes the receiver wrote while it stood still. A walk
        /// over the finest stored LOD level yields a handful of them: their
        /// spacing sits far below that level's tolerance.
        const PARKED: Range<usize> = 200..2_600;

        /// Longitude between consecutive fixes of the moving stretches, about 13 m
        /// at the fixture's latitude.
        const MOVING_STEP_DEGREES: f64 = 0.000_2;

        /// Longitude between consecutive fixes of the parked stretch, about 6 mm.
        const PARKED_STEP_DEGREES: f64 = 0.000_000_1;

        const LATITUDE_DEGREES: f64 = 55.0;

        const FIRST_LONGITUDE_DEGREES: f64 = 12.0;

        /// The fixture's positions: a stretch east, the parked stretch, then a
        /// stretch east again.
        fn positions() -> Vec<(Latitude, Longitude)> {
            let mut longitude = FIRST_LONGITUDE_DEGREES;
            (0..FIX_COUNT)
                .map(|index| {
                    let position = (Latitude::new(LATITUDE_DEGREES), Longitude::new(longitude));
                    longitude += match PARKED.contains(&index) {
                        true => PARKED_STEP_DEGREES,
                        false => MOVING_STEP_DEGREES,
                    };
                    position
                })
                .collect()
        }

        /// A track of [`FIX_COUNT`] fixes one second apart, with the LOD levels
        /// and chunks the track builder computes for it. The fixes of
        /// [`DEAD_RECKONED`] have no heading, which is what the map draws hollow.
        fn a_track_with_a_dead_reckoned_stretch() -> LoadedTrack {
            let points: Vec<NavPoint> = positions()
                .into_iter()
                .enumerate()
                .map(|(index, (lat, lon))| {
                    let seconds = i64::try_from(index).unwrap_or(i64::MAX);
                    let tpv = TimePositionVelocity::builder()
                        .time(GpsTime::from_utc(
                            FIRST_FIX_TIME + TimeDelta::seconds(seconds),
                        ))
                        .lat(lat)
                        .lon(lon)
                        .maybe_heading(
                            (!DEAD_RECKONED.contains(&index)).then(|| Angle::new::<degree>(90.0)),
                        )
                        .build();
                    NavPoint::new(tpv, None)
                })
                .collect();
            let mut track = gt_test_utils::loaded_track_with_points(points);
            let lod = track.placed_points().map(gt_track_builder::build_track_lod);
            if let Some(lod) = lod {
                track.lod = lod;
            }
            track
        }

        /// The viewports every case walks the fixture in: four map scales over the
        /// first fix, over the parked stretch, and over the last fix. At 2^22 px
        /// per world the walk covers a stored level while the moving stretches are
        /// spaced wide enough for icons to draw.
        fn viewports() -> Vec<MercTransform> {
            let positions = positions();
            let anchors = [
                positions.first().copied(),
                positions.get(PARKED.start + PARKED.len() / 2).copied(),
                positions.last().copied(),
            ];
            [
                2_f64.powi(19),
                2_f64.powi(22),
                2_f64.powi(25),
                2_f64.powi(30),
            ]
            .into_iter()
            .flat_map(|world_px| {
                anchors.into_iter().flatten().map(move |(lat, lon)| {
                    MercTransform::for_test_view(world_px, lat, lon, MAP_RECT.center())
                })
            })
            .collect()
        }

        /// The prepared geometry of `track` as the walk leaves it, holding the LOD
        /// level that walk covered. [`TrackGeometry::chevrons_of`] reads that level
        /// and the track. The other fields are set for a track whose icons draw.
        fn geometry_of<'a>(
            track: &'a LoadedTrack,
            transform: &MercTransform,
            filter: &'a GlobalFilter,
        ) -> TrackGeometry<'a> {
            let placed = track.placed_points().unwrap_or_default();
            let cull = GeometryCull::new(transform, MAP_RECT.expand(CULL_MARGIN_PX), filter);
            TrackGeometry {
                fi: the_track().fi,
                ti: the_track().index,
                track,
                entry: TrackEntry {
                    trackline: true,
                    fade: Some(TrackIconFade::PerFix),
                    sat_labels: false,
                    sky_glyphs: false,
                },
                paint_trackline: true,
                need_blink: false,
                path: VisiblePath::OffScreen,
                walked_level_indices: lod_points(track, placed, transform, cull)
                    .walked_level_indices(),
            }
        }

        /// The chevrons of a walk over the track's LOD level: one per point
        /// the walk yields that the map draws hollow, without the points the
        /// query hides.
        fn chevrons_from_the_geometry_walk(
            track: &LoadedTrack,
            transform: &MercTransform,
            filter: &GlobalFilter,
            query_view: &TrackMatchView<'_>,
        ) -> Vec<(usize, ChevronFix)> {
            let placed = track.placed_points().unwrap_or_default();
            let cull = GeometryCull::new(transform, MAP_RECT.expand(CULL_MARGIN_PX), filter);
            lod_points(track, placed, transform, cull)
                .filter_map(|(pi, point)| Some((pi, ChevronFix::for_fix(point.fix)?)))
                .filter(|&(pi, _)| !query_view.is_hidden(pi))
                .collect()
        }

        /// The fixes the viewport query hands the paint pass: those inside the map
        /// rect the icon pass culls against, in the reverse of fix order,
        /// since the R-tree yields its hits in the order its own nodes hold them.
        fn fixes_the_viewport_query_finds(
            track: &LoadedTrack,
            transform: &MercTransform,
        ) -> Vec<usize> {
            let query_rect = tpv_renderer::icon_cull_rect(MAP_RECT);
            let mut hits: Vec<usize> = track
                .placed_points()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .filter(|(_, point)| query_rect.contains(transform.to_screen(point.merc())))
                .map(|(pi, _)| pi)
                .collect();
            hits.reverse();
            hits
        }

        /// The chevrons of `chevrons` drawn inside the map rect, keeping their
        /// order. Past the rect the two sources part: the walk yields the ends
        /// of a chunk it skips, and the viewport query reaches to the rect the
        /// icon pass culls against.
        fn inside_the_map_rect(
            chevrons: Vec<(usize, ChevronFix)>,
            track: &LoadedTrack,
            transform: &MercTransform,
        ) -> Vec<(usize, ChevronFix)> {
            let placed = track.placed_points().unwrap_or_default();
            chevrons
                .into_iter()
                .filter(|&(pi, _)| {
                    placed
                        .get(pi)
                        .is_some_and(|point| MAP_RECT.contains(transform.to_screen(point.merc())))
                })
                .collect()
        }

        /// A window over the parked stretch, which starts and ends inside it.
        fn a_window_inside_the_parked_stretch() -> GlobalFilter {
            let second = |index: usize| {
                FIRST_FIX_TIME + TimeDelta::seconds(i64::try_from(index).unwrap_or(i64::MAX))
            };
            GlobalFilter {
                time_start: Some(second(PARKED.start + 100)),
                time_end: Some(second(PARKED.end - 100)),
                ..GlobalFilter::default()
            }
        }

        /// A run that hides the first half of the dead-reckoned stretch.
        fn a_query_hiding_half_the_dead_reckoned_stretch() -> QueryMatches {
            let first_half = DEAD_RECKONED.start..DEAD_RECKONED.start + DEAD_RECKONED.len() / 2;
            QueryMatches {
                hidden: iter::once((the_track(), Vec::from([first_half]))).collect(),
                ..QueryMatches::default()
            }
        }

        #[rstest]
        #[case::the_whole_recording(GlobalFilter::default(), QueryMatches::default())]
        #[case::a_time_window(a_window_inside_the_parked_stretch(), QueryMatches::default())]
        #[case::a_query_hiding_points(
            GlobalFilter::default(),
            a_query_hiding_half_the_dead_reckoned_stretch()
        )]
        fn the_icon_pass_draws_the_chevrons_the_geometry_walk_collected(
            #[case] filter: GlobalFilter,
            #[case] matches: QueryMatches,
        ) {
            let track = a_track_with_a_dead_reckoned_stretch();
            let query_view = TrackMatchView::for_track(Some(&matches), the_track());
            let mut chevrons_drawn = 0_usize;
            let mut hits_the_level_drops = 0_usize;

            for transform in viewports() {
                let geometry = geometry_of(&track, &transform, &filter);
                let hits = fixes_the_viewport_query_finds(&track, &transform);
                hits_the_level_drops += hits
                    .iter()
                    .filter(|&&pi| !geometry.level_holds_the_fix_at(pi))
                    .count();
                let from_the_viewport = inside_the_map_rect(
                    geometry.chevrons_of(&hits, &filter, &query_view),
                    &track,
                    &transform,
                );
                let from_the_walk = inside_the_map_rect(
                    chevrons_from_the_geometry_walk(&track, &transform, &filter, &query_view),
                    &track,
                    &transform,
                );
                assert_eq!(
                    from_the_viewport,
                    from_the_walk,
                    "at {} px per world",
                    transform.px_per_merc()
                );
                chevrons_drawn += from_the_viewport.len();
            }

            assert!(chevrons_drawn > 0, "no viewport of the case drew a chevron");
            assert!(
                hits_the_level_drops > 0,
                "no viewport of the case held a fix the walked level drops"
            );
        }
    }
}
