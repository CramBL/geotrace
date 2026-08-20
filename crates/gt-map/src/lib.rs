use egui::{Area, Button, Frame, Grid, Label, RichText, Window};
use egui_phosphor::regular::GLOBE_HEMISPHERE_WEST as ICON_GLOBE_HEMISPHERE_WEST;
use egui_phosphor::regular::MAP_TRIFOLD as ICON_MAP_TRIFOLD;
mod collision_grid;
pub mod display_counts;
mod display_toggle;
pub mod event_marker_renderer;
pub(crate) mod generated_marker_renderer;
mod hover_labels;
pub mod icon_mesh;
mod jamming_renderer;
mod log_match_renderer;
pub mod mapbox_tiles;
pub mod marker_renderer;
mod polyline;
mod query_match_renderer;
mod recording_labels;
mod sat_labels;
mod sky_glyph_renderer;
mod sky_trails_window;
mod snapped_track_renderer;
mod tec_renderer;
#[cfg(test)]
mod test_harness;
pub mod tpv_renderer;
mod track_layers;
pub mod track_renderer;
mod transform;
mod viewport;

pub use sky_trails_window::SkyTrailsWindow;
pub use tec_renderer::{TecHeatmapSnapshot, TecLayer};
pub use viewport::GeoBounds;

use std::cell::{Cell, RefCell};

use egui::Context;

use gt_filter::GlobalFilter;
use gt_jam::dataset::JamDataset;
use gt_jam::day_selection::{DaySelection, EmptyReason};
use gt_loaded_files::RecordingNames;
use gt_types::{DataCategory, FileIdx, LoadedFile, SpatialPoint, TrackRef};
use gt_ui_types::{
    DataPointRef, DisplayCategory, DisplayMask, EventMarkerVisibility, GeneratedMarkerVisibility,
    HighlightScope, HoverCandidates, HoveredLogGlyph, LogMatchHover, LogMatches, MapHighlight,
    MapScope, PinnedPopup, PointWindowFolds, QueryMatches, SkyGlyphVariant, SkyTrailsRequest,
    SnappedTracks, TrackDataVisibility,
};
use rstar::PointDistance as _;
use walkers::sources::OpenStreetMap;
use walkers::{HttpTiles, Map, MapMemory};

use crate::event_marker_renderer::EventMarkerRenderer;
use crate::generated_marker_renderer::GeneratedMarkerRenderer;
use crate::hover_labels::{
    PointerOwnership, draw_disambig_row, draw_multi_hover_label_contents,
    should_show_compound_label,
};
use crate::marker_renderer::MarkerRenderer;
use crate::recording_labels::RecordingLabels;
use crate::snapped_track_renderer::SnappedTrackRenderer;
use crate::track_layers::TrackLayers;
use crate::transform::{MapScale, MercTransform};
use crate::viewport::{
    compute_viewport_bounds, compute_visible_bounding_box, is_spatial_point_visible, zoom_to_fit,
};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum MapLayer {
    #[default]
    OpenStreetMap,
    Satellite,
}

/// Whether the layer picker offers the satellite layer without a Mapbox token
/// set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SatelliteLayerAccess {
    /// Selectable with no token: picking it opens the application's token
    /// dialog.
    WithoutToken,
    /// Grayed until a token is set.
    TokenRequired,
}

pub const SATELLITE_LAYER_NEEDS_TOKEN: &str = "Enter a Mapbox token to use the satellite layer";

/// Lowest zoom the satellite layer draws Mapbox tiles at. See
/// [`NavMap::use_mapbox_tiles`] for what happens below it.
pub(crate) const MAPBOX_MIN_SAFE_ZOOM: u8 = 2;

/// Action requested from a right-click context menu on a map element.
///
/// Returned by [`NavMap::draw`] when the user selects an item. The caller is
/// responsible for applying it to the visibility state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapContextAction {
    ShowOnlyTrack(TrackRef),
    ShowOnlyFile(FileIdx),
    /// Open the sky trails window, per the request's track and instant.
    ShowSkyTrails(SkyTrailsRequest),
}

const BLINK_DURATION_SEC: f32 = 3.0;
const BLINK_PULSE_HZ: f32 = 2.0;

/// Timestamps are egui clock seconds (`InputState::time`), so the pulse is
/// deterministic under `egui_kittest`'s simulated time.
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
        if elapsed >= BLINK_DURATION_SEC {
            self.start = None;
            0.0
        } else {
            let fade = 1.0 - (elapsed / BLINK_DURATION_SEC);
            (std::f32::consts::TAU * elapsed * BLINK_PULSE_HZ)
                .sin()
                .abs()
                * fade
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
/// fitting side by side.
const POINT_WINDOW_MIN_WIDTH_PX: f32 = 340.0;
const POINT_WINDOW_MIN_HEIGHT_PX: f32 = 260.0;

/// Whether this category's sticky window shows the sky plot beside the
/// per-constellation satellite tables, in a resizable frame.
///
/// Matched exhaustively so a new [`DataCategory`] has to pick its layout.
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

/// Whether the map may fetch its base tiles.
///
/// The application supplies it at construction. [`TileAccess::Offline`]
/// builds no tile fetcher at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileAccess {
    Network,
    Offline,
}

