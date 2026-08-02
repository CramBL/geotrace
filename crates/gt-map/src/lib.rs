use egui::{Area, Frame, Grid, Label, RichText, Window};
use egui_phosphor::regular::GLOBE_HEMISPHERE_WEST as ICON_GLOBE_HEMISPHERE_WEST;
use egui_phosphor::regular::MAP_TRIFOLD as ICON_MAP_TRIFOLD;
mod collision_grid;
pub mod display_counts;
mod display_toggle;
pub mod event_marker_renderer;
pub mod generated_marker_renderer;
mod hover_labels;
pub mod icon_mesh;
mod jamming_renderer;
pub mod marker_renderer;
mod polyline;
mod query_match_renderer;
mod sat_labels;
mod scope;
mod sky_glyph_renderer;
mod sky_trails_window;
mod snapped_track_renderer;
#[cfg(test)]
mod test_harness;
pub mod tpv_renderer;
mod track_layers;
pub mod track_renderer;
mod transform;
mod viewport;

pub use sky_trails_window::SkyTrailsWindow;
pub use viewport::GeoBounds;

use egui::Context;

use gt_filter::GlobalFilter;
use gt_jam::dataset::JamDataset;
use gt_types::{DataCategory, FileIdx, LoadedFile, SpatialPoint, TrackRef};
use gt_ui_types::{
    DataPointRef, DisplayCategory, DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility,
    HighlightScope, MapHighlight, PointWindowFolds, QueryMatches, SkyGlyphVariant,
    SkyTrailsRequest, SnappedTracks, TrackDataVisibility,
};
use rstar::PointDistance as _;
use walkers::sources::{Mapbox, MapboxStyle, OpenStreetMap};
use walkers::{HttpTiles, Map, MapMemory};

use crate::event_marker_renderer::EventMarkerRenderer;
use crate::generated_marker_renderer::GeneratedMarkerRenderer;
use crate::hover_labels::{
    draw_disambig_row, draw_multi_hover_label_contents, should_show_compound_label,
};
use crate::marker_renderer::MarkerRenderer;
use crate::snapped_track_renderer::SnappedTrackRenderer;
use crate::track_layers::TrackLayers;
use crate::transform::{MapScale, MercTransform};
use crate::viewport::{
    compute_viewport_bounds, compute_visible_bounding_box, is_spatial_point_visible, zoom_to_fit,
};

/// Which tile source to use for the background map.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum MapLayer {
    #[default]
    OpenStreetMap,
    Satellite,
}

/// Action requested from a right-click context menu on a map element.
///
/// Returned by [`NavMap::draw`] when the user selects an item. The caller is
/// responsible for applying it to the visibility state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapContextAction {
    /// Hide every track except the given one.
    ShowOnlyTrack(TrackRef),
    /// Hide every file except the given one.
    ShowOnlyFile(FileIdx),
    /// Open the sky trails window, per the request's track and instant.
    ShowSkyTrails(SkyTrailsRequest),
}

/// Manages the load-highlight pulse animation.
///
/// Tracks when the animation started and provides the per-frame alpha value.
/// Once expired it clears itself so callers can avoid unnecessary repaints.
/// Timestamps are egui clock seconds (`InputState::time`) rather than wall
/// time, so the pulse is deterministic under `egui_kittest`'s simulated time.
struct BlinkState {
    start: Option<f64>,
}

impl BlinkState {
    fn trigger(&mut self, now: f64) {
        self.start = Some(now);
    }

    /// Returns alpha in `[0.0, 1.0]` for the current frame.
    /// Resets the timer when the animation expires so `is_active` returns
    /// `false` on the same frame the last pulse ends.
    fn tick(&mut self, now: f64) -> f32 {
        let Some(start) = self.start else {
            return 0.0;
        };
        let elapsed = (now - start) as f32;
        if elapsed >= 3.0 {
            self.start = None;
            0.0
        } else {
            // 2 Hz pulsing that fades to zero over 3 s.
            let fade = 1.0 - (elapsed / 3.0);
            (std::f32::consts::TAU * elapsed * 2.0).sin().abs() * fade
        }
    }

    fn is_active(&self) -> bool {
        self.start.is_some()
    }
}

/// Minimum time (seconds) the cursor must hold the same focused track before
/// the fade-in begins.
const HOVER_HYSTERESIS_SEC: f64 = 0.15;
/// Fade-in rate in overlay-progress units per second (0→1 in ≈ 330 ms).
const HOVER_FADE_IN_RATE: f32 = 3.0;
/// Fade-out rate when the overlay is near-opaque (start of departure).
const HOVER_FADE_OUT_SLOW: f32 = 0.1;
/// Fade-out rate when the overlay is near-transparent (end of departure).
/// Combined with `HOVER_FADE_OUT_SLOW` the overlay stays ≈ 78 % visible
/// after 1 s and reaches zero in ≈ 2 s, a quadratic ease-in curve.
const HOVER_FADE_OUT_FAST: f32 = 1.5;

/// Opening size of the clicked-point window, wide enough for the sky plot
/// beside two columns of satellites and tall enough that a typical fix needs
/// no scrolling.
const POINT_WINDOW_DEFAULT_SIZE: [f32; 2] = [600.0, 460.0];
/// Floor for the point window, below which the plot and satellite columns stop
/// fitting side by side. Narrower than this the satellite column is squeezed
/// rather than reflowed - single-column reflow lands with the two-column
/// packing work.
const POINT_WINDOW_MIN_WIDTH_PX: f32 = 340.0;
const POINT_WINDOW_MIN_HEIGHT_PX: f32 = 260.0;

/// Whether this category's sticky window shows the full point layout - the sky
/// plot beside the per-constellation satellite tables. That content is the
/// data-heavy one (a 40-satellite fix is far taller than a marker's few rows),
/// so it gets a resizable frame while the marker popups stay auto-sized.
///
/// The window builder and the body both switch on this, so the frame and the
/// content they choose cannot drift apart. Matched exhaustively rather than
/// with `matches!`, so a new [`DataCategory`] cannot quietly default to the
/// wrong frame - it breaks the build until someone decides which layout it
/// takes.
fn sticky_uses_point_layout(category: DataCategory) -> bool {
    match category {
        DataCategory::Tpv | DataCategory::SatelliteReport => true,
        DataCategory::Track
        | DataCategory::CustomMarker
        | DataCategory::GeneratedMarker
        | DataCategory::EventMarker => false,
    }
}

/// Manages the smooth fade animation for the hover-focus overlay.
///
/// Applies a hysteresis delay before starting the fade, and resets the delay
/// whenever the focused track changes, so fast cursor movement across many
/// tracks never triggers the overlay.
struct HoverFadeState {
    /// Current overlay progress in [0.0, 1.0]. 0 = no overlay, 1 = full overlay.
    progress: f32,
    /// egui clock time when the current focused track was first established.
    /// Cleared when hover ends or the focused track changes.
    hover_since: Option<f64>,
    /// egui clock time of the previous `tick` call, used to compute `dt`.
    prev_time: f64,
    /// The focused track at the last `tick`, used to detect changes.
    focused_track: Option<TrackRef>,
}

impl HoverFadeState {
    /// Advance the animation by one frame and return the current progress.
    ///
    /// - **Track changed while hovering**: resets the delay timer and
    ///   *freezes* `progress`, the overlay holds its current opacity while
    ///   the cursor moves, eliminating the "light-up then dim" oscillation
    ///   between adjacent tracks.
    /// - **Same track held for ≥ [`HOVER_HYSTERESIS_SEC`]**: fades in.
    /// - **Hover ended**: ease-in fade-out starting very slowly and
    ///   accelerating, so the overlay is still ≈ 78 % visible after 1 s
    ///   and fully gone after ≈ 2 s.
    fn tick(&mut self, now: f64, hover_active: bool, current_focused: Option<TrackRef>) -> f32 {
        let dt = (now - self.prev_time).clamp(0.0, 0.1) as f32;
        self.prev_time = now;

        if hover_active {
            if current_focused != self.focused_track {
                // Track changed: restart delay but freeze progress so the
                // overlay does not oscillate while the cursor is moving.
                self.focused_track = current_focused;
                self.hover_since = Some(now);
            } else if self.hover_since.is_none() {
                self.hover_since = Some(now);
            }

            let delay_expired = self
                .hover_since
                .is_some_and(|t| now - t >= HOVER_HYSTERESIS_SEC);
            if delay_expired {
                self.progress = (self.progress + dt * HOVER_FADE_IN_RATE).min(1.0);
            }
            // Delay not expired: keep progress frozen, no fade-in, no fade-out.
        } else {
            self.focused_track = None;
            self.hover_since = None;
            // Variable-rate ease-in: slow start → fast end over ≈ 2 s.
            let rate = HOVER_FADE_OUT_SLOW
                + (HOVER_FADE_OUT_FAST - HOVER_FADE_OUT_SLOW) * (1.0 - self.progress);
            self.progress = (self.progress - dt * rate).max(0.0);
        }

        self.progress
    }

    fn is_animating(&self) -> bool {
        self.progress > 0.0 || self.hover_since.is_some()
    }
}

impl Default for HoverFadeState {
    fn default() -> Self {
        Self {
            progress: 0.0,
            hover_since: None,
            prev_time: 0.0,
            focused_track: None,
        }
    }
}

pub struct NavMap {
    egui_ctx: Context,
    osm_tiles: HttpTiles,
    mapbox_tiles: Option<HttpTiles>,
    mapbox_token: String,
    layer: MapLayer,
    map_memory: MapMemory,
    global_tree: rstar::RTree<SpatialPoint>,
    /// Screen position where the last sticky click happened, used as the
    /// default position for the sticky info window.
    sticky_pos: egui::Pos2,
    /// How many files were loaded last frame - used to detect new loads.
    last_file_count: usize,
    /// Load-highlight pulse animation state.
    blink: BlinkState,
    /// Index of the first newly loaded file. files[new_file_boundary..] are new.
    new_file_boundary: usize,
    /// Smooth fade animation for the hover-focus overlay.
    hover_fade: HoverFadeState,
    /// Geographic bounds of the last rendered viewport.
    /// `None` before the first draw call.
    last_viewport_bounds: Option<GeoBounds>,
    /// The element that was under the pointer when the last right-click fired.
    /// Held across frames so the context menu can reference it while it is open.
    right_click_ref: Option<DataPointRef>,
    /// Candidates captured at the last click that had multiple overlapping types.
    /// Displayed in a disambiguation popup until the user picks one or clicks elsewhere.
    disambiguation_candidates: [Option<DataPointRef>; 4],
    /// Screen position where the disambiguation popup is anchored.
    disambiguation_pos: egui::Pos2,
    /// Session-only state of the display toggle (popup open, solo restore).
    display_toggle: display_toggle::DisplayToggleState,
    /// Memoized per-category counts for the display-toggle popup, so an open
    /// popup does not re-walk every point each frame.
    display_counts_cache: display_counts::DisplayCountsCache,
    /// Pre-tessellated marker icon meshes decoded from the embedded blob.
    /// `None` only if the embedded data is corrupted (reported at startup);
    /// marker icons are then skipped.
    icon_meshes: Option<icon_mesh::IconMeshLibrary>,
    /// Per-frame viewport point collection, reused across frames so a steady
    /// stream of frames avoids reallocating the category buffers each time.
    visible_points: viewport::VisiblePoints,
    /// Reused satellite-label decimation scratch, borrowed into the track
    /// layer each frame so the candidate and output buffers persist.
    sat_label_scratch: sat_labels::LabelSelection,
    /// Reused sky-glyph decimation scratch, borrowed into the track layer.
    sky_glyph_scratch: sky_glyph_renderer::GlyphSelection,
}

