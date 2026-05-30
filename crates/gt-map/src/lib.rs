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
    merc_x_center: f64,
    merc_y_center: f64,
    total_px: f64,
}

impl MercTransform {
    /// Build the transform for the current frame.
    ///
    /// `clip_center` must be `ui.max_rect().center()` inside a plugin's
    /// `run()` method — walkers sets the child UI rect to the map widget rect,
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
        let (merc_x_center, merc_y_center) = normalize_merc(center_ll.x(), center_ll.y());
        Self {
            clip_center_x: clip_center.x as f64,
            clip_center_y: clip_center.y as f64,
            merc_x_center,
            merc_y_center,
            total_px,
        }
    }

    /// Project a pre-computed normalised Mercator coordinate to a screen position.
    #[inline]
    pub(crate) fn to_screen(&self, merc_x: f64, merc_y: f64) -> egui::Pos2 {
        egui::pos2(
            (self.clip_center_x + (merc_x - self.merc_x_center) * self.total_px) as f32,
            (self.clip_center_y + (merc_y - self.merc_y_center) * self.total_px) as f32,
        )
    }

    /// Convert a screen-space x-coordinate to a normalised Mercator x value.
    #[inline]
    pub(crate) fn merc_x_from_screen(&self, screen_x: f32) -> f64 {
        (screen_x as f64 - self.clip_center_x) / self.total_px + self.merc_x_center
    }

    /// Convert a screen-space y-coordinate to a normalised Mercator y value.
    #[inline]
    pub(crate) fn merc_y_from_screen(&self, screen_y: f32) -> f64 {
        (screen_y as f64 - self.clip_center_y) / self.total_px + self.merc_y_center
    }

    /// Pixels per metre at the given latitude.
    ///
    /// Uses the Web Mercator scale factor: the equatorial circumference
    /// (≈ 40 030 km) shrinks by cos(lat) at a given latitude.
    #[inline]
    pub(crate) fn pixels_per_meter(&self, lat_deg: f64) -> f64 {
        // At zoom z the world is 256·2^z pixels wide at the equator.
        // 1 Mercator tile column = Earth circumference / 2^z metres at the equator,
        // scaled by cos(lat) at higher latitudes.
        const EARTH_CIRCUMFERENCE_M: f64 = 40_030_173.0;
        self.total_px / (EARTH_CIRCUMFERENCE_M * lat_deg.to_radians().cos())
    }
}

