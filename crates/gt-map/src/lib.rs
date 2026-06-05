pub mod event_marker_renderer;
pub mod generated_marker_renderer;
pub mod marker_renderer;
#[cfg(test)]
mod test_harness;
pub mod tpv_renderer;
pub mod track_renderer;

use egui::Context;

/// Per-frame transform context for projecting pre-computed normalised Mercator
/// coordinates to screen pixel positions with full f64 precision.
///
/// ## Why not `projector.project()`?
///
/// Walkers' `Projector::project()` computes the screen position of an arbitrary
/// geographic point by subtracting two large pixel values (the point's Mercator
/// pixel position and the map centre's) in f64, then calling `.to_vec2()` which
/// truncates the result to f32. At zoom ≥ 17 and for data far from the origin
/// (e.g. Denmark: lat 55° N, lon 12° E), the y-component of this difference is
/// ≈ 12 M px, where f32 ULP = 1 px. The anchor obtained this way has ≈ ±0.5 px
/// constant error per frame, which appears as snapping during smooth zoom
/// animations. Additionally, viewport culling arithmetic done in f32 before
/// casting to f64 has the same issue.
///
/// ## Solution
///
/// This transform uses `projector.unproject(clip_center)` to obtain the map
/// centre's geographic coordinates. Walkers' `unproject` performs all arithmetic
/// in f64 and returns a high-precision `Position`. We then compute the normalised
/// Mercator coordinates of the map centre in f64, and express every point's
/// screen position as:
///
/// ```text
/// screen = clip_center + (merc_point − merc_center) × total_px
/// ```
///
/// `clip_center` is a small, exact f32 value (the pixel centre of the map
/// widget, typically around 800–1000 px). `merc_point − merc_center` is a
/// small number (≤ ~0.05 for any visible point), so no large-magnitude
/// arithmetic occurs anywhere. The final cast to f32 is applied only to the
/// already-small screen coordinate, where f32 ULP < 0.001 px.
pub(crate) struct MercTransform {
    clip_center_x: f64,
    clip_center_y: f64,
    merc_center: MercPoint,
    total_px: f64,
}

impl MercTransform {
    /// Build the transform for the current frame.
    ///
    /// `clip_center` must be `ui.max_rect().center()` inside a plugin's
    /// `run()` method - walkers sets the child UI rect to the map widget rect,
    /// which is also `projector`'s clip rect, so `clip_center` equals
    /// `projector.clip_rect.center()`.
    pub(crate) fn new(
        projector: &walkers::Projector,
        map_memory: &MapMemory,
        clip_center: egui::Pos2,
    ) -> Self {
        let total_px = 2_f64.powf(map_memory.zoom()) * 256.0;
        // unproject(clip_center) returns the geographic position at the
        // viewport centre using f64 arithmetic throughout.
        // In walkers: Position.x() = longitude, Position.y() = latitude.
        let center_ll = projector.unproject(clip_center.to_vec2());
        let merc_center =
            mercator::normalize(Latitude::new(center_ll.y()), Longitude::new(center_ll.x()));
        Self {
            clip_center_x: clip_center.x as f64,
            clip_center_y: clip_center.y as f64,
            merc_center,
            total_px,
        }
    }

    /// Project a pre-computed normalised Mercator coordinate to a screen position.
    #[inline]
    pub(crate) fn to_screen(&self, merc: MercPoint) -> egui::Pos2 {
        egui::pos2(
            (self.clip_center_x + (merc.x - self.merc_center.x) * self.total_px) as f32,
            (self.clip_center_y + (merc.y - self.merc_center.y) * self.total_px) as f32,
        )
    }

    /// Convert a screen-space x-coordinate to a normalised Mercator x value.
    #[inline]
    pub(crate) fn merc_x_from_screen(&self, screen_x: f32) -> f64 {
        (screen_x as f64 - self.clip_center_x) / self.total_px + self.merc_center.x
    }

    /// Convert a screen-space y-coordinate to a normalised Mercator y value.
    #[inline]
    pub(crate) fn merc_y_from_screen(&self, screen_y: f32) -> f64 {
        (screen_y as f64 - self.clip_center_y) / self.total_px + self.merc_center.y
    }

    /// Pixels per metre at the given latitude.
    ///
    /// Uses the Web Mercator scale factor: the equatorial circumference
    /// (≈ 40 030 km) shrinks by cos(lat) at a given latitude.
    #[inline]
    pub(crate) fn pixels_per_meter(&self, lat: Latitude) -> f64 {
        // At zoom z the world is 256·2^z pixels wide at the equator.
        // 1 Mercator tile column = Earth circumference / 2^z metres at the equator,
        // scaled by cos(lat) at higher latitudes.
        const EARTH_CIRCUMFERENCE_M: f64 = 40_030_173.0;
        self.total_px / (EARTH_CIRCUMFERENCE_M * lat.as_degrees().to_radians().cos())
    }
}