impl NavMap {
    pub fn new(egui_ctx: Context) -> Self {
        let icon_meshes = match icon_mesh::IconMeshLibrary::embedded() {
            Ok(library) => Some(library),
            Err(err) => {
                log::error!("icon meshes unavailable, marker icons will not be drawn: {err:#}");
                None
            }
        };
        Self {
            osm_tiles: HttpTiles::new(OpenStreetMap, egui_ctx.clone()),
            mapbox_tiles: None,
            mapbox_token: String::new(),
            layer: MapLayer::default(),
            map_memory: MapMemory::default(),
            egui_ctx,
            global_tree: rstar::RTree::new(),
            sticky_pos: egui::pos2(100.0, 100.0),
            last_file_count: 0,
            blink: BlinkState { start: None },
            new_file_boundary: 0,
            hover_fade: HoverFadeState::default(),
            last_viewport_bounds: None,
            right_click_ref: None,
            disambiguation_candidates: [None; 4],
            disambiguation_pos: egui::pos2(0.0, 0.0),
            display_toggle: display_toggle::DisplayToggleState::default(),
            display_counts_cache: display_counts::DisplayCountsCache::default(),
            icon_meshes,
            visible_points: viewport::VisiblePoints::default(),
            sat_label_scratch: sat_labels::LabelSelection::default(),
            sky_glyph_scratch: sky_glyph_renderer::GlyphSelection::default(),
        }
    }

    /// Return the geographic bounds of the most recently rendered map viewport.
    ///
    /// Returns `None` before the first call to [`Self::draw`].
    pub fn viewport_geo_bounds(&self) -> Option<GeoBounds> {
        self.last_viewport_bounds
    }

    /// Set (or clear) the Mapbox API token. Passing an empty string clears the token
    /// and falls back to OpenStreetMap for satellite mode.
    pub fn set_mapbox_token(&mut self, token: String) {
        if token.is_empty() {
            self.mapbox_token = String::new();
            self.mapbox_tiles = None;
        } else {
            self.mapbox_tiles = Some(HttpTiles::new(
                Mapbox {
                    style: MapboxStyle::Satellite,
                    high_resolution: false,
                    access_token: token.clone(),
                },
                self.egui_ctx.clone(),
            ));
            self.mapbox_token = token;
        }
    }

    pub fn mapbox_token(&self) -> &str {
        &self.mapbox_token
    }

    pub fn has_mapbox_token(&self) -> bool {
        !self.mapbox_token.is_empty()
    }

    pub fn layer(&self) -> MapLayer {
        self.layer
    }

    pub fn set_layer(&mut self, layer: MapLayer) {
        self.layer = layer;
    }

    /// Rebuild the global spatial index from the current file list.
    ///
    /// Must be called after any structural change to `loaded_files` (file or
    /// track deletion) to prevent stale R-tree entries from causing out-of-bounds
    /// panics in the renderers.
    pub fn rebuild_spatial_index(&mut self, files: &[LoadedFile]) {
        self.global_tree = gt_track_builder::build_global_tree(files);
        self.last_file_count = files.len();
    }