/// Normalised Web Mercator projection — both outputs in `[0.0, 1.0]`.
///
/// This is identical to walkers' internal `mercator_normalized()` and to
/// `gt_types::mercator::normalize()`. It is duplicated here because
/// `gt_types::mercator` is a crate-private module, inaccessible from `gt_map`.
/// Both copies must be kept in sync.
fn normalize_merc(lon_deg: f64, lat_deg: f64) -> (f64, f64) {
    use std::f64::consts::PI;
    let x = lon_deg.to_radians();
    let y = lat_deg.to_radians().tan().asinh();
    ((1.0 + x / PI) / 2.0, (1.0 - y / PI) / 2.0)
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

/// Register the embedded SVG marker icons with the egui context.
///
/// Call this once at startup (before the first frame) from your `App::new`
/// implementation, **after** [`egui_extras::install_image_loaders`] has been
/// called. The icons are compiled into the binary via `include_bytes!` and
/// cached by egui's texture system after their first rasterisation; subsequent
/// frames pay only a GPU quad draw — no CPU tessellation, no heap allocation.
pub fn register_marker_icons(ctx: &egui::Context) {
    ctx.include_bytes(
        ICON_URI_LIGHTNING,
        include_bytes!("icons/lightning.svg").as_slice(),
    );
    ctx.include_bytes(
        ICON_URI_WARNING,
        include_bytes!("icons/warning.svg").as_slice(),
    );
    ctx.include_bytes(ICON_URI_ERROR, include_bytes!("icons/error.svg").as_slice());
    ctx.include_bytes(
        ICON_URI_LOG_PIN,
        include_bytes!("icons/log_pin.svg").as_slice(),
    );
    ctx.include_bytes(ICON_URI_PIN, include_bytes!("icons/pin.svg").as_slice());
    ctx.include_bytes(ICON_URI_CROSS, include_bytes!("icons/cross.svg").as_slice());
    ctx.include_bytes(
        ICON_URI_CIRCLE_MARKER,
        include_bytes!("icons/circle_marker.svg").as_slice(),
    );
    ctx.include_bytes(ICON_URI_CHECK, include_bytes!("icons/check.svg").as_slice());
    ctx.include_bytes(
        ICON_URI_SATELLITE,
        include_bytes!("icons/satellite.svg").as_slice(),
    );
    ctx.include_bytes(
        ICON_URI_SATELLITE_LOST,
        include_bytes!("icons/satellite_lost.svg").as_slice(),
    );
    ctx.include_bytes(ICON_URI_GEAR, include_bytes!("icons/gear.svg").as_slice());
    ctx.include_bytes(
        ICON_URI_REFRESH,
        include_bytes!("icons/refresh.svg").as_slice(),
    );
    ctx.include_bytes(
        ICON_URI_DOWNLOAD,
        include_bytes!("icons/download.svg").as_slice(),
    );
    ctx.include_bytes(
        ICON_URI_UPLOAD,
        include_bytes!("icons/upload.svg").as_slice(),
    );
    ctx.include_bytes(
        ICON_URI_WRENCH,
        include_bytes!("icons/wrench.svg").as_slice(),
    );
}
use gt_types::{
    DataCategory, DataPointRef, EventMarkerVisibility, GlobalFilter, HighlightScope, LoadedFile,
    MapHighlight, SpatialPoint, TripDataVisibility, build_global_tree,
};
use std::time::Instant;
use uom::si::angle::degree;
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
    /// Hide every trip except the one at `(file_index, trip_index)`.
    ShowOnlyTrip {
        file_index: usize,
        trip_index: usize,
    },
    /// Hide every file except the one at `file_index`.
    ShowOnlyFile { file_index: usize },
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
    /// How many files were loaded last frame — used to detect new loads.
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

    #[expect(
        clippy::too_many_arguments,
        reason = "draw context requires all parameters; a wrapper struct would not add clarity"
    )]
    #[expect(
        clippy::cognitive_complexity,
        reason = "draw orchestrates viewport query, plugin wiring, hover, click, and popup — splitting would obscure the data flow"
    )]
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        files: &[LoadedFile],
        visibility: &TripDataVisibility,
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
            self.last_file_count = files.len();
            self.blink.trigger();
            if let Some(bbox) = compute_bounding_box(files) {
                zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
            }
            self.global_tree = build_global_tree(files);
        }

        let blink_alpha = self.blink.tick();
        if self.blink.is_active() {
            ui.ctx().request_repaint();
        }

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
        let hover_point_ref: Option<DataPointRef> = if map_response.hovered() {
            ui.input(|i| i.pointer.hover_pos()).and_then(|screen_pos| {
                let merc_x = transform.merc_x_from_screen(screen_pos.x);
                let merc_y = transform.merc_y_from_screen(screen_pos.y);
                let total_px = 2_f64.powf(self.map_memory.zoom()) * 256.0;
                let threshold_merc_sq = (20.0_f64 / total_px).powi(2);
                self.global_tree
                    .nearest_neighbor([merc_x, merc_y])
                    .filter(|sp| {
                        let dx = sp.merc_x - merc_x;
                        let dy = sp.merc_y - merc_y;
                        dx * dx + dy * dy <= threshold_merc_sq
                    })
                    .map(|sp| DataPointRef {
                        file_index: sp.file_index,
                        trip_index: sp.trip_index,
                        category: sp.category,
                        point_index: sp.point_index,
                    })
            })
        } else {
            None
        };

        // Layer toggle — floating panel anchored to the bottom-right of the map.

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
        if map_response.clicked() {
            if let Some(point_ref) = hover_point_ref {
                if highlight.sticky == Some(point_ref) {
                    highlight.sticky = None;
                } else {
                    highlight.sticky = Some(point_ref);
                    self.sticky_pos = ui
                        .ctx()
                        .pointer_latest_pos()
                        .unwrap_or(map_response.rect.center());
                }
            } else {
                highlight.sticky = None;
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
                // Right-clicked on empty map space — nothing to show.
                ui.close();
                return;
            };
            let Some(file) = files.get(point_ref.file_index.0) else {
                ui.close();
                return;
            };
            // Header: file name and trip number (trip omitted for single-trip files).
            ui.add(egui::Label::new(
                egui::RichText::new(file.metadata.filename.as_str()).weak(),
            ));
            if file.trips.len() > 1 {
                ui.add(egui::Label::new(
                    egui::RichText::new(format!("Trip {}", point_ref.trip_index.0 + 1)).weak(),
                ));
            }
            ui.separator();
            if ui.button("Only show elements from this trip").clicked() {
                context_action = Some(MapContextAction::ShowOnlyTrip {
                    file_index: point_ref.file_index.0,
                    trip_index: point_ref.trip_index.0,
                });
                ui.close();
            }
            if ui.button("Only show elements from this file").clicked() {
                context_action = Some(MapContextAction::ShowOnlyFile {
                    file_index: point_ref.file_index.0,
                });
                ui.close();
            }
        });

        // Show a persistent, text-selectable info window for the sticky element.
        if let Some(sticky_ref) = highlight.sticky {
            show_sticky_popup(ui.ctx(), files, sticky_ref, self.sticky_pos);
        }

        highlight.hover = hover_point_ref.map(HighlightScope::Point);

        context_action
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
    use uom::si::angle::degree;

    // For TPV points, satellite reports, and generated-marker events the window
    // title is the point's datetime; for everything else fall back to a generic label.
    let title: String = if sticky_ref.category == DataCategory::Tpv {
        files
            .get(sticky_ref.file_index.0)
            .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
            .and_then(|t| t.points.get(sticky_ref.point_index.0))
            .map_or_else(
                || "GPS Point".to_string(),
                |p| p.tpv.time().utc().format("%Y-%m-%d %H:%M:%S").to_string(),
            )
    } else if sticky_ref.category == DataCategory::SatelliteReport {
        files
            .get(sticky_ref.file_index.0)
            .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
            .and_then(|t| t.points.get(sticky_ref.point_index.0))
            .and_then(|p| p.satellites.as_ref())
            .map_or_else(
                || "Satellite Report".to_string(),
                |sats| {
                    sats.best_time().map_or_else(
                        || "Satellite Report".to_string(),
                        |t| t.format("%Y-%m-%d %H:%M:%S").to_string(),
                    )
                },
            )
    } else if sticky_ref.category == DataCategory::GeneratedMarker {
        files
            .get(sticky_ref.file_index.0)
            .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
            .and_then(|t| t.generated_markers.get(sticky_ref.point_index.0))
            .map_or_else(
                || "GPS Event".to_string(),
                |m| m.time.format("%Y-%m-%d %H:%M:%S").to_string(),
            )
    } else {
        "Point Info".to_string()
    };

    egui::Window::new(title)
        .id(egui::Id::new(("sticky_popup", sticky_ref)))
        .default_pos(default_pos)
        .collapsible(false)
        .auto_sized()
        .show(ctx, |ui| match sticky_ref.category {
            DataCategory::Tpv => {
                if let Some(point) = files
                    .get(sticky_ref.file_index.0)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
                    .and_then(|t| t.points.get(sticky_ref.point_index.0))
                {
                    // Cap the window height so satellite tables never overflow
                    // the screen. For small satellite counts the ScrollArea
                    // auto-sizes to content (no scroll bar); it only activates
                    // when content is taller than the cap — e.g. when the UI is
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
                if let Some(marker) = files
                    .get(sticky_ref.file_index.0)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
                    .and_then(|t| t.custom_markers.get(sticky_ref.point_index.0))
                {
                    egui::Grid::new("sticky_marker_grid")
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
                            ui.label("Label");
                            ui.add(egui::Label::new(marker.label.clone()).selectable(true));
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
                }
            }
            DataCategory::GeneratedMarker => {
                if let Some(marker) = files
                    .get(sticky_ref.file_index.0)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
                    .and_then(|t| t.generated_markers.get(sticky_ref.point_index.0))
                {
                    let kind_str = match marker.kind {
                        GeneratedMarkerKind::GpsFixLost => "GPS fix lost",
                        GeneratedMarkerKind::GpsFixRegained => "GPS fix regained",
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
                                    marker.lat.get::<degree>(),
                                    marker.lon.get::<degree>()
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
                if let Some(point) = files
                    .get(sticky_ref.file_index.0)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
                    .and_then(|t| t.points.get(sticky_ref.point_index.0))
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
            DataCategory::TripTrack => {}
            DataCategory::EventMarker => {
                if let Some(marker) = files
                    .get(sticky_ref.file_index.0)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index.0))
                    .and_then(|t| t.event_markers.get(sticky_ref.point_index.0))
                {
                    egui::Grid::new("sticky_event_marker_grid")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Event");
                            ui.add(egui::Label::new(marker.variant_path.clone()).selectable(true));
                            ui.end_row();
                            ui.label("Time");
                            ui.add(
                                egui::Label::new(
                                    marker.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                                )
                                .selectable(true),
                            );
                            ui.end_row();
                            if let Some(ann) = &marker.annotation {
                                ui.label("Note");
                                ui.add(egui::Label::new(ann.clone()).selectable(true));
                                ui.end_row();
                            }
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
                }
            }
        });
}

/// Bounding box over only the currently **visible** trips (those with both their
/// file and trip enabled). Returns `None` if no visible data exists.
fn compute_visible_bounding_box(
    files: &[LoadedFile],
    visibility: &TripDataVisibility,
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
        for (ti, trip) in file.trips.iter().enumerate() {
            let Some(trip_vis) = file_vis.trips.get(ti) else {
                continue;
            };
            if !trip_vis.enabled {
                continue;
            }
            for point in &trip.points {
                let lat = point.tpv.lat().get::<degree>();
                let lon = point.tpv.lon().get::<degree>();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
            for marker in &trip.custom_markers {
                let lat = marker.lat.get::<degree>();
                let lon = marker.lon.get::<degree>();
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
        for trip in &file.trips {
            for point in &trip.points {
                let lat = point.tpv.lat().get::<degree>();
                let lon = point.tpv.lon().get::<degree>();
                min_lat = min_lat.min(lat);
                max_lat = max_lat.max(lat);
                min_lon = min_lon.min(lon);
                max_lon = max_lon.max(lon);
                any = true;
            }
            for marker in &trip.custom_markers {
                let lat = marker.lat.get::<degree>();
                let lon = marker.lon.get::<degree>();
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
    // walkers library's valid range narrows further — ignore silently.
    let _ignored = map_memory.set_zoom(zoom);
}