/// Context provided to the map for a single frame.
pub struct MapDrawContext<'a> {
    pub files: &'a [LoadedFile],
    /// Per-file display names, resolved from the user's recording-name
    /// template, so the map's labels name a recording the way the side panel
    /// and the plot do.
    pub recording_names: &'a RecordingNames,
    pub snapped_tracks: Option<&'a SnappedTracks>,
    pub jamming_dataset: Option<&'a JamDataset>,
    /// The archived TEC grid, the instant it is shown at, and why it is empty.
    pub tec: TecLayer<'a>,
    pub query_matches: Option<&'a QueryMatches>,
    /// What the loaded logs' filters selected onto the map.
    pub log_matches: &'a LogMatches,
    /// The log match under the cursor, on the map and in the viewer alike.
    pub log_hover: &'a mut LogMatchHover,
    pub empty_reason: Option<EmptyReason>,
    pub filter: &'a GlobalFilter,
    pub visibility: &'a TrackDataVisibility,
    pub event_marker_visibility: &'a EventMarkerVisibility,
    pub generated_marker_visibility: &'a GeneratedMarkerVisibility,
    pub display_mask: &'a mut DisplayMask,
    pub day_selection: &'a mut DaySelection,
    pub highlight: &'a mut MapHighlight,
    pub sky_glyph_variant: &'a mut SkyGlyphVariant,
    pub point_window_folds: &'a mut PointWindowFolds,
    pub center_request: Option<(f64, f64)>,
    pub zoom_to_visible: bool,
    pub sticky_pos_override: Option<egui::Pos2>,
}

impl<'a> MapDrawContext<'a> {
    fn recording_labels(&self) -> RecordingLabels<'a> {
        RecordingLabels::new(self.files, self.recording_names)
    }

    /// What the map draws this frame. Hit-testing, the pinned popup, and the
    /// headless tests all read it.
    fn scope(&self) -> MapScope<'a> {
        MapScope {
            files: self.files,
            visibility: self.visibility,
            filter: self.filter,
            display_mask: *self.display_mask,
            query_matches: self.query_matches,
        }
    }

    /// The bounding box around every element currently drawn, for framing.
    fn visible_bounding_box(&self) -> Option<(f64, f64, f64, f64)> {
        compute_visible_bounding_box(self.files, self.visibility, self.filter, *self.display_mask)
    }

    /// Suppress the renderers' individual hover labels when the disambiguation
    /// popup owns the cursor area, or when several candidates were hovered last
    /// frame - the map layer draws one stacked label in their place, so
    /// independent renderers do not pile theirs at the same spot.
    fn suppress_overlapping_hover_labels(&mut self, disambig_open: bool) {
        let prev_multi_hover = self.highlight.hover_candidates.is_ambiguous();
        // A hovered log hexagon takes the pointer from the fix underneath it:
        // it draws over that fix and lists its line itself.
        let prev_log_glyph_hovered = self.log_hover.glyph.is_some();
        self.highlight.suppress_hover_labels =
            disambig_open || prev_multi_hover || prev_log_glyph_hovered;
    }
}

/// The inputs determining whether the disambiguation popup closes this frame.
#[derive(Clone, Copy)]
struct DisambiguationDismissal {
    just_opened: bool,
    clicked_elsewhere: bool,
    escape_pressed: bool,
}

impl DisambiguationDismissal {
    /// `clicked_elsewhere` also fires on the frame the opening click lands, so
    /// the popup ignores it while `just_opened`.
    fn closes_popup(self) -> bool {
        !self.just_opened && (self.clicked_elsewhere || self.escape_pressed)
    }
}

/// The animation values the renderers read, ticked once at the start of the
/// frame.
#[derive(Clone, Copy)]
struct FrameAnimation {
    /// Alpha of the load-highlight pulse.
    blink_alpha: f32,
    /// Progress of the hover-focus overlay, 0 = absent, 1 = fully faded in.
    hover_fade: f32,
}

pub struct NavMap {
    egui_ctx: Context,
    /// [`None`] under [`TileAccess::Offline`]: the base layer stays blank
    /// and nothing is requested.
    osm_tiles: Option<HttpTiles>,
    mapbox_tiles: Option<HttpTiles>,
    tile_access: TileAccess,
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
    disambiguation_candidates: HoverCandidates,
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
    /// Whether the snapped-track renderer drew its edge tooltip, raised
    /// during the frame and read at the start of the next one.
    snapped_edge_tooltip_shown: Cell<bool>,
    /// The log hexagon the renderer found under the cursor, filled while the
    /// plugins draw and handed to the caller at the end of the frame.
    hovered_log_glyph: RefCell<Option<HoveredLogGlyph>>,
    /// How strongly the TEC heatmap is drawn, as the opacity control's
    /// percentage. A persisted preference, seeded from settings via
    /// [`NavMap::set_tec_heatmap_opacity_percent`].
    tec_heatmap_opacity_percent: f32,
}