    /// Returns `true` when every entry in the spatial index is in-bounds for
    /// the given file list. Used in tests to verify the index is not stale.
    #[cfg(test)]
    pub(crate) fn all_tree_indices_valid(&self, files: &[LoadedFile]) -> bool {
        use gt_types::DataCategory;
        self.global_tree.iter().all(|sp| {
            let Some(file) = sp.file_index.get(files) else {
                return false;
            };
            let Some(track) = sp.track_index.get(&file.tracks) else {
                return false;
            };
            let len = match sp.category {
                DataCategory::Tpv => track.points.len(),
                DataCategory::CustomMarker => track.custom_markers.len(),
                DataCategory::GeneratedMarker => track.generated_markers.len(),
                DataCategory::EventMarker => track.event_markers.len(),
                // These categories are never inserted into the spatial index.
                DataCategory::Track | DataCategory::SatelliteReport => return true,
            };
            sp.point_index.as_usize() < len
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "draw context requires all parameters; a wrapper struct would not add clarity"
    )]
    #[expect(
        clippy::cognitive_complexity,
        reason = "draw orchestrates viewport query, plugin wiring, hover, click, and popup - splitting would obscure the data flow"
    )]
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        files: &[LoadedFile],
        visibility: &TrackDataVisibility,
        highlight: &mut MapHighlight,
        filter: &GlobalFilter,
        display_mask: &mut DisplayMask,
        sky_glyph_variant: &mut SkyGlyphVariant,
        point_window_folds: &mut PointWindowFolds,
        event_marker_visibility: &EventMarkerVisibility,
        generated_marker_visibility: &GeneratedMarkerVisibility,
        query_matches: Option<&QueryMatches>,
        snapped_tracks: Option<&SnappedTracks>,
        jamming_dataset: Option<&JamDataset>,
        center_request: Option<(f64, f64)>,
        zoom_to_visible: bool,
        sticky_pos_override: Option<egui::Pos2>,
    ) -> Option<MapContextAction> {
        if let Some((lat, lon)) = center_request {
            self.map_memory.center_at(walkers::lat_lon(lat, lon));
        }

        if let Some(pos) = sticky_pos_override {
            self.sticky_pos = pos;
        }

        if zoom_to_visible
            && let Some(bbox) =
                compute_visible_bounding_box(files, visibility, filter, *display_mask)
        {
            zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
        }

        let now = ui.ctx().input(|i| i.time);

        // Detect newly loaded files → zoom to fit the visible tracks + start
        // blink animation. The new file is visible by default, so it is always
        // framed. Existing files honor their current visibility.
        if files.len() > self.last_file_count {
            self.new_file_boundary = self.last_file_count;
            let had_tracks = self.last_file_count > 0;
            self.last_file_count = files.len();
            // Only blink when adding to existing content. The first load needs no
            // visual callout because there is nothing else to differentiate from.
            if had_tracks {
                self.blink.trigger(now);
            }
            if let Some(bbox) =
                compute_visible_bounding_box(files, visibility, filter, *display_mask)
            {
                zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
            }
            self.global_tree = gt_track_builder::build_global_tree(files);
        }

        let blink_alpha = self.blink.tick(now);
        if self.blink.is_active() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }

        // Advance the hover-fade animation using the previous frame's highlight,
        // which is what the renderers will see this frame.
        let hover_fade_progress = if highlight.fading_enabled {
            let progress = self.hover_fade.tick(
                now,
                track_renderer::hover_is_active(highlight),
                track_renderer::focused_track_from_highlight(highlight),
            );
            if self.hover_fade.is_animating() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }
            progress
        } else {
            0.0
        };

        // Suppress individual renderer hover labels when:
        // - the disambiguation popup is currently open (it occupies the cursor area), or
        // - multiple hover candidates were active last frame (the map layer draws a single
        //   stacked label instead, avoiding overlapping labels from independent renderers).
        let disambig_open = self.disambiguation_candidates.iter().any(|c| c.is_some());
        let prev_multi_hover = highlight.hover_candidates.iter().flatten().count() > 1;
        highlight.suppress_hover_labels = disambig_open || prev_multi_hover;

        let map_rect_estimate = ui.max_rect();
        let map_center = self
            .map_memory
            .detached()
            .unwrap_or_else(|| walkers::lat_lon(55.676, 12.565));
        let projector_estimate =
            walkers::Projector::new(map_rect_estimate, &self.map_memory, map_center);
        let transform_estimate = MercTransform::new(
            &projector_estimate,
            &self.map_memory,
            map_rect_estimate.center(),
        );

        // All per-track drawing decisions for this frame, derived once and
        // shared by viewport collection and the renderers.
        let plan = viewport::TrackPlan::compute(
            files,
            visibility,
            filter,
            *display_mask,
            self.map_memory.zoom(),
        );
        viewport::collect_visible_points(
            &mut self.visible_points,
            &self.global_tree,
            &plan,
            &transform_estimate,
            map_rect_estimate,
        );

        let is_offline = gt_types::env::offline();
        let map = if is_offline {
            Map::new(None, &mut self.map_memory, walkers::lat_lon(55.676, 12.565))
        } else {
            // Mapbox serves 512px tiles, and walkers' `tile_id` adjusts the
            // integer zoom level by `log2(tile_size / 256)` - 1 for 512px
            // tiles - by plain `u8` subtraction with no underflow check
            // (walkers 0.53.0 `mercator::tile_id`, src/mercator.rs:50). That
            // panics with "attempt to subtract with overflow" once the zoom
            // rounds down to 0, i.e. once it drops below 0.5. OSM's 256px
            // tiles need no adjustment (`log2(256/256) == 0`) and so are
            // immune at any zoom. Stay on OSM with enough margin below that
            // line that no single frame's zoom delta can cross it.
            const MAPBOX_MIN_SAFE_ZOOM: f64 = 2.0;
            let use_mapbox = self.layer == MapLayer::Satellite
                && self.mapbox_tiles.is_some()
                && self.map_memory.zoom() >= MAPBOX_MIN_SAFE_ZOOM;
            if use_mapbox {
                let tiles: Option<&mut dyn walkers::Tiles> = self.mapbox_tiles.as_mut().map(|t| {
                    let r: &mut dyn walkers::Tiles = t;
                    r
                });
                Map::new(
                    tiles,
                    &mut self.map_memory,
                    walkers::lat_lon(55.676, 12.565),
                )
            } else {
                let tiles: &mut dyn walkers::Tiles = &mut self.osm_tiles;
                Map::new(
                    Some(tiles),
                    &mut self.map_memory,
                    walkers::lat_lon(55.676, 12.565),
                )
            }
        };
        // The interference overlay goes on first so every track renderer
        // draws over it.
        let mut map = map;
        if display_mask.is_visible(DisplayCategory::JammingHexes)
            && let Some(dataset) = jamming_dataset
        {
            // Hover yields while a track element owns the pointer, matching
            // the snapped-track renderer.
            map = map.with_plugin(jamming_renderer::JammingRenderer::new(
                dataset,
                highlight.hover.is_none(),
            ));
        }
        // A masked display category skips its whole plugin - the mask is
        // the render-side AND on top of the per-track tree visibility the
        // renderers already consume.
        map = map.with_plugin(
            TrackLayers::builder()
                .files(files)
                .plan(&plan)
                .highlight(highlight)
                .filter(filter)
                .tpv_by_track(&self.visible_points.tpv_by_track)
                .new_file_boundary(self.new_file_boundary)
                .blink_alpha(blink_alpha)
                .hover_fade_alpha(hover_fade_progress)
                .maybe_query_matches(query_matches)
                .display_query_highlights(display_mask.is_visible(DisplayCategory::QueryHighlights))
                .sky_glyph_variant(*sky_glyph_variant)
                .maybe_icon_meshes(self.icon_meshes.as_ref())
                .sat_label_scratch(&mut self.sat_label_scratch)
                .sky_glyph_scratch(&mut self.sky_glyph_scratch)
                .build(),
        );
        if display_mask.is_visible(DisplayCategory::SnappedTracks)
            && let Some(snapped) = snapped_tracks
            && !snapped.is_empty()
        {
            // Edge hover yields while the recorded data owns the pointer.
            map = map.with_plugin(SnappedTrackRenderer::new(
                snapped,
                files,
                highlight.hover.is_none(),
            ));
        }
        if display_mask.is_visible(DisplayCategory::CustomMarkers) {
            map = map.with_plugin(MarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                &self.visible_points.custom,
                self.icon_meshes.as_ref(),
            ));
        }
        if display_mask.is_visible(DisplayCategory::GeneratedMarkers) {
            map = map.with_plugin(GeneratedMarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                generated_marker_visibility,
                &self.visible_points.generated,
                self.icon_meshes.as_ref(),
            ));
        }
        if display_mask.is_visible(DisplayCategory::EventMarkers) {
            map = map.with_plugin(EventMarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                event_marker_visibility,
                &self.visible_points.event,
                self.icon_meshes.as_ref(),
            ));
        }

        let map_response = ui.add(map);

        // Double-click anywhere on the map: zoom out to fit the visible tracks.
        if map_response.double_clicked()
            && let Some(bbox) =
                compute_visible_bounding_box(files, visibility, filter, *display_mask)
        {
            zoom_to_fit(&mut self.map_memory, map_response.rect, bbox);
        }

        // Compute and cache the current viewport's geographic bounds so callers
        // can query them via `viewport_geo_bounds()` after each draw call.
        let map_rect = map_response.rect;
        self.last_viewport_bounds = Some(compute_viewport_bounds(&self.map_memory, map_rect));

        // Recompute the transform from the actual map rect for accurate hover detection.
        let projector_actual = walkers::Projector::new(map_rect, &self.map_memory, map_center);
        let transform = MercTransform::new(&projector_actual, &self.map_memory, map_rect.center());
        // Collect the nearest visible candidate per category group within the
        // hover threshold. Slot 0 = Tpv/SatelliteReport, 1 = EventMarker,
        // 2 = CustomMarker, 3 = GeneratedMarker (matches DataCategory::hover_slot).
        let mut hover_candidates: [Option<DataPointRef>; 4] = [None; 4];
        let hover_point_ref: Option<DataPointRef> = if map_response.hovered() {
            ui.input(|i| i.pointer.hover_pos()).and_then(|screen_pos| {
                let merc_x = transform.merc_x_from_screen(screen_pos.x);
                let merc_y = transform.merc_y_from_screen(screen_pos.y);
                let px_per_merc = MapScale::from_zoom(self.map_memory.zoom()).px_per_merc();
                let threshold_merc_sq = (20.0_f64 / px_per_merc).powi(2);
                // First pass: one candidate per slot, in nearest-first order.
                for sp in self
                    .global_tree
                    .nearest_neighbor_iter([merc_x, merc_y])
                    .take_while(|sp| sp.distance_2(&[merc_x, merc_y]) <= threshold_merc_sq)
                {
                    if !is_spatial_point_visible(
                        sp,
                        files,
                        visibility,
                        filter,
                        *display_mask,
                        query_matches,
                    ) {
                        continue;
                    }
                    if let Some(slot) = sp.category.hover_slot() {
                        #[expect(
                            clippy::indexing_slicing,
                            reason = "hover_slot() returns 0..=3; array has 4 elements"
                        )]
                        hover_candidates[slot].get_or_insert_with(|| DataPointRef {
                            track: sp.track_ref(),
                            category: sp.category,
                            point_index: sp.point_index,
                        });
                    }
                    if hover_candidates.iter().all(|c| c.is_some()) {
                        break;
                    }
                }
                // Primary hover: Tpv wins if present, otherwise the first non-None slot.
                hover_candidates.iter().flatten().copied().next()
            })
        } else {
            None
        };

        // Layer toggle - floating panel anchored to the bottom-right of the map.
        let layer_toggle = Area::new(egui::Id::new("map_layer_toggle"))
            .fixed_pos(egui::pos2(map_rect.right() - 8.0, map_rect.bottom() - 8.0))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    for (layer, icon, label) in [
                        (MapLayer::OpenStreetMap, ICON_MAP_TRIFOLD, "Map"),
                        (MapLayer::Satellite, ICON_GLOBE_HEMISPHERE_WEST, "Satellite"),
                    ] {
                        let selected = self.layer == layer;
                        if ui
                            .selectable_label(selected, format!("{icon} {label}"))
                            .clicked()
                        {
                            self.layer = layer;
                        }
                    }
                });
            });

        // Display toggle - the eye button stacked above the layer toggle. The
        // counts closure only runs while the popup is open, and the cache skips
        // the full point walk when its inputs are unchanged frame to frame.
        let counts_cache = &mut self.display_counts_cache;
        display_toggle::show_display_toggle(
            ui,
            layer_toggle.response.rect,
            &mut self.display_toggle,
            display_mask,
            sky_glyph_variant,
            || {
                counts_cache.get(
                    files,
                    visibility,
                    filter,
                    event_marker_visibility,
                    generated_marker_visibility,
                    query_matches,
                    display_counts::SuppliedCounts {
                        snapped_tracks,
                        jamming_cells: jamming_dataset.map_or(0, JamDataset::len),
                    },
                )
            },
        );

        // Clicking near a map element makes its info popup sticky, clicking on
        // empty space clears it. Clicking the same element again also clears it.
        // When multiple category types are within the threshold, show a small
        // disambiguation menu rather than immediately picking one.
        let candidate_count = hover_candidates.iter().flatten().count();
        // True only on the frame the popup is first opened. `clicked_elsewhere()` on the
        // disambiguation area fires on the same frame as the click that opened it (the click
        // was on the map, not inside the popup), which would immediately close the popup.
        // Skipping that check for the opening frame prevents the flash.
        let just_opened_disambig = if map_response.clicked() {
            if candidate_count > 1 {
                self.disambiguation_candidates = hover_candidates;
                self.disambiguation_pos = ui
                    .ctx()
                    .pointer_latest_pos()
                    .unwrap_or(map_response.rect.center());
                true
            } else if let Some(point_ref) = hover_point_ref {
                self.disambiguation_candidates = [None; 4];
                if highlight.sticky == Some(point_ref) {
                    highlight.sticky = None;
                } else {
                    highlight.sticky = Some(point_ref);
                    self.sticky_pos = ui
                        .ctx()
                        .pointer_latest_pos()
                        .unwrap_or(map_response.rect.center());
                }
                false
            } else {
                self.disambiguation_candidates = [None; 4];
                highlight.sticky = None;
                false
            }
        } else {
            false
        };

        // Disambiguation popup: shown after a click when multiple types overlap.
        let disambig_candidates = self.disambiguation_candidates;
        if disambig_candidates.iter().any(|c| c.is_some()) {
            let disambig_pos = self.disambiguation_pos;
            let area_resp = Area::new(egui::Id::new("map_disambig"))
                .fixed_pos(disambig_pos)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(160.0);
                        for candidate in disambig_candidates.iter().flatten().copied() {
                            if draw_disambig_row(
                                ui,
                                candidate,
                                files,
                                highlight.sticky == Some(candidate),
                            )
                            .clicked()
                            {
                                if highlight.sticky == Some(candidate) {
                                    highlight.sticky = None;
                                } else {
                                    highlight.sticky = Some(candidate);
                                    self.sticky_pos = disambig_pos;
                                }
                                self.disambiguation_candidates = [None; 4];
                            }
                        }
                    });
                });
            if !just_opened_disambig
                && (area_resp.response.clicked_elsewhere()
                    || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)))
            {
                self.disambiguation_candidates = [None; 4];
            }
        }

        // Right-click context menu: capture the hovered element on the frame
        // the right button fires, then hold it for the lifetime of the menu.
        if map_response.secondary_clicked() {
            self.right_click_ref = hover_point_ref;
        }
        let right_click_ref = self.right_click_ref;
        let mut context_action: Option<MapContextAction> = None;
        map_response.context_menu(|ui| {
            let Some(point_ref) = right_click_ref else {
                // Right-clicked on empty map space - nothing to show.
                ui.close();
                return;
            };
            let Some(file) = point_ref.track.fi.get(files) else {
                ui.close();
                return;
            };
            ui.add(Label::new(
                RichText::new(file.metadata.filename.as_str()).weak(),
            ));
            if file.tracks.len() > 1 {
                ui.add(Label::new(
                    RichText::new(format!("#{}", point_ref.track.index.as_usize() + 1)).weak(),
                ));
            }
            ui.separator();
            if ui.button("Only show elements from this track").clicked() {
                context_action = Some(MapContextAction::ShowOnlyTrack(point_ref.track));
                ui.close();
            }
            if ui.button("Only show elements from this file").clicked() {
                context_action = Some(MapContextAction::ShowOnlyFile(point_ref.track.fi));
                ui.close();
            }
            ui.separator();
            if ui.button("Show sky trails…").clicked() {
                context_action = Some(MapContextAction::ShowSkyTrails(
                    SkyTrailsRequest::whole_track(point_ref.track),
                ));
                ui.close();
            }
        });

        // Escape dismisses the sticky info window. This is the single window
        // shared by every map item type (TPV, satellite, markers), so handling
        // it here covers them all.
        if highlight.sticky.is_some() && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            highlight.sticky = None;
        }

        // Show a persistent, text-selectable info window for the sticky element.
        if let Some(sticky_ref) = highlight.sticky
            && let Some(request) = show_sticky_popup(
                ui.ctx(),
                files,
                sticky_ref,
                self.sticky_pos,
                point_window_folds,
            )
        {
            context_action = Some(MapContextAction::ShowSkyTrails(request));
        }

        // When multiple different item types are hovered simultaneously, draw a
        // single stacked label near the cursor instead of letting each renderer
        // place its own label (which would all overlap at the same spot).
        //
        // Guard on `suppress_hover_labels` (set from the previous frame's candidate
        // count) so that on the first frame of a multi-hover transition the
        // individual renderer tooltips show normally. From the second frame onward
        // `suppress_hover_labels` is true, individual tooltips are suppressed, and
        // the compound label takes over, preventing the two from appearing at once.
        let current_multi_hover = hover_candidates.iter().flatten().count() > 1;
        if should_show_compound_label(
            current_multi_hover,
            disambig_open,
            highlight.suppress_hover_labels,
        ) && let Some(cursor_pos) = ui.input(|i| i.pointer.hover_pos())
        {
            Area::new(egui::Id::new("map_multi_hover_labels"))
                .fixed_pos(cursor_pos + egui::vec2(15.0, 10.0))
                .order(egui::Order::Tooltip)
                .show(ui.ctx(), |ui| {
                    draw_multi_hover_label_contents(ui, &hover_candidates, files);
                });
        }

        highlight.hover = hover_point_ref.map(HighlightScope::Point);
        highlight.hover_candidates = hover_candidates;

        context_action
    }
}

/// Shows a draggable, text-selectable egui window with data for the given sticky element.
/// The clicked-point window body: the deselect hint pinned to the window floor
/// as an inner bottom panel, with the plot-and-satellites content filling the
/// space above it. Same inner-panel idiom the side panel uses to pin its
/// progress strip above a scrolling tree, so the satellite table scrolls
/// inside the remaining space rather than the whole body scrolling.
/// Returns whether the user asked to open the sky trails window at this
/// point's instant.
#[must_use]
fn show_point_window_body(
    ui: &mut egui::Ui,
    point: &gt_types::NavPoint,
    sky: &crate::tpv_renderer::SkySection<'_>,
    folds: &mut PointWindowFolds,
) -> bool {
    egui::Panel::bottom("sticky_point_hint").show_inside(ui, |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("Click to deselect").small().weak());
    });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show_inside(ui, |ui| {
            crate::tpv_renderer::show_sticky_tpv_content(ui, point, sky, folds)
        })
        .inner
}