// URI constants used by the marker renderer and the startup registration call.
pub(crate) const ICON_URI_LIGHTNING: &str = "bytes://gt-map/icons/lightning.svg";
pub(crate) const ICON_URI_WARNING: &str = "bytes://gt-map/icons/warning.svg";
pub(crate) const ICON_URI_ERROR: &str = "bytes://gt-map/icons/error.svg";
pub(crate) const ICON_URI_LOG_PIN: &str = "bytes://gt-map/icons/log_pin.svg";
pub(crate) const ICON_URI_PIN: &str = "bytes://gt-map/icons/pin.svg";
pub(crate) const ICON_URI_CROSS: &str = "bytes://gt-map/icons/cross.svg";
pub(crate) const ICON_URI_CIRCLE_MARKER: &str = "bytes://gt-map/icons/circle_marker.svg";
pub(crate) const ICON_URI_CHECK: &str = "bytes://gt-map/icons/check.svg";
pub(crate) const ICON_URI_SATELLITE: &str = "bytes://gt-map/icons/satellite.svg";
pub(crate) const ICON_URI_SATELLITE_LOST: &str = "bytes://gt-map/icons/satellite_lost.svg";
pub(crate) const ICON_URI_GEAR: &str = "bytes://gt-map/icons/gear.svg";
pub(crate) const ICON_URI_REFRESH: &str = "bytes://gt-map/icons/refresh.svg";
pub(crate) const ICON_URI_DOWNLOAD: &str = "bytes://gt-map/icons/download.svg";
pub(crate) const ICON_URI_UPLOAD: &str = "bytes://gt-map/icons/upload.svg";
pub(crate) const ICON_URI_WRENCH: &str = "bytes://gt-map/icons/wrench.svg";
pub(crate) const ICON_URI_GHOST_FIX: &str = "bytes://gt-map/icons/ghost_fix.svg";

/// Register the embedded SVG marker icons with the egui context.
///
/// Call this once at startup (before the first frame) from your `App::new`
/// implementation, **after** [`egui_extras::install_image_loaders`] has been
/// called. The icons are compiled into the binary via `include_bytes!` and
/// cached by egui's texture system after their first rasterisation; subsequent
/// frames pay only a GPU quad draw - no CPU tessellation, no heap allocation.
macro_rules! icon_bytes {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/icons/",
            $name
        ))
        .as_slice()
    };
}

pub fn register_marker_icons(ctx: &egui::Context) {
    ctx.include_bytes(ICON_URI_LIGHTNING, icon_bytes!("lightning.svg"));
    ctx.include_bytes(ICON_URI_WARNING, icon_bytes!("warning.svg"));
    ctx.include_bytes(ICON_URI_ERROR, icon_bytes!("error.svg"));
    ctx.include_bytes(ICON_URI_LOG_PIN, icon_bytes!("log_pin.svg"));
    ctx.include_bytes(ICON_URI_PIN, icon_bytes!("pin.svg"));
    ctx.include_bytes(ICON_URI_CROSS, icon_bytes!("cross.svg"));
    ctx.include_bytes(ICON_URI_CIRCLE_MARKER, icon_bytes!("circle_marker.svg"));
    ctx.include_bytes(ICON_URI_CHECK, icon_bytes!("check.svg"));
    ctx.include_bytes(ICON_URI_SATELLITE, icon_bytes!("satellite.svg"));
    ctx.include_bytes(ICON_URI_SATELLITE_LOST, icon_bytes!("satellite_lost.svg"));
    ctx.include_bytes(ICON_URI_GEAR, icon_bytes!("gear.svg"));
    ctx.include_bytes(ICON_URI_REFRESH, icon_bytes!("refresh.svg"));
    ctx.include_bytes(ICON_URI_DOWNLOAD, icon_bytes!("download.svg"));
    ctx.include_bytes(ICON_URI_UPLOAD, icon_bytes!("upload.svg"));
    ctx.include_bytes(ICON_URI_WRENCH, icon_bytes!("wrench.svg"));
    ctx.include_bytes(ICON_URI_GHOST_FIX, icon_bytes!("ghost_fix.svg"));
}
/// Draw an SVG marker icon at `rect`, with optional `tint`.
///
/// The resolved `TextureId` is cached in egui's context data store after the
/// first successful load so that subsequent frames skip the URI hash and image
/// cache lookup and go directly to `painter.add(Shape::image(...))`.
pub(crate) fn draw_cached_icon(
    ui: &egui::Ui,
    uri: &'static str,
    rect: egui::Rect,
    tint: egui::Color32,
) {
    let cache_key = egui::Id::new(("gt_icon_tex", uri));
    if let Some(tex_id) = ui.ctx().data(|d| d.get_temp::<egui::TextureId>(cache_key)) {
        ui.painter().add(egui::Shape::image(
            tex_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        ));
        return;
    }
    if let Ok(egui::load::TexturePoll::Ready { texture }) = ui.ctx().try_load_texture(
        uri,
        egui::TextureOptions::LINEAR,
        egui::load::SizeHint::default(),
    ) {
        ui.ctx().data_mut(|d| d.insert_temp(cache_key, texture.id));
        ui.painter().add(egui::Shape::image(
            texture.id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        ));
    }
}