impl NavMap {
    pub fn new(egui_ctx: Context, tile_access: TileAccess) -> Self {
        let icon_meshes = match icon_mesh::IconMeshLibrary::embedded() {
            Ok(library) => Some(library),
            Err(err) => {
                log::error!("icon meshes unavailable, marker icons will not be drawn: {err:#}");
                None
            }
        };
        Self {
            osm_tiles: match tile_access {
                TileAccess::Network => Some(HttpTiles::new(OpenStreetMap, egui_ctx.clone())),
                TileAccess::Offline => None,
            },
            mapbox_tiles: None,
            tile_access,
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
            disambiguation_candidates: HoverCandidates::default(),
            disambiguation_pos: egui::pos2(0.0, 0.0),
            display_toggle: display_toggle::DisplayToggleState::default(),
            display_counts_cache: display_counts::DisplayCountsCache::default(),
            icon_meshes,
            visible_points: viewport::VisiblePoints::default(),
            sat_label_scratch: sat_labels::LabelSelection::default(),
            sky_glyph_scratch: sky_glyph_renderer::GlyphSelection::default(),
            snapped_edge_tooltip_shown: Cell::new(false),
            hovered_log_glyph: RefCell::new(None),
            tec_heatmap_opacity_percent: gt_ui_theme::TEC_OPACITY_PERCENT_DEFAULT,
        }
    }

    /// The TEC heatmap's opacity percentage, for the app to persist to
    /// settings.
    pub const fn tec_heatmap_opacity_percent(&self) -> f32 {
        self.tec_heatmap_opacity_percent
    }