/// Returns a request to open the sky trails window when the point window's
/// header button was pressed, carrying the clicked point's instant.
#[must_use]
fn show_sticky_popup(
    ctx: &egui::Context,
    files: &[LoadedFile],
    sticky_ref: DataPointRef,
    default_pos: egui::Pos2,
    folds: &mut PointWindowFolds,
) -> Option<SkyTrailsRequest> {
    // For TPV points, satellite reports, and generated-marker events the window
    // title is the point's datetime. For everything else fall back to a generic label.
    let title: String = if sticky_ref.category == DataCategory::Tpv {
        sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.points))
            .map_or_else(
                || "GNSS fix".to_string(),
                |p| p.tpv.time().utc().format("%Y-%m-%d %H:%M:%S").to_string(),
            )
    } else if sticky_ref.category == DataCategory::SatelliteReport {
        sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.points))
            .and_then(|p| p.satellites.as_ref())
            .map_or_else(
                || "Satellite report".to_string(),
                |sats| {
                    sats.best_time().map_or_else(
                        || "Satellite report".to_string(),
                        |t| t.format("%Y-%m-%d %H:%M:%S").to_string(),
                    )
                },
            )
    } else if sticky_ref.category == DataCategory::GeneratedMarker {
        sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.generated_markers))
            .map_or_else(
                || "GNSS event".to_string(),
                |m| m.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            )
    } else if sticky_ref.category == DataCategory::EventMarker {
        sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.event_markers))
            .map_or_else(
                || "Event".to_string(),
                |m| m.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            )
    } else {
        // CustomMarker
        sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.custom_markers))
            .map_or_else(
                || "Custom marker".to_string(),
                |m| m.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            )
    };

    let window = Window::new(title)
        .id(egui::Id::new(("sticky_popup", sticky_ref)))
        .default_pos(default_pos)
        .collapsible(false);
    let window = if sticky_uses_point_layout(sticky_ref.category) {
        window
            .resizable(true)
            .default_size(POINT_WINDOW_DEFAULT_SIZE)
            .min_width(POINT_WINDOW_MIN_WIDTH_PX)
            .min_height(POINT_WINDOW_MIN_HEIGHT_PX)
    } else {
        window.auto_sized()
    };
    let mut trails_request = None;
    window.show(ctx, |ui| match sticky_ref.category {
        // Both carry the same point content, so they share one arm - and with
        // it the resizable frame that `sticky_uses_point_layout` selects.
        DataCategory::Tpv | DataCategory::SatelliteReport => {
            if let Some(track) = sticky_ref
                .track
                .fi
                .get(files)
                .and_then(|f| sticky_ref.track.index.get(&f.tracks))
                && let Some(point) = sticky_ref.point_index.get(&track.points)
            {
                let sky = crate::tpv_renderer::SkySection::resolve(track, sticky_ref.point_index);
                if show_point_window_body(ui, point, &sky, folds) {
                    trails_request = Some(SkyTrailsRequest::at_instant(
                        sticky_ref.track,
                        point.tpv.time(),
                    ));
                }
            }
        }
        DataCategory::CustomMarker => {
            if let Some(marker) = sticky_ref
                .track
                .fi
                .get(files)
                .and_then(|f| sticky_ref.track.index.get(&f.tracks))
                .and_then(|t| sticky_ref.point_index.get(&t.custom_markers))
            {
                Grid::new("sticky_marker_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Label");
                        ui.add(Label::new(marker.label.as_str()).selectable(true));
                        ui.end_row();
                    });
                ui.add_space(4.0);
                ui.label(RichText::new("Click to deselect").small().weak());
            }
        }
        DataCategory::GeneratedMarker => {
            if let Some(marker) = sticky_ref
                .track
                .fi
                .get(files)
                .and_then(|f| sticky_ref.track.index.get(&f.tracks))
                .and_then(|t| sticky_ref.point_index.get(&t.generated_markers))
            {
                // The window title already shows the time, so the body adds
                // what hovering shows plus the position: the event summary,
                // the position, and (for a slip) the per-satellite table.
                let header =
                    crate::generated_marker_renderer::generated_marker_header(&marker.kind);
                Grid::new("sticky_gen_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Event");
                    ui.add(Label::new(header).selectable(true));
                    ui.end_row();
                    ui.label("Position");
                    ui.add(
                        Label::new(format!(
                            "{:.6}, {:.6}",
                            marker.lat.as_degrees(),
                            marker.lon.as_degrees()
                        ))
                        .selectable(true),
                    );
                    ui.end_row();
                });
                if let gt_types::GeneratedMarkerKind::Slip(event) = &marker.kind {
                    ui.add_space(4.0);
                    crate::generated_marker_renderer::show_slip_table(ui, event);
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Click to deselect").small().weak());
            }
        }
        DataCategory::Track => {}
        DataCategory::EventMarker => {
            if let Some(marker) = sticky_ref
                .track
                .fi
                .get(files)
                .and_then(|f| sticky_ref.track.index.get(&f.tracks))
                .and_then(|t| sticky_ref.point_index.get(&t.event_markers))
            {
                Grid::new("sticky_event_marker_grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label("Event");
                        ui.add(Label::new(marker.variant_path.as_str()).selectable(true));
                        ui.end_row();
                        if let Some(ann) = &marker.annotation {
                            ui.label("Note");
                            ui.add(Label::new(ann.as_str()).selectable(true));
                            ui.end_row();
                        }
                    });
                ui.add_space(4.0);
                ui.label(RichText::new("Click to deselect").small().weak());
            }
        }
    });
    trails_request
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::hover_labels::candidate_label;
    use gt_test_utils::nav_test_data;
    use gt_types::{
        Coord, DataCategory, FileIdx, FileMetadata, LoadedFile, LoadedTrack, MercPoint, PointIdx,
        Rect, SpatialPoint, TimeRange, TrackIdx, TrackMetadata, merc_bounds_for_rect,
    };
    use gt_ui_types::{FileVisibility, TrackVisibility};
    use uom::si::f64::Length;
    use uom::si::length::{kilometer, meter};

    fn make_file_from_points(points: Vec<gt_types::NavPoint>) -> LoadedFile {
        let now = chrono::Utc::now();
        let bb = Rect::new(
            Coord {
                x: 12.55f64,
                y: 55.67,
            },
            Coord {
                x: 12.59f64,
                y: 55.69,
            },
        );
        let n = points.len();
        let track = LoadedTrack {
            metadata: TrackMetadata {
                index: 0,
                distance_km: Length::new::<kilometer>(1.0),
                duration: chrono::Duration::seconds(n as i64),
                time_range: TimeRange::new(now, now + chrono::Duration::seconds(n as i64)),
                bounding_box: bb,
                merc_bounds: merc_bounds_for_rect(bb),
                point_set_diameter_m: Length::new::<meter>(100.0),
                has_custom_markers: false,
                tpv_count: n,
                satellite_report_count: 0,
                custom_marker_count: 0,
                generated_marker_count: 0,
                event_marker_count: 0,
                ..TrackMetadata::default()
            },
            points,
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
            channels: vec![],
        };
        LoadedFile {
            metadata: FileMetadata {
                filename: format!("test_{n}.gtd"),
                total_distance_km: Length::new::<kilometer>(1.0),
                total_duration: chrono::Duration::seconds(n as i64),
                time_range: TimeRange::new(now, now + chrono::Duration::seconds(n as i64)),
                ..FileMetadata::default()
            },
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(PathBuf::from(format!("test_{n}.gtd"))),
            load_warnings: vec![],
        }
    }

    fn tpv_spatial_point(fi: usize, ti: usize, pi: usize) -> SpatialPoint {
        SpatialPoint {
            merc: MercPoint { x: 0.5, y: 0.5 },
            file_index: FileIdx::new(fi),
            track_index: TrackIdx::new(ti),
            point_index: PointIdx::new(pi),
            category: DataCategory::Tpv,
        }
    }

    fn vis_all_visible() -> TrackDataVisibility {
        TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![TrackVisibility::all_visible()],
            }],
        }
    }

    /// Regression test: a point in a visible track must be hoverable.
    #[test]
    fn visible_tpv_point_is_hoverable() {
        let sp = tpv_spatial_point(0, 0, 0);
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let vis = vis_all_visible();
        assert!(is_spatial_point_visible(
            &sp,
            &files,
            &vis,
            &GlobalFilter::default(),
            DisplayMask::default(),
            None
        ));
    }

    /// Regression test: hiding the file must prevent hover on all its points.
    #[test]
    fn hidden_file_blocks_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let mut vis = vis_all_visible();
        vis.files[0].enabled = false;
        assert!(!is_spatial_point_visible(
            &sp,
            &files,
            &vis,
            &GlobalFilter::default(),
            DisplayMask::default(),
            None
        ));
    }

    /// Regression test: hiding the track must prevent hover even when the file is visible.
    #[test]
    fn hidden_track_blocks_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let mut vis = vis_all_visible();
        vis.files[0].tracks[0].enabled = false;
        assert!(!is_spatial_point_visible(
            &sp,
            &files,
            &vis,
            &GlobalFilter::default(),
            DisplayMask::default(),
            None
        ));
    }

    fn track_at(lat: f64, lon: f64) -> LoadedTrack {
        let tpv = gt_types::TimePositionVelocity::builder()
            .time(gt_types::GpsTime::from_utc(chrono::Utc::now()))
            .lat(gt_types::Latitude::new(lat))
            .lon(gt_types::Longitude::new(lon))
            .build();
        LoadedTrack {
            metadata: TrackMetadata::default(),
            points: vec![gt_types::NavPoint::new(tpv, None)],
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
            channels: vec![],
        }
    }

    fn file_with_tracks(tracks: Vec<LoadedTrack>) -> LoadedFile {
        LoadedFile {
            metadata: FileMetadata::default(),
            tracks,
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(PathBuf::from("test.gtd")),
            load_warnings: vec![],
        }
    }

    /// Regression test: "zoom to fit" frames only the visible tracks. Hiding a
    /// track must drop its corner from `compute_visible_bounding_box`.
    #[test]
    fn visible_bounding_box_excludes_hidden_tracks() {
        // Track 0 sits south-west, track 1 sits far north-east.
        let files = vec![file_with_tracks(vec![
            track_at(55.0, 12.0),
            track_at(56.0, 13.0),
        ])];
        let mut vis = TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![
                    TrackVisibility::all_visible(),
                    TrackVisibility::all_visible(),
                ],
            }],
        };

        // Everything visible: the box spans both tracks (min_lat, max_lat, min_lon, max_lon).
        let filter = GlobalFilter::default();
        let all_visible =
            compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
                .expect("visible data has a bbox");
        assert_eq!(all_visible, (55.0, 56.0, 12.0, 13.0));

        // Hide the north-east track: its corner drops out of the box.
        vis.files[0].tracks[1].enabled = false;
        let only_first =
            compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
                .expect("track 0 still visible");
        assert_eq!(only_first, (55.0, 55.0, 12.0, 12.0));
    }

    /// Regression test: turning off the TPV layer must prevent hover on TPV points.
    #[test]
    fn hidden_tpv_layer_blocks_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let mut vis = vis_all_visible();
        vis.files[0].tracks[0].set_category_visible(DataCategory::Tpv, false);
        assert!(!is_spatial_point_visible(
            &sp,
            &files,
            &vis,
            &GlobalFilter::default(),
            DisplayMask::default(),
            None
        ));
    }

    /// A masked display category must block hover exactly like the tree
    /// toggle: hidden ink cannot be hit-tested.
    #[test]
    fn masked_track_points_block_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let vis = vis_all_visible();
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::TrackPoints, false);
        assert!(!is_spatial_point_visible(
            &sp,
            &files,
            &vis,
            &GlobalFilter::default(),
            mask,
            None
        ));
    }

    /// The display mask gates each plan decision independently of the tree
    /// toggles: tracks, track points, and satellite labels have their own
    /// categories.
    #[test]
    fn track_plan_respects_the_display_mask() {
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let vis = vis_all_visible();
        let filter = GlobalFilter::default();
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));

        let all_on =
            viewport::TrackPlan::compute(&files, &vis, &filter, DisplayMask::default(), 15.0)
                .entry(track)
                .expect("track is in the plan");
        assert!(all_on.trackline);
        assert!(all_on.fade.is_some());

        assert!(all_on.sky_glyphs);

        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::Tracks, false);
        mask.set_visible(DisplayCategory::TrackPoints, false);
        mask.set_visible(DisplayCategory::SatelliteLabels, false);
        mask.set_visible(DisplayCategory::SkyGlyphs, false);
        let all_off = viewport::TrackPlan::compute(&files, &vis, &filter, mask, 15.0)
            .entry(track)
            .expect("track is in the plan");
        assert!(!all_off.trackline);
        assert!(all_off.fade.is_none());
        assert!(all_off.draws_nothing());

        // Track points masked alone: the line, the labels, and the sky
        // glyphs stay - they have their own categories.
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::TrackPoints, false);
        let points_off = viewport::TrackPlan::compute(&files, &vis, &filter, mask, 15.0)
            .entry(track)
            .expect("track is in the plan");
        assert!(points_off.trackline);
        assert!(points_off.fade.is_none());
        assert!(points_off.sat_labels);
        assert!(points_off.sky_glyphs);

        // Sky glyphs masked alone: everything else stays.
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::SkyGlyphs, false);
        let glyphs_off = viewport::TrackPlan::compute(&files, &vis, &filter, mask, 15.0)
            .entry(track)
            .expect("track is in the plan");
        assert!(!glyphs_off.sky_glyphs);
        assert!(glyphs_off.trackline);
        assert!(glyphs_off.fade.is_some());
    }

    /// With every position-carrying category masked there is no visible
    /// ink to frame, so zoom-to-fit must do nothing.
    #[test]
    fn fully_masked_map_has_no_bounding_box() {
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let vis = vis_all_visible();
        let filter = GlobalFilter::default();
        let mut mask = DisplayMask::default();
        for category in [
            DisplayCategory::Tracks,
            DisplayCategory::TrackPoints,
            DisplayCategory::SatelliteLabels,
            DisplayCategory::CustomMarkers,
        ] {
            mask.set_visible(category, false);
        }
        assert_eq!(
            compute_visible_bounding_box(&files, &vis, &filter, mask),
            None
        );
        mask.set_visible(DisplayCategory::Tracks, true);
        assert!(compute_visible_bounding_box(&files, &vis, &filter, mask).is_some());
    }

    /// Builds a single-point [`NavPoint`] stamped at `time`.
    fn nav_at(time: chrono::DateTime<chrono::Utc>, lat: f64, lon: f64) -> gt_types::NavPoint {
        let tpv = gt_types::TimePositionVelocity::builder()
            .time(gt_types::GpsTime::from_utc(time))
            .lat(gt_types::Latitude::new(lat))
            .lon(gt_types::Longitude::new(lon))
            .build();
        gt_types::NavPoint::new(tpv, None)
    }

    /// Regression test: with a partially-overlapping track, points outside the
    /// filter's time window must not be hoverable - they are hidden on the map,
    /// so the hit-test must agree (otherwise filtered points stay clickable).
    #[test]
    fn time_filtered_point_is_not_hoverable() {
        let early = chrono::DateTime::from_timestamp(0, 0).expect("valid");
        let late = early + chrono::Duration::seconds(100);
        let track = LoadedTrack {
            metadata: TrackMetadata {
                time_range: TimeRange::new(early, late),
                ..TrackMetadata::default()
            },
            points: vec![nav_at(early, 55.0, 12.0), nav_at(late, 55.0, 12.0)],
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
            channels: vec![],
        };
        let files = vec![file_with_tracks(vec![track])];
        let vis = vis_all_visible();
        // Start the window between the two points: the track still overlaps it,
        // but the early point falls outside.
        let filter = GlobalFilter {
            time_start: Some(early + chrono::Duration::seconds(50)),
            ..GlobalFilter::default()
        };
        assert!(
            !is_spatial_point_visible(
                &tpv_spatial_point(0, 0, 0),
                &files,
                &vis,
                &filter,
                DisplayMask::default(),
                None
            ),
            "the pre-window point must not be hoverable"
        );
        assert!(
            is_spatial_point_visible(
                &tpv_spatial_point(0, 0, 1),
                &files,
                &vis,
                &filter,
                DisplayMask::default(),
                None
            ),
            "the in-window point must stay hoverable"
        );
    }

    /// Regression test: points a `keep`/`hide` query removed are not drawn, so
    /// they must not be hoverable or clickable either.
    #[test]
    fn query_hidden_point_is_not_hoverable() {
        let now = chrono::DateTime::from_timestamp(0, 0).expect("valid");
        let track = LoadedTrack {
            metadata: TrackMetadata::default(),
            points: vec![nav_at(now, 55.0, 12.0), nav_at(now, 55.0001, 12.0001)],
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
            channels: vec![],
        };
        let files = vec![file_with_tracks(vec![track])];
        let vis = vis_all_visible();
        let filter = GlobalFilter::default();
        // A range built from arguments, so the single-element vec does not trip
        // clippy's `single_range_in_vec_init`.
        let rng = |start: usize, end: usize| start..end;
        let matches = QueryMatches {
            hidden: std::collections::HashMap::from([(
                TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                vec![rng(0, 1)],
            )]),
            ..QueryMatches::default()
        };
        assert!(
            !is_spatial_point_visible(
                &tpv_spatial_point(0, 0, 0),
                &files,
                &vis,
                &filter,
                DisplayMask::default(),
                Some(&matches)
            ),
            "the query-hidden point must not be hoverable"
        );
        assert!(
            is_spatial_point_visible(
                &tpv_spatial_point(0, 0, 1),
                &files,
                &vis,
                &filter,
                DisplayMask::default(),
                Some(&matches)
            ),
            "the point the query kept must stay hoverable"
        );
        assert!(
            is_spatial_point_visible(
                &tpv_spatial_point(0, 0, 0),
                &files,
                &vis,
                &filter,
                DisplayMask::default(),
                None
            ),
            "without a query run the point is hoverable"
        );
    }

    /// The hover must skip the hidden nearest point and return a visible one instead.
    #[test]
    fn hover_skips_hidden_nearest_and_finds_visible() {
        // Two overlapping SpatialPoints in the same Mercator position.
        // Track 0 is hidden, track 1 is visible.
        let hidden = SpatialPoint {
            merc: MercPoint { x: 0.5, y: 0.5 },
            file_index: FileIdx::new(0),
            track_index: TrackIdx::new(0),
            point_index: PointIdx::new(0),
            category: DataCategory::Tpv,
        };
        let visible = SpatialPoint {
            merc: MercPoint { x: 0.5, y: 0.5 },
            file_index: FileIdx::new(0),
            track_index: TrackIdx::new(1),
            point_index: PointIdx::new(0),
            category: DataCategory::Tpv,
        };
        let tree = rstar::RTree::bulk_load(vec![hidden, visible]);
        let files = vec![file_with_tracks(vec![
            track_at(55.0, 12.0),
            track_at(56.0, 13.0),
        ])];
        let mut first_disabled = TrackVisibility::all_visible();
        first_disabled.enabled = false;
        let vis = TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![first_disabled, TrackVisibility::all_visible()],
            }],
        };
        let filter = GlobalFilter::default();
        let found = tree
            .nearest_neighbor_iter([0.5_f64, 0.5_f64])
            .take_while(|sp| sp.distance_2(&[0.5, 0.5]) <= f64::MAX)
            .find(|sp| {
                is_spatial_point_visible(sp, &files, &vis, &filter, DisplayMask::default(), None)
            });
        assert!(found.is_some(), "should find the visible track");
        assert_eq!(
            found.unwrap().track_index,
            TrackIdx::new(1),
            "should return track 1, not the hidden track 0"
        );
    }

    /// After deleting a file, the global spatial index must be rebuilt so that
    /// point indices from the old (deleted) file don't survive into the next frame
    /// and cause out-of-bounds panics in the renderers.
    #[test]
    fn spatial_index_valid_after_file_deletion() {
        let all_points = nav_test_data(); // 1 200 points, all with headings
        let points_a: Vec<_> = all_points.iter().take(700).cloned().collect();
        let points_b: Vec<_> = all_points.iter().take(340).cloned().collect();

        let file_a = make_file_from_points(points_a);
        let file_b = make_file_from_points(points_b.clone());

        // Confirm the bug scenario: the stale tree (built before deletion) has
        // entries with point_index ≥ 340, which would be OOB for file_b alone.
        let files_initial = vec![file_a, make_file_from_points(points_b)];
        let stale_tree = gt_track_builder::build_global_tree(&files_initial);
        let files_after = vec![file_b];
        let stale_has_oob = stale_tree.iter().any(|sp| {
            let Some(file) = sp.file_index.get(&files_after) else {
                return true; // file index out of bounds → OOB
            };
            let Some(track) = sp.track_index.get(&file.tracks) else {
                return true; // track index out of bounds → OOB
            };
            let len = match sp.category {
                DataCategory::Tpv => track.points.len(),
                DataCategory::CustomMarker => track.custom_markers.len(),
                DataCategory::GeneratedMarker => track.generated_markers.len(),
                DataCategory::EventMarker => track.event_markers.len(),
                DataCategory::Track | DataCategory::SatelliteReport => return false,
            };
            sp.point_index.as_usize() >= len
        });
        assert!(
            stale_has_oob,
            "test setup: stale tree must have OOB entries"
        );

        // After calling rebuild_spatial_index, all entries must be in-bounds.
        let mut map = NavMap::new(egui::Context::default());
        map.rebuild_spatial_index(&files_initial);
        map.rebuild_spatial_index(&files_after);

        assert!(
            map.all_tree_indices_valid(&files_after),
            "spatial index has stale entries after file deletion"
        );
    }

    #[test]
    fn compound_label_guard_truth_table() {
        for (multi, disambig, suppress, expected) in [
            (true, false, false, false), // first frame, suppress not yet set
            (true, false, true, true),   // settled multi-hover
            (false, false, true, false), // single hover
            (true, true, true, false),   // disambiguation popup open
        ] {
            assert_eq!(
                should_show_compound_label(multi, disambig, suppress),
                expected,
                "multi={multi} disambig={disambig} suppress={suppress}"
            );
        }
    }

    /// candidate_label for a GnssFixRegained marker with a known duration must
    /// produce the same string as generated_marker_header, both surfaces share
    /// the same text so the disambiguation popup and the compound hover label agree.
    #[test]
    fn candidate_label_generated_marker_matches_header() {
        use gt_types::{
            GeneratedMarker, GeneratedMarkerKind, Latitude, LoadedTrack, Longitude, mercator,
        };

        let now = chrono::Utc::now();
        let dur = chrono::Duration::milliseconds(12_300);
        let lat = Latitude::new(55.686_7);
        let lon = Longitude::new(12.563_8);
        let bb = Rect::new(Coord { x: 12.55, y: 55.67 }, Coord { x: 12.59, y: 55.69 });
        let track = LoadedTrack {
            metadata: TrackMetadata {
                index: 0,
                distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0),
                duration: chrono::Duration::seconds(1),
                time_range: TimeRange::new(now, now + chrono::Duration::seconds(1)),
                bounding_box: bb,
                merc_bounds: merc_bounds_for_rect(bb),
                point_set_diameter_m: uom::si::f64::Length::new::<uom::si::length::meter>(10.0),
                has_custom_markers: false,
                tpv_count: 0,
                satellite_report_count: 0,
                custom_marker_count: 0,
                generated_marker_count: 1,
                event_marker_count: 0,
                ..TrackMetadata::default()
            },
            points: vec![],
            lod: gt_types::TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: vec![],
            generated_markers: vec![GeneratedMarker {
                time: now,
                kind: GeneratedMarkerKind::GnssFixRegained {
                    fix_lost_duration: dur,
                },
                lat,
                lon,
                merc: mercator::normalize(lat, lon),
            }],
            event_markers: vec![],
            channels: vec![],
        };
        let file = LoadedFile {
            metadata: FileMetadata {
                filename: "test.gtd".to_string(),
                total_distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0),
                total_duration: chrono::Duration::seconds(1),
                time_range: TimeRange::new(now, now + chrono::Duration::seconds(1)),
                ..FileMetadata::default()
            },
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(PathBuf::from("test.gtd")),
            load_warnings: vec![],
        };

        let candidate = gt_ui_types::DataPointRef {
            track: gt_types::TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::GeneratedMarker,
            point_index: PointIdx::new(0),
        };
        let expected = crate::generated_marker_renderer::generated_marker_header(
            &GeneratedMarkerKind::GnssFixRegained {
                fix_lost_duration: dur,
            },
        );
        assert_eq!(
            candidate_label(candidate, &[file]),
            expected,
            "candidate_label must delegate to generated_marker_header"
        );
    }
}