/// Draw a rotated SVG icon centred on `center`, with the icon's "up" direction aligned to
/// `direction`.
///
/// The icon is rendered as a rotated [`egui::epaint::Mesh`] quad so it can be oriented to any
/// travel direction without re-rasterising the SVG. `size` is the half-extent: the quad spans
/// `2*size × 2*size` pixels. The white SVG stroke is multiplied by `tint` at render time so a
/// single texture serves all colours.
pub(crate) fn draw_rotated_cached_icon(
    ui: &egui::Ui,
    uri: &'static str,
    center: egui::Pos2,
    direction: egui::Vec2,
    size: f32,
    tint: egui::Color32,
) {
    let cache_key = egui::Id::new(("gt_icon_tex", uri));
    let tex_id = if let Some(id) = ui.ctx().data(|d| d.get_temp::<egui::TextureId>(cache_key)) {
        id
    } else if let Ok(egui::load::TexturePoll::Ready { texture }) = ui.ctx().try_load_texture(
        uri,
        egui::TextureOptions::LINEAR,
        egui::load::SizeHint::default(),
    ) {
        ui.ctx().data_mut(|d| d.insert_temp(cache_key, texture.id));
        texture.id
    } else {
        return;
    };

    // Rotate the four corners of a [-size, size]² quad so the SVG's "up" direction (0, −1)
    // aligns with `direction`. Rotation matrix R where R*(0,−1) = (dx,dy):
    //   R*[px, py] = (−px·dy − py·dx,  px·dx − py·dy)
    let dx = direction.x;
    let dy = direction.y;
    let corner_offsets: [([f32; 2], egui::Pos2); 4] = [
        ([-size, -size], egui::pos2(0.0, 0.0)), // top-left    → UV (0,0)
        ([size, -size], egui::pos2(1.0, 0.0)),  // top-right   → UV (1,0)
        ([size, size], egui::pos2(1.0, 1.0)),   // bottom-right → UV (1,1)
        ([-size, size], egui::pos2(0.0, 1.0)),  // bottom-left → UV (0,1)
    ];

    let mut mesh = egui::epaint::Mesh::with_texture(tex_id);
    for ([px, py], uv) in corner_offsets {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + egui::vec2(-px * dy - py * dx, px * dx - py * dy),
            uv,
            color: tint,
        });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    ui.painter().add(egui::Shape::Mesh(mesh.into()));
}

use gt_filter::GlobalFilter;
use gt_types::mercator;
use gt_types::{
    DataCategory, FileIdx, Latitude, LoadedFile, Longitude, MercPoint, SpatialPoint, TrackRef,
};
use gt_ui_types::{
    DataPointRef, EventMarkerVisibility, HighlightScope, MapHighlight, TrackDataVisibility,
};
use rstar::PointDistance as _;
use std::time::Instant;
use walkers::sources::{Mapbox, MapboxStyle, OpenStreetMap};
use walkers::{HttpTiles, Map, MapMemory};

use crate::event_marker_renderer::EventMarkerRenderer;
use crate::generated_marker_renderer::GeneratedMarkerRenderer;
use crate::marker_renderer::MarkerRenderer;
use crate::tpv_renderer::TpvRenderer;
use crate::track_renderer::TrackRenderer;

/// Which tile source to use for the background map.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum MapLayer {
    #[default]
    OpenStreetMap,
    Satellite,
}

/// Action requested from a right-click context menu on a map element.
///
/// Returned by [`NavMap::draw`] when the user selects an item; the caller is
/// responsible for applying it to the visibility state.
#[derive(Debug, Clone, Copy)]
pub enum MapContextAction {
    /// Hide every track except the given one.
    ShowOnlyTrack(TrackRef),
    /// Hide every file except the given one.
    ShowOnlyFile(FileIdx),
}

/// Manages the load-highlight pulse animation.
///
/// Tracks when the animation started and provides the per-frame alpha value.
/// Once expired it clears itself so callers can avoid unnecessary repaints.
struct BlinkState {
    start: Option<Instant>,
}

impl BlinkState {
    fn trigger(&mut self) {
        self.start = Some(Instant::now());
    }