    /// Seed the TEC heatmap's opacity percentage from persisted settings,
    /// clamped to the valid range.
    pub fn set_tec_heatmap_opacity_percent(&mut self, percent: f32) {
        self.tec_heatmap_opacity_percent = percent.clamp(
            gt_ui_theme::TEC_OPACITY_PERCENT_MIN,
            gt_ui_theme::TEC_OPACITY_PERCENT_MAX,
        );
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
        if self.tile_access == TileAccess::Offline {
            self.mapbox_token = token;
            return;
        }
        if token.is_empty() {
            self.mapbox_token = String::new();
            self.mapbox_tiles = None;
        } else {
            self.mapbox_tiles = Some(HttpTiles::new(
                mapbox_tiles::satellite_source(token.clone()),
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

    /// The tile layer picker, rendered by the map's floating control and by the
    /// settings window's Interface page. Entries follow the caller's layout.
    pub fn show_layer_selector(&mut self, ui: &mut egui::Ui, satellite: SatelliteLayerAccess) {
        for (layer, icon, label) in [
            (MapLayer::OpenStreetMap, ICON_MAP_TRIFOLD, "Map"),
            (MapLayer::Satellite, ICON_GLOBE_HEMISPHERE_WEST, "Satellite"),
        ] {
            let selectable = layer != MapLayer::Satellite
                || satellite == SatelliteLayerAccess::WithoutToken
                || self.has_mapbox_token();
            if ui
                .add_enabled(
                    selectable,
                    Button::selectable(self.layer == layer, format!("{icon} {label}")),
                )
                .on_disabled_hover_text(SATELLITE_LAYER_NEEDS_TOKEN)
                .clicked()
            {
                self.layer = layer;
            }
        }
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

    /// Draw one frame of the map, and return the action the user asked for
    /// through a context menu or a popup button.
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        mut ctx: MapDrawContext<'_>,
    ) -> Option<MapContextAction> {
        let now = ui.ctx().input(|i| i.time);
        self.apply_camera_requests(ui, &ctx);
        self.adopt_new_files(ui, &ctx, now);
        let animation = self.tick_animations(ui, &ctx, now);

        // Read before the click below can open or close the popup, and reused
        // as-is by the compound hover label at the end of the frame.
        let disambig_open = self.disambiguation_is_open();
        ctx.suppress_overlapping_hover_labels(disambig_open);

        let map_center = self
            .map_memory
            .detached()
            .unwrap_or_else(default_map_center);
        let plan = self.collect_viewport_points(ui.max_rect(), map_center, &ctx);
        let map_response = self.show_map(ui, &ctx, &plan, animation);
        ctx.log_hover.glyph = self.hovered_log_glyph.take();

        if map_response.double_clicked()
            && let Some(bbox) = ctx.visible_bounding_box()
        {
            zoom_to_fit(&mut self.map_memory, map_response.rect, bbox);
        }

        let map_rect = map_response.rect;
        self.last_viewport_bounds = Some(compute_viewport_bounds(&self.map_memory, map_rect));

        // The mask is copied, so the display toggle below changes it only for
        // the next frame.
        let scope = ctx.scope();
        let hover = self.detect_hover(ui, &map_response, map_center, scope);

        self.show_overlay_controls(ui, map_rect, &mut ctx);

        let just_opened_disambig = self.apply_click(ui, &map_response, &mut ctx, scope, hover);
        self.show_disambiguation_popup(ui, &mut ctx, scope, just_opened_disambig);

        let mut context_action = self.show_context_menu(&map_response, &ctx, hover.primary());
        if let Some(request) = self.show_pinned_popup(ui, &mut ctx, scope) {
            context_action = Some(MapContextAction::ShowSkyTrails(request));
        }
        show_compound_hover_label(ui, &ctx, hover, disambig_open);

        ctx.highlight.hover = hover.primary().map(HighlightScope::Point);
        ctx.highlight.hover_candidates = hover;

        context_action
    }

    /// Apply the caller's camera requests: an explicit center, a position for
    /// the sticky window, and a fit around everything visible.
    fn apply_camera_requests(&mut self, ui: &egui::Ui, ctx: &MapDrawContext<'_>) {
        if let Some((lat, lon)) = ctx.center_request {
            self.map_memory.center_at(walkers::lat_lon(lat, lon));
        }
        if let Some(pos) = ctx.sticky_pos_override {
            self.sticky_pos = pos;
        }
        if ctx.zoom_to_visible
            && let Some(bbox) = ctx.visible_bounding_box()
        {
            zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
        }
    }

    /// Frame newly loaded files, start the load-highlight pulse, and rebuild
    /// the spatial index around them.
    ///
    /// A new file is visible by default, so it is always framed. Existing
    /// files honor their current visibility.
    fn adopt_new_files(&mut self, ui: &egui::Ui, ctx: &MapDrawContext<'_>, now: f64) {
        if ctx.files.len() <= self.last_file_count {
            return;
        }
        self.new_file_boundary = self.last_file_count;
        let had_tracks = self.last_file_count > 0;
        self.last_file_count = ctx.files.len();
        // Only blink when adding to existing content.
        if had_tracks {
            self.blink.trigger(now);
        }
        if let Some(bbox) = ctx.visible_bounding_box() {
            zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
        }
        self.global_tree = gt_track_builder::build_global_tree(ctx.files);
    }

    /// Advance the blink pulse and the hover-focus fade, requesting another
    /// frame while either is running.
    ///
    /// The fade reads the previous frame's highlight, which is what the
    /// renderers will see this frame.
    fn tick_animations(
        &mut self,
        ui: &egui::Ui,
        ctx: &MapDrawContext<'_>,
        now: f64,
    ) -> FrameAnimation {
        let blink_alpha = self.blink.tick(now);
        if self.blink.is_active() {
            request_animation_frame(ui);
        }
        let hover_fade = if ctx.highlight.fading_enabled {
            let progress = self.hover_fade.tick(
                now,
                track_renderer::hover_is_active(ctx.highlight),
                track_renderer::focused_track_from_highlight(ctx.highlight),
            );
            if self.hover_fade.is_animating() {
                request_animation_frame(ui);
            }
            progress
        } else {
            0.0
        };
        FrameAnimation {
            blink_alpha,
            hover_fade,
        }
    }

    /// Whether the disambiguation popup is holding candidates for a pick.
    fn disambiguation_is_open(&self) -> bool {
        self.disambiguation_candidates.primary().is_some()
    }

    /// Derive this frame's per-track drawing decisions and gather the points
    /// inside the viewport.
    ///
    /// Runs before the map widget takes its rect, so it works from the rect
    /// the layout is about to hand out.
    fn collect_viewport_points(
        &mut self,
        map_rect: egui::Rect,
        map_center: walkers::Position,
        ctx: &MapDrawContext<'_>,
    ) -> viewport::TrackPlan {
        let projector = walkers::Projector::new(map_rect, &self.map_memory, map_center);
        let transform = MercTransform::new(&projector, &self.map_memory, map_rect.center());
        let plan = viewport::TrackPlan::compute(
            ctx.files,
            ctx.visibility,
            ctx.filter,
            *ctx.display_mask,
            self.map_memory.zoom(),
        );
        viewport::collect_visible_points(
            &mut self.visible_points,
            &self.global_tree,
            &plan,
            &transform,
            map_rect,
        );
        plan
    }

    /// Whether the satellite layer may draw Mapbox tiles at the current zoom.
    ///
    /// Mapbox serves 512px tiles, and walkers' `tile_id` adjusts the integer
    /// zoom level by `log2(tile_size / 256)` - 1 for 512px tiles - by plain
    /// `u8` subtraction with no underflow check (walkers 0.53.0
    /// `mercator::tile_id`, src/mercator.rs:50). That panics with "attempt to
    /// subtract with overflow" once the zoom rounds down to 0, i.e. once it
    /// drops below 0.5. OSM's 256px tiles need no adjustment
    /// (`log2(256/256) == 0`) and so are immune at any zoom. Stay on OSM with
    /// enough margin below that line that no single frame's zoom delta can
    /// cross it.
    fn use_mapbox_tiles(&self) -> bool {
        self.layer == MapLayer::Satellite
            && self.mapbox_tiles.is_some()
            && self.map_memory.zoom() >= f64::from(MAPBOX_MIN_SAFE_ZOOM)
    }

    /// Build the base tile layer plus every enabled overlay plugin, then hand
    /// the map to the UI.
    ///
    /// A masked display category skips its whole plugin - the mask is the
    /// render-side AND on top of the per-track tree visibility the renderers
    /// already consume.
    fn show_map(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &MapDrawContext<'_>,
        plan: &viewport::TrackPlan,
        animation: FrameAnimation,
    ) -> egui::Response {
        let mut map = if self.use_mapbox_tiles() {
            let tiles = self
                .mapbox_tiles
                .as_mut()
                .map(|t| -> &mut dyn walkers::Tiles { t });
            Map::new(tiles, &mut self.map_memory, default_map_center())
        } else {
            let tiles = self
                .osm_tiles
                .as_mut()
                .map(|t| -> &mut dyn walkers::Tiles { t });
            Map::new(tiles, &mut self.map_memory, default_map_center())
        };
        let pointer_ownership = PointerOwnership {
            recorded_element_hovered: ctx.highlight.hover.is_some(),
            marker_hovered: ctx.highlight.hover_candidates.any_marker(),
            snapped_edge_tooltip_shown: self.snapped_edge_tooltip_shown.replace(false),
            interference_layer_drawn: ctx.display_mask.is_visible(DisplayCategory::JammingHexes)
                && ctx.jamming_dataset.is_some(),
        };
        // The overlays go on first so every track renderer draws over them,
        // and the global TEC grid goes under the interference cells.
        if ctx.display_mask.is_visible(DisplayCategory::TecHeatmap)
            && let Some(snapshot) = ctx.tec.snapshot
        {
            map = map.with_plugin(tec_renderer::TecHeatmapRenderer::new(
                snapshot,
                self.tec_heatmap_opacity_percent,
                pointer_ownership.tec_node_hover_enabled(),
            ));
        }
        if ctx.display_mask.is_visible(DisplayCategory::JammingHexes)
            && let Some(dataset) = ctx.jamming_dataset
        {
            map = map.with_plugin(jamming_renderer::JammingRenderer::new(
                dataset,
                pointer_ownership.jamming_cell_hover_enabled(),
            ));
        }
        map = map.with_plugin(
            TrackLayers::builder()
                .files(ctx.files)
                .plan(plan)
                .highlight(ctx.highlight)
                .filter(ctx.filter)
                .tpv_by_track(&self.visible_points.tpv_by_track)
                .new_file_boundary(self.new_file_boundary)
                .blink_alpha(animation.blink_alpha)
                .hover_fade_alpha(animation.hover_fade)
                .maybe_query_matches(ctx.query_matches)
                .display_query_highlights(
                    ctx.display_mask
                        .is_visible(DisplayCategory::QueryHighlights),
                )
                .sky_glyph_variant(*ctx.sky_glyph_variant)
                .maybe_icon_meshes(self.icon_meshes.as_ref())
                .recording_labels(ctx.recording_labels())
                .sat_label_scratch(&mut self.sat_label_scratch)
                .sky_glyph_scratch(&mut self.sky_glyph_scratch)
                .build(),
        );
        if ctx.display_mask.is_visible(DisplayCategory::SnappedTracks)
            && let Some(snapped) = ctx.snapped_tracks
            && !snapped.is_empty()
        {
            // Edge hover is disabled while a recorded element is hovered.
            map = map.with_plugin(SnappedTrackRenderer::new(
                snapped,
                ctx.files,
                pointer_ownership.snapped_track_hover_enabled(),
                &self.snapped_edge_tooltip_shown,
            ));
        }
        // Between the track line and the markers: a hexagon must not cover a
        // pin, and must be legible over the line it sits on. This renderer
        // also draws the ring at the viewer's hovered row, which has a
        // position even where the filters selected no line of its log.
        if ctx.display_mask.is_visible(DisplayCategory::LogMatches)
            && (!ctx.log_matches.is_empty() || ctx.log_hover.row_position.is_some())
        {
            map = map.with_plugin(
                log_match_renderer::LogMatchRenderer::builder()
                    .matches(ctx.log_matches)
                    .maybe_icon_meshes(self.icon_meshes.as_ref())
                    .dark_mode(ui.visuals().dark_mode)
                    .hover_enabled(pointer_ownership.log_hexagon_hover_enabled())
                    .maybe_hovered_row_position(ctx.log_hover.row_position)
                    .hovered_glyph(&self.hovered_log_glyph)
                    .build(),
            );
        }
        if ctx.display_mask.is_visible(DisplayCategory::CustomMarkers) {
            map = map.with_plugin(MarkerRenderer::new(
                ctx.files,
                ctx.visibility,
                ctx.highlight,
                ctx.filter,
                &self.visible_points.custom,
                self.icon_meshes.as_ref(),
            ));
        }
        if ctx
            .display_mask
            .is_visible(DisplayCategory::GeneratedMarkers)
        {
            map = map.with_plugin(
                GeneratedMarkerRenderer::builder()
                    .files(ctx.files)
                    .visibility(ctx.visibility)
                    .highlight(ctx.highlight)
                    .filter(ctx.filter)
                    .generated_vis(ctx.generated_marker_visibility)
                    .visible_generated(&self.visible_points.generated)
                    .maybe_icon_meshes(self.icon_meshes.as_ref())
                    .recording_labels(ctx.recording_labels())
                    .build(),
            );
        }
        if ctx.display_mask.is_visible(DisplayCategory::EventMarkers) {
            map = map.with_plugin(EventMarkerRenderer::new(
                ctx.files,
                ctx.visibility,
                ctx.highlight,
                ctx.filter,
                ctx.event_marker_visibility,
                &self.visible_points.event,
                self.icon_meshes.as_ref(),
            ));
        }
        ui.add(map)
    }

    /// The nearest visible element per category group within the hover
    /// threshold of the cursor.
    fn detect_hover(
        &self,
        ui: &egui::Ui,
        map_response: &egui::Response,
        map_center: walkers::Position,
        scope: MapScope<'_>,
    ) -> HoverCandidates {
        let mut hover = HoverCandidates::default();
        if !map_response.hovered() {
            return hover;
        }
        let Some(screen_pos) = ui.input(|i| i.pointer.hover_pos()) else {
            return hover;
        };
        // Recomputed from the rect the map actually took, so hit-testing lands
        // on what was drawn.
        let map_rect = map_response.rect;
        let projector = walkers::Projector::new(map_rect, &self.map_memory, map_center);
        let transform = MercTransform::new(&projector, &self.map_memory, map_rect.center());
        let merc_x = transform.merc_x_from_screen(screen_pos.x);
        let merc_y = transform.merc_y_from_screen(screen_pos.y);
        let px_per_merc = MapScale::from_zoom(self.map_memory.zoom()).px_per_merc();
        let threshold_merc_sq = (20.0_f64 / px_per_merc).powi(2);
        // One candidate per slot, in nearest-first order.
        for sp in self
            .global_tree
            .nearest_neighbor_iter([merc_x, merc_y])
            .take_while(|sp| sp.distance_2(&[merc_x, merc_y]) <= threshold_merc_sq)
        {
            if !is_spatial_point_visible(sp, scope) {
                continue;
            }
            hover.keep_nearest(DataPointRef {
                track: sp.track_ref(),
                category: sp.category,
                point_index: sp.point_index,
            });
            if hover.every_category_filled() {
                break;
            }
        }
        hover
    }

    /// The floating controls in the map's bottom-right corner: the tile layer
    /// picker, and the display toggle stacked above it.
    fn show_overlay_controls(
        &mut self,
        ui: &egui::Ui,
        map_rect: egui::Rect,
        ctx: &mut MapDrawContext<'_>,
    ) {
        let layer_toggle = Area::new(egui::Id::new("map_layer_toggle"))
            .fixed_pos(egui::pos2(map_rect.right() - 8.0, map_rect.bottom() - 8.0))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    self.show_layer_selector(ui, SatelliteLayerAccess::WithoutToken);
                });
            });

        // The counts closure only runs while the popup is open, and the cache
        // skips the full point walk when its inputs are unchanged frame to frame.
        let counts_cache = &mut self.display_counts_cache;
        let tec_nodes = ctx
            .tec
            .snapshot
            .as_ref()
            .map_or(0, TecHeatmapSnapshot::node_count);
        display_toggle::show_display_toggle(
            ui,
            layer_toggle.response.rect,
            &mut self.display_toggle,
            ctx.display_mask,
            ctx.sky_glyph_variant,
            display_toggle::LayerRows {
                interference: display_toggle::InterferenceRow {
                    day: ctx.day_selection,
                    empty_reason: ctx.empty_reason,
                },
                tec: display_toggle::TecRow {
                    instant: ctx.tec.instant,
                    opacity_percent: &mut self.tec_heatmap_opacity_percent,
                    empty_reason: ctx.tec.empty_reason,
                },
            },
            || {
                counts_cache.get(
                    ctx.files,
                    ctx.visibility,
                    ctx.filter,
                    ctx.event_marker_visibility,
                    ctx.generated_marker_visibility,
                    ctx.query_matches,
                    display_counts::SuppliedCounts {
                        snapped_tracks: ctx.snapped_tracks,
                        jamming_cells: ctx.jamming_dataset.map_or(0, JamDataset::len),
                        tec_nodes,
                        log_matches: ctx.log_matches.match_count(),
                    },
                )
            },
        );
    }