#[cfg(test)]
mod snapshot_tests {
    use std::path::PathBuf;

    use super::*;
    use gt_types::mercator::MercPoint;
    use gt_types::{DataCategory, DisplayMode, FileIdx, NavPoint, PointIdx, TrackIdx, TrackRef};
    use gt_ui_types::{DataPointRef, DisplayCategory, DisplayMask};

    fn tpv_ref() -> DataPointRef {
        DataPointRef {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::Tpv,
            point_index: PointIdx::new(0),
        }
    }

    fn event_ref() -> DataPointRef {
        DataPointRef {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::EventMarker,
            point_index: PointIdx::new(0),
        }
    }

    fn custom_ref() -> DataPointRef {
        DataPointRef {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::CustomMarker,
            point_index: PointIdx::new(0),
        }
    }

    fn gen_ref() -> DataPointRef {
        DataPointRef {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::GeneratedMarker,
            point_index: PointIdx::new(0),
        }
    }

    /// Builds a single `LoadedFile` with one TPV point, one event marker, one custom
    /// marker, and one `GnssFixRegained` generated marker, all at index 0.  Used by
    /// snapshot tests so each candidate type produces real human-readable text.
    fn make_snapshot_file() -> gt_types::LoadedFile {
        use gt_types::{
            CustomMarker, EventMarker, FileMetadata, GeneratedMarker, GeneratedMarkerKind,
            Latitude, LoadedFile, LoadedTrack, Longitude, MarkerIcon, TimeRange, TrackMetadata,
            merc_bounds_for_rect, mercator,
        };
        use uom::si::f64::Length;
        use uom::si::length::kilometer;

        let points = gt_test_utils::nav_test_data();
        let t0 = points[0].tpv.time().utc();
        let lat = Latitude::new(55.686_7);
        let lon = Longitude::new(12.563_8);

        let event_marker = EventMarker::new(
            t0,
            "Lap/Start".to_string(),
            Some("Lap start point".to_string()),
            lat,
            lon,
        );
        let custom_marker = CustomMarker::new(
            t0,
            "Coffee stop".to_string(),
            MarkerIcon::Pin,
            lat,
            lon,
            None,
        );
        let generated_marker = GeneratedMarker {
            time: t0,
            kind: GeneratedMarkerKind::GnssFixRegained {
                fix_lost_duration: chrono::Duration::milliseconds(12_300),
            },
            lat,
            lon,
            merc: mercator::normalize(lat, lon),
        };

        let bb = gt_types::Rect::new(
            gt_types::Coord { x: 12.55, y: 55.67 },
            gt_types::Coord { x: 12.59, y: 55.69 },
        );
        let n = points.len();
        // Counted from the points rather than hard-coded: `SkySection::resolve`
        // short-circuits on a zero count, so claiming zero here would hide the
        // sky plot even though these points carry satellite reports.
        let satellite_report_count = points.iter().filter(|p| p.satellites.is_some()).count();
        let track = LoadedTrack {
            metadata: TrackMetadata {
                index: 0,
                distance_km: Length::new::<kilometer>(5.0),
                duration: chrono::Duration::seconds(n as i64),
                time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
                bounding_box: bb,
                merc_bounds: merc_bounds_for_rect(bb),
                point_set_diameter_m: Length::new::<uom::si::length::meter>(500.0),
                has_custom_markers: true,
                tpv_count: n,
                satellite_report_count,
                custom_marker_count: 1,
                generated_marker_count: 1,
                event_marker_count: 1,
                ..TrackMetadata::default()
            },
            sat_label_anchors: gt_track_builder::build_sat_label_anchors(&points),
            points,
            lod: gt_types::TrackLod::default(),
            custom_markers: vec![custom_marker],
            generated_markers: vec![generated_marker],
            event_markers: vec![event_marker],
            channels: vec![],
        };

        LoadedFile {
            metadata: FileMetadata {
                filename: "snapshot_test.gtd".to_string(),
                total_distance_km: Length::new::<kilometer>(5.0),
                total_duration: chrono::Duration::seconds(n as i64),
                time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
                ..FileMetadata::default()
            },
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(PathBuf::from("snapshot_test.gtd")),
            load_warnings: vec![],
        }
    }