    /// Returns alpha in `[0.0, 1.0]` for the current frame.
    /// Resets the timer when the animation expires so `is_active` returns
    /// `false` on the same frame the last pulse ends.
    fn tick(&mut self) -> f32 {
        let Some(start) = self.start else {
            return 0.0;
        };
        let elapsed = start.elapsed().as_secs_f32();
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

/// Geographic bounding box of the currently visible map viewport.
#[derive(Debug, Clone, Copy)]
pub struct GeoBounds {
    pub lat_min: f64,
    pub lat_max: f64,
    pub lon_min: f64,
    pub lon_max: f64,
}

pub struct NavMap {
    egui_ctx: Context,
    osm_tiles: HttpTiles,
    mapbox_tiles: Option<HttpTiles>,
    mapbox_token: String,
    layer: MapLayer,
    map_memory: MapMemory,
    global_tree: rstar::RTree<SpatialPoint>,
    /// Screen position where the last sticky click happened; used as the
    /// default position for the sticky info window.
    sticky_pos: egui::Pos2,
    /// How many files were loaded last frame - used to detect new loads.
    last_file_count: usize,
    /// Load-highlight pulse animation state.
    blink: BlinkState,
    /// Index of the first newly loaded file; files[new_file_boundary..] are new.
    new_file_boundary: usize,
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
}

impl NavMap {
    pub fn new(egui_ctx: Context) -> Self {
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
            last_viewport_bounds: None,
            right_click_ref: None,
            disambiguation_candidates: [None; 4],
            disambiguation_pos: egui::pos2(0.0, 0.0),
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
        event_marker_visibility: &EventMarkerVisibility,
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

        if zoom_to_visible && let Some(bbox) = compute_visible_bounding_box(files, visibility) {
            zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
        }

        // Detect newly loaded files → zoom to fit all data + start blink animation.
        if files.len() > self.last_file_count {
            self.new_file_boundary = self.last_file_count;
            let had_tracks = self.last_file_count > 0;
            self.last_file_count = files.len();
            // Only blink when adding to existing content; the first load needs no
            // visual callout because there is nothing else to differentiate from.
            if had_tracks {
                self.blink.trigger();
            }
            if let Some(bbox) = compute_bounding_box(files) {
                zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
            }
            self.global_tree = gt_track_builder::build_global_tree(files);
        }

        let blink_alpha = self.blink.tick();
        if self.blink.is_active() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }

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

        let (visible_tpv, visible_custom, visible_generated, visible_event) = {
            let lt = map_rect_estimate.left_top();
            let rb = map_rect_estimate.right_bottom();
            let aabb = rstar::AABB::from_corners(
                [
                    transform_estimate.merc_x_from_screen(lt.x),
                    transform_estimate.merc_y_from_screen(lt.y),
                ],
                [
                    transform_estimate.merc_x_from_screen(rb.x),
                    transform_estimate.merc_y_from_screen(rb.y),
                ],
            );
            let mut tpv: Vec<SpatialPoint> = Vec::new();
            let mut custom: Vec<SpatialPoint> = Vec::new();
            let mut generated: Vec<SpatialPoint> = Vec::new();
            let mut event: Vec<SpatialPoint> = Vec::new();
            for sp in self.global_tree.locate_in_envelope(aabb) {
                match sp.category {
                    DataCategory::Tpv => tpv.push(*sp),
                    DataCategory::CustomMarker => custom.push(*sp),
                    DataCategory::GeneratedMarker => generated.push(*sp),
                    DataCategory::EventMarker => event.push(*sp),
                    _ => {}
                }
            }
            (tpv, custom, generated, event)
        };

        let use_mapbox = self.layer == MapLayer::Satellite && self.mapbox_tiles.is_some();
        let map = if use_mapbox {
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
        };
        let map = map
            .with_plugin(TrackRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                self.new_file_boundary,
                blink_alpha,
            ))
            .with_plugin(TpvRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                visible_tpv,
            ))
            .with_plugin(MarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                visible_custom,
            ))
            .with_plugin(GeneratedMarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                visible_generated,
            ))
            .with_plugin(EventMarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                event_marker_visibility,
                visible_event,
            ));

        let map_response = ui.add(map);

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
                let total_px = 2_f64.powf(self.map_memory.zoom()) * 256.0;
                let threshold_merc_sq = (20.0_f64 / total_px).powi(2);
                // First pass: one candidate per slot, in nearest-first order.
                for sp in self
                    .global_tree
                    .nearest_neighbor_iter([merc_x, merc_y])
                    .take_while(|sp| sp.distance_2(&[merc_x, merc_y]) <= threshold_merc_sq)
                {
                    if !is_spatial_point_visible(sp, visibility) {
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
        egui::Area::new(egui::Id::new("map_layer_toggle"))
            .fixed_pos(egui::pos2(map_rect.right() - 8.0, map_rect.bottom() - 8.0))
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    for (layer, icon, label) in [
                        (
                            MapLayer::OpenStreetMap,
                            egui_phosphor::regular::MAP_TRIFOLD,
                            "Map",
                        ),
                        (
                            MapLayer::Satellite,
                            egui_phosphor::regular::GLOBE_HEMISPHERE_WEST,
                            "Satellite",
                        ),
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

        // Handle click: clicking near a map element makes its info popup sticky;
        // clicking on empty space clears it. Clicking the same element again also clears it.
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
            let area_resp = egui::Area::new(egui::Id::new("map_disambig"))
                .fixed_pos(disambig_pos)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
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
            ui.add(egui::Label::new(
                egui::RichText::new(file.metadata.filename.as_str()).weak(),
            ));
            if file.tracks.len() > 1 {
                ui.add(egui::Label::new(
                    egui::RichText::new(format!("#{}", point_ref.track.index.as_usize() + 1))
                        .weak(),
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
        });

        // Show a persistent, text-selectable info window for the sticky element.
        if let Some(sticky_ref) = highlight.sticky {
            show_sticky_popup(ui.ctx(), files, sticky_ref, self.sticky_pos);
        }

        // When multiple different item types are hovered simultaneously, draw a
        // single compact stacked label near the cursor instead of letting each
        // renderer place its own label (which would all overlap at the same spot).
        let current_multi_hover = hover_candidates.iter().flatten().count() > 1;
        if current_multi_hover
            && !disambig_open
            && let Some(cursor_pos) = ui.input(|i| i.pointer.hover_pos())
        {
            egui::Area::new(egui::Id::new("map_multi_hover_labels"))
                .fixed_pos(cursor_pos + egui::vec2(15.0, 10.0))
                .order(egui::Order::Tooltip)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        draw_multi_hover_label_contents(ui, &hover_candidates, files);
                    });
                });
        }

        highlight.hover = hover_point_ref.map(HighlightScope::Point);
        highlight.hover_candidates = hover_candidates;

        context_action
    }
}