    /// Apply this frame's primary click: pin the hovered element, unpin it
    /// when it was already pinned, clear the pin on empty space, or open the
    /// disambiguation popup when several element types overlap.
    ///
    /// Returns whether the popup opened on this very frame.
    /// [`egui::Response::clicked_elsewhere`] on the popup's area fires on the
    /// same frame as the click that opened it - the click was on the map, not
    /// inside the popup - so the caller skips its dismissal check that frame
    /// and the popup does not flash.
    fn apply_click(
        &mut self,
        ui: &egui::Ui,
        map_response: &egui::Response,
        ctx: &mut MapDrawContext<'_>,
        scope: MapScope<'_>,
        hover: HoverCandidates,
    ) -> bool {
        if !map_response.clicked() {
            return false;
        }
        let click_pos = ui
            .ctx()
            .pointer_latest_pos()
            .unwrap_or(map_response.rect.center());
        if hover.is_ambiguous() {
            self.disambiguation_candidates = hover;
            self.disambiguation_pos = click_pos;
            return true;
        }
        self.disambiguation_candidates = HoverCandidates::default();
        if let Some(point_ref) = hover.primary() {
            if ctx.highlight.toggle_sticky_if_drawn(scope, point_ref) {
                self.sticky_pos = click_pos;
            }
        } else {
            ctx.highlight.sticky = None;
        }
        false
    }