    /// Snapshot: the stacked multi-hover label popup for TPV + event marker +
    /// custom marker simultaneously within cursor radius.  Calls the real
    /// production function so the test stays in sync with the code.
    #[test]
    fn snap_multi_hover_stacked_label() {
        let files = vec![make_snapshot_file()];
        let candidates = [Some(tpv_ref()), Some(event_ref()), Some(custom_ref()), None];

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(400.0, 800.0))
            .ui(move |ui| {
                draw_multi_hover_label_contents(ui, &candidates, &files);
            });

        harness.fit_contents();
        harness.snapshot("multi_hover_stacked_label");
    }

    /// Snapshot: the stacked multi-hover label for the common case where a TPV
    /// fix point and a GNSS-fix-regained generated marker share the same map
    /// position.  The TPV section shows the full hover table. The generated-marker
    /// section shows the kind and the fix-lost duration.
    #[test]
    fn snap_multi_hover_tpv_and_generated_marker() {
        let files = vec![make_snapshot_file()];
        let candidates = [Some(tpv_ref()), None, None, Some(gen_ref())];

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(400.0, 800.0))
            .ui(move |ui| {
                draw_multi_hover_label_contents(ui, &candidates, &files);
            });

        harness.fit_contents();
        harness.snapshot("multi_hover_tpv_and_generated_marker");
    }

    /// Snapshot: the disambiguation popup (item 16) with large icons via
    /// LayoutJob. Calls the real `draw_disambig_row` so the test stays in sync
    /// with the production code. Verifies that the icon renders at a visually
    /// larger size than the label text.
    #[test]
    fn snap_disambig_popup_big_icons() {
        let files = vec![make_snapshot_file()];
        let candidates = [Some(tpv_ref()), Some(event_ref()), None, None];
        let sticky = Some(tpv_ref());

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(300.0, 90.0))
            .ui(move |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    for candidate in candidates.iter().flatten().copied() {
                        draw_disambig_row(ui, candidate, &files, sticky == Some(candidate));
                    }
                });
            });

        harness.run();
        harness.snapshot("disambig_popup_big_icons");
    }

    /// The gaps between `ranges` within `0..len` - the points a `keep` query
    /// hides.
    fn complement(ranges: &[std::ops::Range<usize>], len: usize) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        let mut cursor = 0;
        for r in ranges {
            if cursor < r.start {
                out.push(cursor..r.start);
            }
            cursor = r.end;
        }
        if cursor < len {
            out.push(cursor..len);
        }
        out
    }

    /// Drive the full `NavMap::draw` path over the fixture track with a
    /// hardcoded set of query matches. Requires `GEOTRACE_OFFLINE=1` (set by
    /// `just test`) so no map tiles render beneath the halos.
    fn snapshot_nav_map_with_matches(name: &'static str, mode: DisplayMode, stale: bool) {
        use std::collections::HashMap;

        use gt_ui_types::{DrawLayer, QueryMatches, TrackDataVisibility};

        let files = vec![make_snapshot_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let len = files
            .first()
            .and_then(|f| f.tracks.first())
            .map_or(0, |t| t.points.len());
        // Two multi-point stretches on different legs of the fixture loop,
        // plus a single-point match that must render as a ring.
        let ranges = vec![150..300, 700..701, 900..1000];
        let per_track = |rs: Vec<std::ops::Range<usize>>| HashMap::from([(track, rs)]);
        let matches = match mode {
            DisplayMode::Draw => QueryMatches {
                draws: vec![DrawLayer {
                    color: 0,
                    ranges: per_track(ranges),
                }],
                stale,
                ..QueryMatches::default()
            },
            DisplayMode::Hide => QueryMatches {
                hidden: per_track(ranges),
                stale,
                ..QueryMatches::default()
            },
            DisplayMode::Keep => QueryMatches {
                hidden: per_track(complement(&ranges, len)),
                stale,
                ..QueryMatches::default()
            },
        };

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight::default();
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut gt_ui_types::DisplayMask::default(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        Some(&matches),
                        None,
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        // First frame zooms to fit the newly seen file; extra frames let the
        // blink/fade animations settle before the snapshot.
        for _ in 0..5 {
            harness.run();
        }
        harness.snapshot_loose(name);
    }

    /// Interference cells around the snapshot fixture's track, tallied
    /// across the ramp so one snapshot shows clear, elevated, heavy, and
    /// low-sample fills together.
    fn snapshot_jamming_dataset() -> JamDataset {
        use gt_jam::wire::HexObservation;

        let center = h3o::LatLng::new(55.686_7, 12.563_8)
            .expect("fixture position")
            .to_cell(gt_jam::H3_RESOLUTION);
        let tallies = [
            (400, 0),
            (98, 2),
            (94, 6),
            (90, 10),
            (60, 40),
            (2, 2),
            (1, 1),
        ];
        let observations = center
            .grid_disk::<Vec<_>>(1)
            .into_iter()
            .zip(tallies)
            .map(|(cell, (good, bad))| HexObservation { cell, good, bad })
            .collect();
        JamDataset::new(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 20).expect("date"),
            observations,
        )
    }

    /// The interference overlay under the fixture track. At the zoom that
    /// frames a 1 km track a single 22 km cell covers the viewport, so this
    /// pins the fill and the draw order - track ink over cells - rather than
    /// the ramp, which `jamming_renderer`'s own tests cover. Requires
    /// `GEOTRACE_OFFLINE=1` (set by `just test`) so no map tiles render.
    #[rstest::rstest]
    #[case::dark("jamming_overlay_dark", true, None)]
    #[case::light("jamming_overlay_light", false, None)]
    #[case::hover("jamming_overlay_hover", true, Some(egui::pos2(400.0, 300.0)))]
    fn snapshot_jamming_overlay(
        #[case] name: &str,
        #[case] dark_mode: bool,
        #[case] hover: Option<egui::Pos2>,
    ) {
        let files = vec![make_snapshot_file()];
        let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
        let dataset = snapshot_jamming_dataset();

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .theme(dark_mode)
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight::default();
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut gt_ui_types::DisplayMask::default(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        None,
                        Some(&dataset),
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        // The first frame zooms to fit the file; the rest settle animations.
        for _ in 0..5 {
            harness.run();
        }
        if let Some(pos) = hover {
            harness.inner.hover_at(pos);
            // Tooltips appear after egui's hover delay.
            for _ in 0..60 {
                harness.run();
            }
        }
        harness.snapshot_loose(name);
    }

    /// Nudge north (smaller Mercator y) by roughly ten pixels at the
    /// snapped-track snapshot tests' zoom, so the snapped line reads beside
    /// the recorded one instead of on top of it.
    const SNAPPED_OFFSET_MERC_Y: f64 = -1.5e-6;

    /// Snapshot the map with `make_snapshot_file`'s track plus the snapped
    /// geometry `geometry_for` derives from its points, drawn under `mask`.
    /// With `hover`, the pointer is parked there before the snapshot (frames
    /// are stepped past egui's tooltip delay). Requires `GEOTRACE_OFFLINE=1`
    /// (set by `just test`) so no map tiles render.
    fn snapshot_snapped_tracks_with(
        name: &str,
        mask: DisplayMask,
        hover: Option<egui::Pos2>,
        geometry_for: impl Fn(&[NavPoint]) -> gt_ui_types::SnappedTrackGeometry,
    ) {
        use std::sync::Arc;

        use gt_ui_types::{SnappedTracks, TrackDataVisibility};

        let files = vec![make_snapshot_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let points = files
            .first()
            .and_then(|f| f.tracks.first())
            .map(|t| t.points.clone())
            .unwrap_or_default();
        let snapped = SnappedTracks {
            by_track: std::collections::HashMap::from([(
                track_ref,
                Arc::new(geometry_for(&points)),
            )]),
        };

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight::default();
                    let mut mask = mask;
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut mask,
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        Some(&snapped),
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        for _ in 0..5 {
            harness.run();
        }
        if let Some(pos) = hover {
            harness.inner.hover_at(pos);
            // Tooltips appear after egui's hover delay; keep stepping until
            // it elapsed and the tooltip laid itself out.
            for _ in 0..60 {
                harness.run();
            }
        }
        harness.snapshot_loose(name);
    }

    /// [`snapshot_snapped_tracks_with`] for bare polylines (no edge data,
    /// no hover).
    fn snapshot_snapped_tracks(
        name: &str,
        mask: DisplayMask,
        segments_for: impl Fn(&[NavPoint]) -> Vec<Vec<MercPoint>>,
    ) {
        snapshot_snapped_tracks_with(name, mask, None, |points| {
            gt_ui_types::SnappedTrackGeometry {
                segments: segments_for(points)
                    .into_iter()
                    .map(|points| gt_ui_types::SnappedSegment {
                        points,
                        edge_spans: Vec::new(),
                    })
                    .collect(),
                edges: Vec::new(),
                whiskers: Vec::new(),
            }
        });
    }

    /// A snapped segment following the recorded points in `range`, nudged
    /// north by [`SNAPPED_OFFSET_MERC_Y`].
    fn snapped_segment(points: &[NavPoint], range: std::ops::Range<usize>) -> Vec<MercPoint> {
        points
            .get(range)
            .unwrap_or_default()
            .iter()
            .map(|p| MercPoint {
                x: p.merc.x,
                y: p.merc.y + SNAPPED_OFFSET_MERC_Y,
            })
            .collect()
    }

    /// Snapshot: dashed translucent snapped-track polylines beside the
    /// recorded track, with the empty stretch between the two segments
    /// rendering as a gap (a route discontinuity) - the recorded track
    /// beneath is never painted over or hidden.
    #[test]
    fn snap_snapped_track_polylines() {
        snapshot_snapped_tracks(
            "snapped_track_polylines",
            DisplayMask::default(),
            |points| {
                vec![
                    snapped_segment(points, 100..400),
                    snapped_segment(points, 600..950),
                ]
            },
        );
    }

    /// Snapshot: a snapped segment whose tail runs far past the viewport.
    /// The culling in `SnappedTrackRenderer` must not clip visible geometry:
    /// the dashed line has to reach the viewport edge exactly, while the
    /// off-screen stretch generates no dashes at all (partially visible
    /// segments keep exact endpoints; only provably invisible ones are
    /// dropped).
    #[test]
    fn snap_snapped_track_culled_tail() {
        /// Mercator step between synthetic tail points, ≈ 5 viewport widths
        /// beyond the fitted view over 60 points, so most of the tail is
        /// provably off-screen.
        const TAIL_STEP_MERC_X: f64 = 2e-5;

        snapshot_snapped_tracks(
            "snapped_track_culled_tail",
            DisplayMask::default(),
            |points| {
                let mut segment = snapped_segment(points, 100..400);
                if let Some(&end) = segment.last() {
                    segment.extend((1..=60).map(|i| MercPoint {
                        x: end.x + f64::from(i) * TAIL_STEP_MERC_X,
                        y: end.y,
                    }));
                }
                vec![segment]
            },
        );
    }

    /// Snapshot: a snapped segment whose on-screen extent packs below one
    /// pixel draws as a dot instead of vanishing - the `VisiblePath::Dot`
    /// case, reached when snapped geometry collapses at low zoom. The dot
    /// sits north of the recorded track's midpoint.
    #[test]
    fn snap_snapped_track_collapsed_dot() {
        /// Mercator spacing of the collapsed cluster's points, ≈ 0.1 px at
        /// the fitted zoom - far below the sub-pixel merge threshold.
        const CLUSTER_STEP_MERC_X: f64 = 2e-8;

        /// Extra northward offset so the dot is clearly separate from the
        /// recorded trackline.
        const CLUSTER_OFFSET_MERC_Y: f64 = -6e-6;

        snapshot_snapped_tracks(
            "snapped_track_collapsed_dot",
            DisplayMask::default(),
            |points| {
                let mid = points.len() / 2;
                let Some(base) = points.get(mid) else {
                    return vec![];
                };
                vec![
                    (0..4)
                        .map(|i| MercPoint {
                            x: base.merc.x + f64::from(i) * CLUSTER_STEP_MERC_X,
                            y: base.merc.y + CLUSTER_OFFSET_MERC_Y,
                        })
                        .collect(),
                ]
            },
        );
    }

    /// Snapshot: hiding the snapped-tracks display category removes the
    /// dashed ink entirely - only the recorded track remains - without
    /// touching the underlying snapped geometry.
    #[test]
    fn snap_snapped_track_hidden_by_display_mask() {
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::SnappedTracks, false);
        snapshot_snapped_tracks("snapped_track_hidden_by_display_mask", mask, |points| {
            vec![
                snapped_segment(points, 100..400),
                snapped_segment(points, 600..950),
            ]
        });
    }

    /// Snapshot: hovering the snapped line shows the matched edge's
    /// attributes. The synthetic segment runs horizontally through the
    /// viewport center (zoom-to-fit centers the recorded track's bounds),
    /// so parking the pointer at the center hits it deterministically.
    #[test]
    fn snap_snapped_track_edge_hover() {
        /// Half-width of the synthetic segment, Mercator units - wide
        /// enough to cross the whole fitted viewport.
        const SEGMENT_HALF_WIDTH_MERC: f64 = 1.0e-4;

        snapshot_snapped_tracks_with(
            "snapped_track_edge_hover",
            DisplayMask::default(),
            Some(egui::pos2(400.0, 300.0)),
            |points| {
                let (min, max) = points.iter().fold(
                    ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)),
                    |(min, max), p| {
                        (
                            (min.0.min(p.merc.x), min.1.min(p.merc.y)),
                            (max.0.max(p.merc.x), max.1.max(p.merc.y)),
                        )
                    },
                );
                let center = MercPoint {
                    x: f64::midpoint(min.0, max.0),
                    y: f64::midpoint(min.1, max.1),
                };
                gt_ui_types::SnappedTrackGeometry {
                    segments: vec![gt_ui_types::SnappedSegment {
                        points: vec![
                            MercPoint {
                                x: center.x - SEGMENT_HALF_WIDTH_MERC,
                                y: center.y,
                            },
                            MercPoint {
                                x: center.x + SEGMENT_HALF_WIDTH_MERC,
                                y: center.y,
                            },
                        ],
                        edge_spans: vec![gt_ui_types::SnappedEdgeSpan {
                            start: 0,
                            end: 2,
                            edge: 0,
                        }],
                    }],
                    edges: vec![gt_ui_types::SnappedEdgeInfo {
                        name: Some("H.C. Andersens Boulevard".to_owned()),
                        road_class: Some("Tertiary".to_owned()),
                        speed_limit: Some("50 km/h".to_owned()),
                        surface: Some("Paved smooth".to_owned()),
                    }],
                    whiskers: Vec::new(),
                }
            },
        );
    }

    /// Eastward Mercator offset of the synthetic whisker tests' snapped
    /// positions: ~9 m at the fixture latitude, so whiskers are clearly
    /// longer than the strokes they connect.
    const WHISKER_OFFSET_MERC_X: f64 = 4.0e-7;

    /// Whisker anchors and the matching snapped polyline for a run over
    /// `points`: every recorded point snaps [`WHISKER_OFFSET_MERC_X`] east.
    fn whisker_geometry(points: &[NavPoint]) -> gt_ui_types::SnappedTrackGeometry {
        let snapped: Vec<MercPoint> = points
            .iter()
            .map(|p| MercPoint {
                x: p.merc.x + WHISKER_OFFSET_MERC_X,
                y: p.merc.y,
            })
            .collect();
        gt_ui_types::SnappedTrackGeometry {
            segments: vec![gt_ui_types::SnappedSegment {
                points: snapped.clone(),
                edge_spans: Vec::new(),
            }],
            edges: Vec::new(),
            whiskers: points
                .iter()
                .zip(snapped)
                .enumerate()
                .map(|(i, (_, snapped))| gt_ui_types::WhiskerAnchor {
                    point: PointIdx::new(i),
                    snapped,
                })
                .collect(),
        }
    }

    /// A file whose single track spans only ~55 m, so zoom-to-fit lands
    /// far above the whisker scale gate.
    fn make_short_walk_file() -> gt_types::LoadedFile {
        use gt_types::time_types::GpsTime;
        use gt_types::{
            FileMetadata, Latitude, LoadedFile, LoadedTrack, Longitude, TimeRange, TrackMetadata,
            merc_bounds_for_rect,
        };

        let t0 = chrono::DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default();
        let points: Vec<gt_types::NavPoint> = (0..6)
            .map(|i| {
                let tpv = gt_types::TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(t0 + chrono::Duration::seconds(i)))
                    .lat(Latitude::new(55.68 + i as f64 * 1.0e-4))
                    .lon(Longitude::new(12.56))
                    .build();
                gt_types::NavPoint::new(tpv, None)
            })
            .collect();
        let bb = gt_types::Rect::new(
            gt_types::Coord { x: 12.56, y: 55.68 },
            gt_types::Coord {
                x: 12.56,
                y: 55.6805,
            },
        );
        let n = points.len();
        let track = LoadedTrack {
            metadata: TrackMetadata {
                time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
                bounding_box: bb,
                merc_bounds: merc_bounds_for_rect(bb),
                tpv_count: n,
                ..TrackMetadata::default()
            },
            sat_label_anchors: Vec::new(),
            points,
            lod: gt_types::TrackLod::default(),
            custom_markers: Vec::new(),
            generated_markers: Vec::new(),
            event_markers: Vec::new(),
            channels: Vec::new(),
        };
        LoadedFile {
            metadata: FileMetadata {
                filename: "short_walk.gtd".to_string(),
                time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
                ..FileMetadata::default()
            },
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(PathBuf::from("short_walk.gtd")),
            load_warnings: vec![],
        }
    }

    /// Snapshot: above the scale gate (a ~55 m track fitted into the
    /// viewport) every snapped point gets its error whisker - a thin line
    /// from the recorded point to the snapped position.
    #[test]
    fn snap_snapped_track_whiskers_at_high_zoom() {
        use std::sync::Arc;

        use gt_ui_types::{SnappedTracks, TrackDataVisibility};

        let files = vec![make_short_walk_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let points = files
            .first()
            .and_then(|f| f.tracks.first())
            .map(|t| t.points.clone())
            .unwrap_or_default();
        let snapped = SnappedTracks {
            by_track: std::collections::HashMap::from([(
                track_ref,
                Arc::new(whisker_geometry(&points)),
            )]),
        };

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight::default();
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut gt_ui_types::DisplayMask::default(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        Some(&snapped),
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );
        for _ in 0..5 {
            harness.run();
        }
        harness.snapshot_loose("snapped_track_whiskers");
    }

    /// Snapshot: below the scale gate (the standard km-scale fixture) the
    /// same whisker anchors draw nothing - only the dashed snapped line.
    #[test]
    fn snap_snapped_track_whiskers_hidden_below_gate() {
        snapshot_snapped_tracks_with(
            "snapped_track_whiskers_below_gate",
            DisplayMask::default(),
            None,
            whisker_geometry,
        );
    }

    /// Snapshot: match halos along the track, including the single-point
    /// ring, over the live map canvas.
    #[test]
    fn snap_query_match_halos() {
        snapshot_nav_map_with_matches("query_match_halos", DisplayMode::Draw, false);
    }

    /// Snapshot: the same matches grayed out after the visible data changed
    /// (stale results are dimmed, never hidden).
    #[test]
    fn snap_query_match_halos_stale() {
        snapshot_nav_map_with_matches("query_match_halos_stale", DisplayMode::Draw, true);
    }

    /// Snapshot: `keep` mode shows only the matching stretches; the rest of
    /// the track is hidden and the polyline breaks at the gaps.
    #[test]
    fn snap_query_keep_mode() {
        snapshot_nav_map_with_matches("query_keep_mode", DisplayMode::Keep, false);
    }

    /// Snapshot: `hide` mode drops the matching stretches, leaving the rest
    /// of the track with breaks where the matches were.
    #[test]
    fn snap_query_hide_mode() {
        snapshot_nav_map_with_matches("query_hide_mode", DisplayMode::Hide, false);
    }

    /// Snapshot: the halo band for the match hovered in the query results
    /// table - the highlight blue over the matched stretch, without any
    /// `draw` layers underneath.
    #[test]
    fn snap_query_match_hover_halo() {
        let files = vec![make_snapshot_file()];
        let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight {
                        hover_match: Some(gt_ui_types::MatchHighlight::new(track, &(150..300))),
                        ..gt_ui_types::MapHighlight::default()
                    };
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut gt_ui_types::DisplayMask::default(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        None,
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        for _ in 0..5 {
            harness.run();
        }
        harness.snapshot_loose("query_match_hover_halo");
    }

    /// Snapshot: the display mask removes the marker ink (custom, generated,
    /// event) while the track, its icons, and the satellite labels stay.
    /// Compare against the marker-bearing fixture in the other snapshots.
    #[test]
    fn snap_display_mask_hides_markers() {
        use gt_ui_types::{DisplayCategory, DisplayMask, TrackDataVisibility};

        let files = vec![make_snapshot_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        let mut mask = DisplayMask::default();
        for category in [
            DisplayCategory::CustomMarkers,
            DisplayCategory::GeneratedMarkers,
            DisplayCategory::EventMarkers,
        ] {
            mask.set_visible(category, false);
        }

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight::default();
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut mask.clone(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        None,
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        for _ in 0..5 {
            harness.run();
        }
        harness.snapshot_loose("display_mask_hides_markers");
    }

    /// Snapshot: with every category except sky glyphs hidden, the glyphs are
    /// the only ink left - so their own category keeps drawing them even when
    /// the trackline, points, and labels are all off. Run for each variant so
    /// both the ring and the disc are exercised through the full map path.
    #[rstest::rstest]
    #[case::ring("sky_glyphs_only_ring", gt_ui_types::SkyGlyphVariant::Ring)]
    #[case::disc("sky_glyphs_only_disc", gt_ui_types::SkyGlyphVariant::Disc)]
    fn snap_sky_glyphs_only(#[case] name: &str, #[case] variant: gt_ui_types::SkyGlyphVariant) {
        use gt_ui_types::{DisplayCategory, DisplayMask, TrackDataVisibility};

        let files = vec![make_snapshot_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        let mut mask = DisplayMask::default();
        for category in [
            DisplayCategory::Tracks,
            DisplayCategory::TrackPoints,
            DisplayCategory::SatelliteLabels,
            DisplayCategory::CustomMarkers,
            DisplayCategory::GeneratedMarkers,
            DisplayCategory::EventMarkers,
        ] {
            mask.set_visible(category, false);
        }

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight::default();
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut mask.clone(),
                        &mut variant.clone(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        None,
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        for _ in 0..5 {
            harness.run();
        }
        harness.snapshot_loose(name);
    }

    /// Snapshot: hovering the time-series plot draws the detailed sky disc at
    /// the corresponding map point - even with the sky glyphs overlay hidden,
    /// since the plot-hover disc is a focus indicator, not part of the
    /// overlay. The ring around the point is the existing cross-highlight.
    #[test]
    fn snap_plot_hover_sky_disc() {
        use gt_ui_types::{DisplayCategory, DisplayMask, TrackDataVisibility};

        let files = vec![make_snapshot_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        // Overlay off, so the only disc on the map is the plot-hover one.
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::SkyGlyphs, false);
        // A mid-track point that carries a satellite report in the fixture.
        let hovered = (FileIdx::new(0), TrackIdx::new(0), PointIdx::new(50));

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(800.0, 600.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight {
                        plot_hover_point: Some(hovered),
                        ..gt_ui_types::MapHighlight::default()
                    };
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut mask.clone(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        None,
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        for _ in 0..5 {
            harness.run();
        }
        harness.snapshot_loose("plot_hover_sky_disc");
    }

    /// Snapshot: the clicked-point window itself - the resizable frame, the
    /// sky plot pinned beside the satellite tables, and the deselect hint on
    /// the window floor. Guards the whole composition, not just the body.
    #[test]
    fn snap_sticky_point_window() {
        use gt_ui_types::TrackDataVisibility;

        let files = vec![make_snapshot_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        // A mid-track point carrying a multi-constellation satellite report.
        let clicked = gt_ui_types::DataPointRef {
            track: gt_types::TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: gt_types::DataCategory::Tpv,
            point_index: PointIdx::new(50),
        };

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(900.0, 700.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight {
                        sticky: Some(clicked),
                        ..gt_ui_types::MapHighlight::default()
                    };
                    map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut gt_ui_types::DisplayMask::default(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        None,
                        None,
                        None,
                        false,
                        None,
                    );
                },
                None,
            );

        for _ in 0..5 {
            harness.run();
        }
        harness.snapshot_loose("sticky_point_window");
    }

    /// The point window's open-trails button has to travel the whole way out
    /// of `draw`: through the window body, into a [`SkyTrailsRequest`] carrying
    /// the clicked point's instant, and out as a [`MapContextAction`]. The
    /// widget-level test one layer down cannot see this wiring, so a dropped
    /// return value here would leave the button a silent no-op.
    #[test]
    fn the_point_window_button_returns_a_timed_sky_trails_action() {
        use egui_kittest::kittest::Queryable as _;
        use gt_ui_types::TrackDataVisibility;

        let files = vec![make_snapshot_file()];
        let visibility = TrackDataVisibility::from_loaded(&files);
        let clicked = gt_ui_types::DataPointRef {
            track: gt_types::TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: gt_types::DataCategory::Tpv,
            point_index: PointIdx::new(50),
        };
        let point_time = files
            .first()
            .and_then(|f| f.tracks.first())
            .and_then(|t| t.points.get(50))
            .map(|p| p.tpv.time())
            .expect("the fixture has a point 50");
        let action = std::rc::Rc::new(std::cell::Cell::new(None));
        let seen = action.clone();

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(900.0, 700.0))
            .ui_state(
                move |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone()));
                    let mut highlight = gt_ui_types::MapHighlight {
                        sticky: Some(clicked),
                        ..gt_ui_types::MapHighlight::default()
                    };
                    let returned = map.draw(
                        ui,
                        &files,
                        &visibility,
                        &mut highlight,
                        &gt_filter::GlobalFilter::default(),
                        &mut gt_ui_types::DisplayMask::default(),
                        &mut gt_ui_types::SkyGlyphVariant::default(),
                        &mut gt_ui_types::PointWindowFolds::default(),
                        &gt_ui_types::EventMarkerVisibility::default(),
                        &gt_ui_types::GeneratedMarkerVisibility::default(),
                        None,
                        None,
                        None,
                        None,
                        false,
                        None,
                    );
                    if returned.is_some() {
                        seen.set(returned);
                    }
                },
                None,
            );

        for _ in 0..5 {
            harness.run();
        }
        assert!(action.get().is_none(), "nothing requested before the click");

        harness
            .inner
            .get_by_label(egui_phosphor::regular::ARROW_SQUARE_OUT)
            .click();
        harness.inner.run_steps(2);

        assert_eq!(
            action.get(),
            Some(MapContextAction::ShowSkyTrails(
                gt_ui_types::SkyTrailsRequest::at_instant(clicked.track, point_time)
            ))
        );
    }

    /// The point layout - and with it the resizable frame - covers both
    /// categories that render the sky plot beside the satellite tables. A
    /// satellite-report popup carries the same 40-satellite content as a fix,
    /// so it must not fall back to the cramped auto-sized frame.
    #[rstest::rstest]
    #[case::tpv(gt_types::DataCategory::Tpv, true)]
    #[case::satellite_report(gt_types::DataCategory::SatelliteReport, true)]
    #[case::custom_marker(gt_types::DataCategory::CustomMarker, false)]
    #[case::generated_marker(gt_types::DataCategory::GeneratedMarker, false)]
    #[case::event_marker(gt_types::DataCategory::EventMarker, false)]
    #[case::track(gt_types::DataCategory::Track, false)]
    fn point_layout_covers_the_satellite_bearing_categories(
        #[case] category: gt_types::DataCategory,
        #[case] expected: bool,
    ) {
        assert_eq!(super::sticky_uses_point_layout(category), expected);
    }
}