/// Renders the rows of a multi-hover stacked label into `ui`.
///
/// Callers wrap this in `egui::Frame::popup` to get the border and padding.
pub(crate) fn draw_multi_hover_label_contents(
    ui: &mut egui::Ui,
    candidates: &[Option<DataPointRef>; 4],
    files: &[LoadedFile],
) {
    for candidate in candidates.iter().flatten().copied() {
        let icon = category_icon(candidate.category);
        let label = candidate_label(candidate, files);
        ui.label(format!("{icon}  {label}"));
    }
}

/// Renders a single row of the disambiguation popup.
///
/// Returns the response from `selectable_label` so the caller can check `.clicked()`.
pub(crate) fn draw_disambig_row(
    ui: &mut egui::Ui,
    candidate: DataPointRef,
    files: &[LoadedFile],
    is_selected: bool,
) -> egui::Response {
    let icon = category_icon(candidate.category);
    let label = candidate_label(candidate, files);
    let mut job = egui::text::LayoutJob::default();
    let text_color = ui.visuals().text_color();
    job.append(
        icon,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(20.0),
            color: text_color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        &format!("  {label}"),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color: text_color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    ui.selectable_label(is_selected, job)
}

pub(crate) fn category_icon(cat: DataCategory) -> &'static str {
    match cat {
        DataCategory::Tpv | DataCategory::SatelliteReport => egui_phosphor::regular::CROSSHAIR,
        DataCategory::EventMarker => egui_phosphor::regular::FLAG,
        DataCategory::CustomMarker => egui_phosphor::regular::MAP_PIN,
        DataCategory::GeneratedMarker => egui_phosphor::regular::ARROWS_SPLIT,
        DataCategory::Track => "",
    }
}

pub(crate) fn candidate_label(candidate: DataPointRef, files: &[LoadedFile]) -> String {
    let Some(file) = candidate.track.fi.get(files) else {
        return String::new();
    };
    let Some(track) = candidate.track.index.get(&file.tracks) else {
        return String::new();
    };
    match candidate.category {
        DataCategory::Tpv | DataCategory::SatelliteReport => {
            if let Some(p) = candidate.point_index.get(&track.points) {
                p.tpv.time().utc().format("GNSS fix  %H:%M:%S").to_string()
            } else {
                "GNSS fix".to_string()
            }
        }
        DataCategory::EventMarker => {
            if let Some(m) = candidate.point_index.get(&track.event_markers) {
                match &m.annotation {
                    Some(note) if !note.is_empty() => {
                        format!("{}  -  {note}", m.variant_path)
                    }
                    _ => m.variant_path.clone(),
                }
            } else {
                "Event marker".to_string()
            }
        }
        DataCategory::CustomMarker => {
            if let Some(m) = candidate.point_index.get(&track.custom_markers) {
                m.label.clone()
            } else {
                "Custom marker".to_string()
            }
        }
        DataCategory::GeneratedMarker => {
            if let Some(m) = candidate.point_index.get(&track.generated_markers) {
                match m.kind {
                    gt_types::GeneratedMarkerKind::GpsFixLost => "GNSS fix lost".to_string(),
                    gt_types::GeneratedMarkerKind::GpsFixRegained => {
                        "GNSS fix regained".to_string()
                    }
                }
            } else {
                "Generated marker".to_string()
            }
        }
        DataCategory::Track => String::new(),
    }
}