    /// The popup that follows a click landing on several element types at
    /// once, one row per candidate. Picking a row, clicking outside it, or
    /// Escape closes it.
    fn show_disambiguation_popup(
        &mut self,
        ui: &egui::Ui,
        ctx: &mut MapDrawContext<'_>,
        scope: MapScope<'_>,
        just_opened: bool,
    ) {
        let candidates = self.disambiguation_candidates;
        if candidates.primary().is_none() {
            return;
        }
        let popup_pos = self.disambiguation_pos;
        let area_resp = Area::new(egui::Id::new("map_disambig"))
            .fixed_pos(popup_pos)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(160.0);
                    for candidate in candidates.iter() {
                        if draw_disambig_row(
                            ui,
                            candidate,
                            ctx.files,
                            ctx.highlight.sticky == Some(candidate),
                        )
                        .clicked()
                        {
                            if ctx.highlight.toggle_sticky_if_drawn(scope, candidate) {
                                self.sticky_pos = popup_pos;
                            }
                            self.disambiguation_candidates = HoverCandidates::default();
                        }
                    }
                });
            });
        let dismissal = DisambiguationDismissal {
            just_opened,
            clicked_elsewhere: area_resp.response.clicked_elsewhere(),
            escape_pressed: ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)),
        };
        if dismissal.closes_popup() {
            self.disambiguation_candidates = HoverCandidates::default();
        }
    }

    /// The right-click menu for the element under the pointer.
    ///
    /// The element is captured on the frame the right button fires, then held
    /// for the lifetime of the menu.
    fn show_context_menu(
        &mut self,
        map_response: &egui::Response,
        ctx: &MapDrawContext<'_>,
        hover_point_ref: Option<DataPointRef>,
    ) -> Option<MapContextAction> {
        if map_response.secondary_clicked() {
            self.right_click_ref = hover_point_ref;
        }
        let right_click_ref = self.right_click_ref;
        let mut action: Option<MapContextAction> = None;
        map_response.context_menu(|ui| {
            let Some(point_ref) = right_click_ref else {
                // Right-clicked on empty map space - nothing to show.
                ui.close();
                return;
            };
            let Some(file) = point_ref.track.fi.get(ctx.files) else {
                ui.close();
                return;
            };
            if let Some(name) = ctx.recording_labels().display_name(point_ref.track.fi) {
                ui.add(Label::new(RichText::new(name).weak()));
            }
            if file.tracks.len() > 1 {
                ui.add(Label::new(
                    RichText::new(format!("#{}", point_ref.track.index.as_usize() + 1)).weak(),
                ));
            }
            ui.separator();
            if ui.button("Only show elements from this track").clicked() {
                action = Some(MapContextAction::ShowOnlyTrack(point_ref.track));
                ui.close();
            }
            if ui.button("Only show elements from this file").clicked() {
                action = Some(MapContextAction::ShowOnlyFile(point_ref.track.fi));
                ui.close();
            }
            ui.separator();
            if ui.button("Show sky trails…").clicked() {
                action = Some(MapContextAction::ShowSkyTrails(
                    SkyTrailsRequest::whole_track(point_ref.track),
                ));
                ui.close();
            }
        });
        action
    }

    /// The persistent, text-selectable window for the pinned element, shown
    /// only while the map draws that element.
    ///
    /// Escape dismisses it. This is the single window shared by every map item
    /// type (TPV, satellite, markers), so handling the key here covers them all.
    ///
    /// Returns the request its header button raised, if any.
    fn show_pinned_popup(
        &self,
        ui: &egui::Ui,
        ctx: &mut MapDrawContext<'_>,
        scope: MapScope<'_>,
    ) -> Option<SkyTrailsRequest> {
        if ctx.highlight.sticky.is_some() && ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.highlight.sticky = None;
        }
        let PinnedPopup::Drawn(sticky_ref) = ctx.highlight.pin_this_frame(scope)? else {
            return None;
        };
        show_sticky_popup(
            ui.ctx(),
            ctx.files,
            ctx.recording_labels(),
            sticky_ref,
            self.sticky_pos,
            ctx.point_window_folds,
        )
    }
}