/// Shows a draggable, text-selectable egui window with data for the given sticky element.
fn show_sticky_popup(
    ctx: &egui::Context,
    files: &[LoadedFile],
    sticky_ref: DataPointRef,
    default_pos: egui::Pos2,
) {
    use crate::tpv_renderer::show_sticky_tpv_content;
    use gt_types::{DataCategory, GeneratedMarkerKind};

    // For TPV points, satellite reports, and generated-marker events the window
    // title is the point's datetime; for everything else fall back to a generic label.
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

    egui::Window::new(title)
        .id(egui::Id::new(("sticky_popup", sticky_ref)))
        .default_pos(default_pos)
        .collapsible(false)
        .auto_sized()
        .show(ctx, |ui| match sticky_ref.category {
            DataCategory::Tpv => {
                if let Some(point) = sticky_ref
                    .track
                    .fi
                    .get(files)
                    .and_then(|f| sticky_ref.track.index.get(&f.tracks))
                    .and_then(|t| sticky_ref.point_index.get(&t.points))
                {
                    // Cap the window height so satellite tables never overflow
                    // the screen. For small satellite counts the ScrollArea
                    // auto-sizes to content (no scroll bar); it only activates
                    // when content is taller than the cap - e.g. when the UI is
                    // zoomed in with Ctrl-+ or when there are many satellites.
                    let max_h = (ui.ctx().viewport_rect().height() * 0.75).min(500.0);
                    egui::ScrollArea::vertical()
                        .max_height(max_h)
                        .show(ui, |ui| {
                            show_sticky_tpv_content(ui, point);
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
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
                    egui::Grid::new("sticky_marker_grid")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Label");
                            ui.add(egui::Label::new(marker.label.as_str()).selectable(true));
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
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
                    let kind_str = match marker.kind {
                        GeneratedMarkerKind::GpsFixLost => "GNSS fix lost",
                        GeneratedMarkerKind::GpsFixRegained => "GNSS fix regained",
                    };
                    egui::Grid::new("sticky_gen_grid")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Time");
                            ui.add(
                                egui::Label::new(
                                    marker.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                                )
                                .selectable(true),
                            );
                            ui.end_row();
                            ui.label("Event");
                            ui.add(egui::Label::new(kind_str).selectable(true));
                            ui.end_row();
                            ui.label("Position");
                            ui.add(
                                egui::Label::new(format!(
                                    "{:.6}, {:.6}",
                                    marker.lat.as_degrees(),
                                    marker.lon.as_degrees()
                                ))
                                .selectable(true),
                            );
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
                }
            }
            DataCategory::SatelliteReport => {
                if let Some(point) = sticky_ref
                    .track
                    .fi
                    .get(files)
                    .and_then(|f| sticky_ref.track.index.get(&f.tracks))
                    .and_then(|t| sticky_ref.point_index.get(&t.points))
                {
                    let max_h = (ui.ctx().viewport_rect().height() * 0.75).min(500.0);
                    egui::ScrollArea::vertical()
                        .max_height(max_h)
                        .show(ui, |ui| {
                            show_sticky_tpv_content(ui, point);
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
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
                    egui::Grid::new("sticky_event_marker_grid")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Event");
                            ui.add(egui::Label::new(marker.variant_path.as_str()).selectable(true));
                            ui.end_row();
                            if let Some(ann) = &marker.annotation {
                                ui.label("Note");
                                ui.add(egui::Label::new(ann.as_str()).selectable(true));
                                ui.end_row();
                            }
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
                }
            }
        });
}

/// Bounding box over only the currently **visible** tracks (those with both their
/// file and track enabled). Returns `None` if no visible data exists.
fn compute_visible_bounding_box(
    files: &[LoadedFile],
    visibility: &TrackDataVisibility,
) -> Option<(f64, f64, f64, f64)> {
    let mut min_lat = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut min_lon = f64::MAX;
    let mut max_lon = f64::MIN;
    let mut any = false;

    for (fi, file) in files.iter().enumerate() {
        let Some(file_vis) = visibility.files.get(fi) else {
            continue;
        };
        if !file_vis.enabled {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            let Some(trip_vis) = file_vis.tracks.get(ti) else {
                continue;
            };
            if !trip_vis.enabled {
                continue;
            }
            for point in &track.points {
                let lat = point.tpv.lat().as_degrees();
                let lon = point.tpv.lon().as_degrees();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
            for marker in &track.custom_markers {
                let lat = marker.lat.as_degrees();
                let lon = marker.lon.as_degrees();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
        }
    }

    if any {
        Some((min_lat, max_lat, min_lon, max_lon))
    } else {
        None
    }
}

/// Bounding box (min_lat, max_lat, min_lon, max_lon) over all GPS points and
/// custom markers in every loaded file. Returns `None` if there is no data.
fn compute_bounding_box(files: &[LoadedFile]) -> Option<(f64, f64, f64, f64)> {
    let mut min_lat = f64::MAX;
    let mut max_lat = f64::MIN;
    let mut min_lon = f64::MAX;
    let mut max_lon = f64::MIN;
    let mut any = false;

    for file in files {
        for track in &file.tracks {
            for point in &track.points {
                let lat = point.tpv.lat().as_degrees();
                let lon = point.tpv.lon().as_degrees();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
            for marker in &track.custom_markers {
                let lat = marker.lat.as_degrees();
                let lon = marker.lon.as_degrees();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
        }
    }

    if any {
        Some((min_lat, max_lat, min_lon, max_lon))
    } else {
        None
    }
}

/// Compute the geographic bounding box of the given map viewport rect.
///
/// Uses the walkers `Projector` to unproject the four corners of `map_rect`
/// into geographic positions and returns their bounding envelope.
fn compute_viewport_bounds(map_memory: &MapMemory, map_rect: egui::Rect) -> GeoBounds {
    // `my_position` is only used as a fallback when the map is in GPS-follow mode;
    // since we always call `center_at()` explicitly, `detached()` provides the
    // actual center.  Fall back to (0, 0) if `detached()` is unset.
    let center = map_memory
        .detached()
        .unwrap_or_else(|| walkers::lat_lon(0.0, 0.0));
    let projector = walkers::Projector::new(map_rect, map_memory, center);

    let corners = [
        map_rect.left_top(),
        map_rect.right_top(),
        map_rect.left_bottom(),
        map_rect.right_bottom(),
    ];

    let mut lat_min = f64::INFINITY;
    let mut lat_max = f64::NEG_INFINITY;
    let mut lon_min = f64::INFINITY;
    let mut lon_max = f64::NEG_INFINITY;

    for corner in corners {
        let pos = projector.unproject(corner.to_vec2());
        let lat = pos.y();
        let lon = pos.x();
        lat_min = lat_min.min(lat);
        lat_max = lat_max.max(lat);
        lon_min = lon_min.min(lon);
        lon_max = lon_max.max(lon);
    }

    GeoBounds {
        lat_min,
        lat_max,
        lon_min,
        lon_max,
    }
}

/// Returns `true` when a spatial point should participate in hover and click detection.
///
/// The renderers already suppress invisible elements from being drawn; this function
/// ensures the hit-test layer applies the same rules so that hidden tracks cannot be
/// accidentally hovered or clicked.
fn is_spatial_point_visible(sp: &SpatialPoint, visibility: &TrackDataVisibility) -> bool {
    let Some(file_vis) = sp.file_index.get(&visibility.files) else {
        return false;
    };
    if !file_vis.enabled {
        return false;
    }
    let Some(trip_vis) = sp.track_index.get(&file_vis.tracks) else {
        return false;
    };
    if !trip_vis.enabled {
        return false;
    }
    match sp.category {
        DataCategory::Tpv => trip_vis.tpv_visible,
        DataCategory::CustomMarker => trip_vis.custom_markers_visible,
        DataCategory::GeneratedMarker => trip_vis.generated_markers_visible,
        DataCategory::EventMarker => trip_vis.event_markers_visible,
        DataCategory::Track | DataCategory::SatelliteReport => false,
    }
}

/// Center the map and set the zoom so the given bounding box fills ~80 % of the
/// viewport. Respects walkers' valid zoom range [1, 18].
fn zoom_to_fit(
    map_memory: &mut MapMemory,
    viewport: egui::Rect,
    (min_lat, max_lat, min_lon, max_lon): (f64, f64, f64, f64),
) {
    let center_lat = (min_lat + max_lat) / 2.0;
    let center_lon = (min_lon + max_lon) / 2.0;
    map_memory.center_at(walkers::lat_lon(center_lat, center_lon));

    let lat_range = (max_lat - min_lat).max(0.001);
    let lon_range = (max_lon - min_lon).max(0.001);
    let vw = viewport.width() as f64;
    let vh = viewport.height() as f64;

    // At zoom z the world is 256·2^z pixels wide (equatorial Mercator).
    // Fill 80 % of the viewport with the bounding box:
    //   lon_range · (256·2^z / 360) = vw · 0.8
    //   → z = log2(vw · 0.8 · 360 / (256 · lon_range))
    let z_lon = (vw * 0.8 * 360.0 / (256.0 * lon_range)).log2();
    let z_lat = (vh * 0.8 * 360.0 / (256.0 * lat_range)).log2();
    let zoom = z_lon.min(z_lat).clamp(1.0, 18.0);
    // zoom is already clamped to [1, 18], so set_zoom can only fail if the
    // walkers library's valid range narrows further - ignore silently.
    let _ignored = map_memory.set_zoom(zoom);
}

#[cfg(test)]
mod tests {
    use super::*;
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
            },
            points,
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
        };
        LoadedFile {
            metadata: FileMetadata {
                filename: format!("test_{n}.gtd"),
                total_distance_km: Length::new::<kilometer>(1.0),
                total_duration: chrono::Duration::seconds(n as i64),
                time_range: TimeRange::new(now, now + chrono::Duration::seconds(n as i64)),
            },
            identity: format!("auto:test_{n}.gtd"),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(std::path::PathBuf::from(format!(
                "test_{n}.gtd"
            ))),
            load_warnings: vec![],
            db_ref: None,
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
        let vis = vis_all_visible();
        assert!(is_spatial_point_visible(&sp, &vis));
    }

    /// Regression test: hiding the file must prevent hover on all its points.
    #[test]
    fn hidden_file_blocks_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let mut vis = vis_all_visible();
        vis.files[0].enabled = false;
        assert!(!is_spatial_point_visible(&sp, &vis));
    }

    /// Regression test: hiding the track must prevent hover even when the file is visible.
    #[test]
    fn hidden_track_blocks_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let mut vis = vis_all_visible();
        vis.files[0].tracks[0].enabled = false;
        assert!(!is_spatial_point_visible(&sp, &vis));
    }

    /// Regression test: turning off the TPV layer must prevent hover on TPV points.
    #[test]
    fn hidden_tpv_layer_blocks_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let mut vis = vis_all_visible();
        vis.files[0].tracks[0].tpv_visible = false;
        assert!(!is_spatial_point_visible(&sp, &vis));
    }

    /// The hover must skip the hidden nearest point and return a visible one instead.
    #[test]
    fn hover_skips_hidden_nearest_and_finds_visible() {
        // Two overlapping SpatialPoints in the same Mercator position.
        // Track 0 is hidden; track 1 is visible.
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
        let vis = TrackDataVisibility {
            files: vec![FileVisibility {
                enabled: true,
                tracks: vec![
                    TrackVisibility {
                        enabled: false,
                        ..TrackVisibility::all_visible()
                    },
                    TrackVisibility::all_visible(),
                ],
            }],
        };
        let found = tree
            .nearest_neighbor_iter([0.5_f64, 0.5_f64])
            .take_while(|sp| sp.distance_2(&[0.5, 0.5]) <= f64::MAX)
            .find(|sp| is_spatial_point_visible(sp, &vis));
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
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::test_harness::TestHarness;
    use gt_types::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};
    use gt_ui_types::DataPointRef;

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

    fn install_phosphor(ui: &egui::Ui) {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        ui.ctx().set_fonts(fonts);
    }

    /// Builds a single `LoadedFile` that has one TPV point, one event marker,
    /// and one custom marker, all at index 0.  Used by snapshot tests so
    /// `candidate_label` produces real human-readable text.
    fn make_snapshot_file() -> gt_types::LoadedFile {
        use gt_types::{
            CustomMarker, EventMarker, FileMetadata, Latitude, LoadedFile, LoadedTrack, Longitude,
            MarkerIcon, TimeRange, TrackMetadata, merc_bounds_for_rect,
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

        let bb = gt_types::Rect::new(
            gt_types::Coord { x: 12.55, y: 55.67 },
            gt_types::Coord { x: 12.59, y: 55.69 },
        );
        let n = points.len();
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
                satellite_report_count: 0,
                custom_marker_count: 1,
                generated_marker_count: 0,
                event_marker_count: 1,
            },
            points,
            custom_markers: vec![custom_marker],
            generated_markers: vec![],
            event_markers: vec![event_marker],
        };

        LoadedFile {
            metadata: FileMetadata {
                filename: "snapshot_test.gtd".to_string(),
                total_distance_km: Length::new::<kilometer>(5.0),
                total_duration: chrono::Duration::seconds(n as i64),
                time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
            },
            identity: "auto:snapshot_test.gtd".to_string(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(std::path::PathBuf::from("snapshot_test.gtd")),
            load_warnings: vec![],
            db_ref: None,
        }
    }

    /// Snapshot: the stacked multi-hover label popup that appears when multiple
    /// item types are simultaneously within cursor radius (item 15). Calls the
    /// real production function so the test stays in sync with the code.
    #[test]
    fn snap_multi_hover_stacked_label() {
        let files = vec![make_snapshot_file()];
        let candidates = [Some(tpv_ref()), Some(event_ref()), Some(custom_ref()), None];

        let mut harness = TestHarness::new_wgpu(egui::vec2(280.0, 110.0), move |ui| {
            install_phosphor(ui);
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                draw_multi_hover_label_contents(ui, &candidates, &files);
            });
        });

        harness.run();
        harness.snapshot("multi_hover_stacked_label");
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

        let mut harness = TestHarness::new_wgpu(egui::vec2(300.0, 90.0), move |ui| {
            install_phosphor(ui);
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(200.0);
                for candidate in candidates.iter().flatten().copied() {
                    draw_disambig_row(ui, candidate, &files, sticky == Some(candidate));
                }
            });
        });

        harness.run();
        harness.snapshot("disambig_popup_big_icons");
    }
}