/// Default map center: Copenhagen.
fn default_map_center() -> walkers::Position {
    walkers::lat_lon(55.676, 12.565)
}

fn request_animation_frame(ui: &egui::Ui) {
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(16));
}

/// One stacked label near the cursor when several item types are hovered at
/// once.
///
/// Guarded on `suppress_hover_labels` (set from the previous frame's candidate
/// count) so that on the first frame of a multi-hover transition the individual
/// renderer tooltips show normally. From the second frame onward those are
/// suppressed and this label takes over, so the two never appear at once.
fn show_compound_hover_label(
    ui: &egui::Ui,
    ctx: &MapDrawContext<'_>,
    hover: HoverCandidates,
    disambig_open: bool,
) {
    if !should_show_compound_label(
        hover.is_ambiguous(),
        disambig_open,
        ctx.highlight.suppress_hover_labels,
    ) {
        return;
    }
    let Some(cursor_pos) = ui.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    Area::new(egui::Id::new("map_multi_hover_labels"))
        .fixed_pos(cursor_pos + egui::vec2(15.0, 10.0))
        .order(egui::Order::Tooltip)
        .show(ui.ctx(), |ui| {
            draw_multi_hover_label_contents(ui, hover, ctx.files, ctx.recording_labels());
        });
}

/// The clicked-point window body: the deselect hint pinned to the window floor
/// as an inner bottom panel, with the plot-and-satellites content scrolling in
/// the space above it.
///
/// Returns whether the user asked to open the sky trails window at this
/// point's instant.
#[must_use]
fn show_point_window_body(
    ui: &mut egui::Ui,
    point: &gt_types::NavPoint,
    sky: &crate::tpv_renderer::SkySection<'_>,
    folds: &mut PointWindowFolds,
    recording_name: Option<&str>,
) -> bool {
    egui::Panel::bottom("sticky_point_hint").show(ui, |ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("Click to deselect").small().weak());
    });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            crate::tpv_renderer::show_sticky_tpv_content(ui, point, sky, folds, recording_name)
        })
        .inner
}

/// Returns a request to open the sky trails window when the point window's
/// header button was pressed, carrying the clicked point's instant.
#[must_use]
fn show_sticky_popup(
    ctx: &egui::Context,
    files: &[LoadedFile],
    recording_labels: RecordingLabels<'_>,
    sticky_ref: DataPointRef,
    default_pos: egui::Pos2,
    folds: &mut PointWindowFolds,
) -> Option<SkyTrailsRequest> {
    let title: String = match sticky_ref.category {
        DataCategory::Tpv => sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.points))
            .map_or_else(
                || "GNSS fix".to_string(),
                |p| p.tpv.time().utc().format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
        DataCategory::SatelliteReport => sticky_ref
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
            ),
        DataCategory::GeneratedMarker => sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.generated_markers))
            .map_or_else(
                || "GNSS event".to_string(),
                |m| m.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
        DataCategory::EventMarker => sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.event_markers))
            .map_or_else(
                || "Event".to_string(),
                |m| m.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
        DataCategory::CustomMarker => sticky_ref
            .track
            .fi
            .get(files)
            .and_then(|f| sticky_ref.track.index.get(&f.tracks))
            .and_then(|t| sticky_ref.point_index.get(&t.custom_markers))
            .map_or_else(
                || "Custom marker".to_string(),
                |m| m.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
        DataCategory::Track => String::new(),
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
        DataCategory::Tpv | DataCategory::SatelliteReport => {
            if let Some(track) = sticky_ref
                .track
                .fi
                .get(files)
                .and_then(|f| sticky_ref.track.index.get(&f.tracks))
                && let Some(point) = sticky_ref.point_index.get(&track.points)
            {
                let sky = crate::tpv_renderer::SkySection::resolve(track, sticky_ref.point_index);
                if show_point_window_body(
                    ui,
                    point,
                    &sky,
                    folds,
                    recording_labels.name_when_several_files_loaded(sticky_ref.track.fi),
                ) {
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
                // The window title already shows the time.
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

/// The per-frame state a [`MapDrawContext`] borrows, owned so a test can
/// spell out only the inputs it is about and take the defaults for the rest.
#[cfg(test)]
struct DrawState {
    recording_names: RecordingNames,
    filter: GlobalFilter,
    event_marker_visibility: EventMarkerVisibility,
    generated_marker_visibility: GeneratedMarkerVisibility,
    display_mask: DisplayMask,
    sky_glyph_variant: SkyGlyphVariant,
    point_window_folds: PointWindowFolds,
    highlight: MapHighlight,
    day_selection: DaySelection,
    tec_instant: gt_ionex::TecInstantSelection,
    log_matches: LogMatches,
    log_hover: LogMatchHover,
}

#[cfg(test)]
impl Default for DrawState {
    fn default() -> Self {
        Self {
            recording_names: RecordingNames::default(),
            log_matches: LogMatches::default(),
            log_hover: LogMatchHover::default(),
            filter: GlobalFilter::default(),
            event_marker_visibility: EventMarkerVisibility::default(),
            generated_marker_visibility: GeneratedMarkerVisibility::default(),
            display_mask: DisplayMask::default(),
            sky_glyph_variant: SkyGlyphVariant::default(),
            point_window_folds: PointWindowFolds::default(),
            highlight: MapHighlight::default(),
            day_selection: DaySelection::new(None, gt_jam::calendar::today_utc()),
            tec_instant: gt_ionex::TecInstantSelection::new(None, chrono::Utc::now().date_naive()),
        }
    }
}

#[cfg(test)]
impl DrawState {
    /// The context for one [`NavMap::draw`] call, with every optional input
    /// absent. Override what a test is about with struct update syntax.
    fn context<'a>(
        &'a mut self,
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
    ) -> MapDrawContext<'a> {
        MapDrawContext {
            files,
            recording_names: &self.recording_names,
            snapped_tracks: None,
            jamming_dataset: None,
            tec: TecLayer {
                snapshot: None,
                instant: &mut self.tec_instant,
                empty_reason: None,
            },
            query_matches: None,
            log_matches: &self.log_matches,
            log_hover: &mut self.log_hover,
            empty_reason: None,
            filter: &self.filter,
            visibility,
            event_marker_visibility: &self.event_marker_visibility,
            generated_marker_visibility: &self.generated_marker_visibility,
            display_mask: &mut self.display_mask,
            day_selection: &mut self.day_selection,
            highlight: &mut self.highlight,
            sky_glyph_variant: &mut self.sky_glyph_variant,
            point_window_folds: &mut self.point_window_folds,
            center_request: None,
            zoom_to_visible: false,
            sticky_pos_override: None,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod reference_illustration;

#[cfg(test)]
mod snapshot_tests;
