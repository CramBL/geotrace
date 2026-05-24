pub mod generated_marker_renderer;
pub mod marker_renderer;
pub mod tpv_renderer;
pub mod track_renderer;

use egui::Context;

/// Convert pre-computed normalized Mercator coordinates to a screen position.
///
/// `anchor` is the screen-space `Vec2` returned by
/// `projector.project(walkers::lat_lon(0.0, 0.0))` — call this once per
/// plugin run and reuse it for every point.  `total_px` is `2^zoom × 256`,
/// computed from `2_f64.powf(map_memory.zoom()) * 256.0`.
///
/// The math: `screen = anchor + (merc - 0.5) * total_px`, which is the same
/// result as `projector.project(pos)` but replaces trig with two multiplies
/// and two adds per point.
#[inline]
pub(crate) fn merc_to_screen(
    anchor: egui::Vec2,
    total_px: f64,
    merc_x: f64,
    merc_y: f64,
) -> egui::Pos2 {
    egui::pos2(
        anchor.x + ((merc_x - 0.5) * total_px) as f32,
        anchor.y + ((merc_y - 0.5) * total_px) as f32,
    )
}

// URI constants used by the marker renderer and the startup registration call.
pub(crate) const ICON_URI_LIGHTNING: &str = "bytes://nav-map/icons/lightning.svg";
pub(crate) const ICON_URI_WARNING: &str = "bytes://nav-map/icons/warning.svg";
pub(crate) const ICON_URI_ERROR: &str = "bytes://nav-map/icons/error.svg";
pub(crate) const ICON_URI_LOG_PIN: &str = "bytes://nav-map/icons/log_pin.svg";
pub(crate) const ICON_URI_PIN: &str = "bytes://nav-map/icons/pin.svg";
pub(crate) const ICON_URI_CROSS: &str = "bytes://nav-map/icons/cross.svg";
pub(crate) const ICON_URI_CIRCLE_MARKER: &str = "bytes://nav-map/icons/circle_marker.svg";
pub(crate) const ICON_URI_CHECK: &str = "bytes://nav-map/icons/check.svg";

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
}
use nav_types::{
    DataPointRef, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, TripDataVisibility,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;
use uom::si::angle::degree;
use walkers::sources::{Mapbox, MapboxStyle, OpenStreetMap};
use walkers::{HttpTiles, Map, MapMemory};

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

pub struct NavMap {
    egui_ctx: Context,
    osm_tiles: HttpTiles,
    mapbox_tiles: Option<HttpTiles>,
    mapbox_token: String,
    layer: MapLayer,
    map_memory: MapMemory,
    hover_cell: Rc<Cell<Option<(DataPointRef, f32)>>>,
    /// Screen position where the last sticky click happened; used as the
    /// default position for the sticky info window.
    sticky_pos: egui::Pos2,
    /// How many files were loaded last frame — used to detect new loads.
    last_file_count: usize,
    /// Start time of the load-highlight blink animation (None = not animating).
    blink_start: Option<Instant>,
    /// Index of the first newly loaded file; files[new_file_boundary..] are new.
    new_file_boundary: usize,
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
            hover_cell: Rc::new(Cell::new(None)),
            sticky_pos: egui::pos2(100.0, 100.0),
            last_file_count: 0,
            blink_start: None,
            new_file_boundary: 0,
        }
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

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        files: &[LoadedFile],
        visibility: &TripDataVisibility,
        highlight: &mut MapHighlight,
        filter: &GlobalFilter,
        center_request: Option<(f64, f64)>,
    ) {
        if let Some((lat, lon)) = center_request {
            self.map_memory.center_at(walkers::lat_lon(lat, lon));
        }

        // Detect newly loaded files → zoom to fit all data + start blink animation.
        if files.len() > self.last_file_count {
            self.new_file_boundary = self.last_file_count;
            self.last_file_count = files.len();
            self.blink_start = Some(Instant::now());
            if let Some(bbox) = compute_bounding_box(files) {
                zoom_to_fit(&mut self.map_memory, ui.max_rect(), bbox);
            }
        }

        // Compute the current blink intensity (0 = off, 1 = fully lit).
        let blink_alpha = match self.blink_start {
            Some(start) => {
                let elapsed = start.elapsed().as_secs_f32();
                if elapsed >= 3.0 {
                    self.blink_start = None;
                    0.0_f32
                } else {
                    // 2 Hz pulsing that fades to zero over 3 s.
                    let fade = 1.0 - (elapsed / 3.0);
                    (std::f32::consts::TAU * elapsed * 2.0).sin().abs() * fade
                }
            }
            None => 0.0_f32,
        };
        if self.blink_start.is_some() {
            ui.ctx().request_repaint();
        }

        self.hover_cell.set(None);
        let hover_ref = Rc::clone(&self.hover_cell);

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
                Rc::clone(&hover_ref),
            ))
            .with_plugin(MarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                Rc::clone(&hover_ref),
            ))
            .with_plugin(GeneratedMarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                Rc::clone(&hover_ref),
            ));

        let map_response = ui.add(map);

        // Handle click: clicking near a map element makes its info popup sticky;
        // clicking on empty space clears it. Clicking the same element again also clears it.
        if map_response.clicked() {
            if let Some((point_ref, _)) = hover_ref.get() {
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

        // Show a persistent, text-selectable info window for the sticky element.
        if let Some(sticky_ref) = highlight.sticky {
            show_sticky_popup(ui.ctx(), files, sticky_ref, self.sticky_pos);
        }

        highlight.hover = if map_response.hovered() {
            hover_ref
                .get()
                .map(|(point_ref, _)| HighlightScope::Point(point_ref))
        } else {
            None
        };
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
    use nav_types::{DataCategory, GeneratedMarkerKind};
    use uom::si::angle::degree;

    // For TPV points and generated-marker events the window title is the
    // point's datetime; for everything else fall back to a generic label.
    let title: String = if sticky_ref.category == DataCategory::Tpv {
        files
            .get(sticky_ref.file_index)
            .and_then(|f| f.trips.get(sticky_ref.trip_index))
            .and_then(|t| t.points.get(sticky_ref.point_index))
            .map_or_else(
                || "GPS Point".to_string(),
                |p| p.tpv.time().format("%Y-%m-%d %H:%M:%S").to_string(),
            )
    } else if sticky_ref.category == DataCategory::GeneratedMarker {
        files
            .get(sticky_ref.file_index)
            .and_then(|f| f.trips.get(sticky_ref.trip_index))
            .and_then(|t| t.generated_markers.get(sticky_ref.point_index))
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
                    .get(sticky_ref.file_index)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index))
                    .and_then(|t| t.points.get(sticky_ref.point_index))
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
                    .get(sticky_ref.file_index)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index))
                    .and_then(|t| t.custom_markers.get(sticky_ref.point_index))
                {
                    egui::Grid::new("sticky_marker_grid")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Time:");
                            ui.add(
                                egui::Label::new(
                                    marker.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                                )
                                .selectable(true),
                            );
                            ui.end_row();
                            ui.label("Label:");
                            ui.add(egui::Label::new(marker.label.clone()).selectable(true));
                            ui.end_row();
                        });
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Click to deselect").small().weak());
                }
            }
            DataCategory::GeneratedMarker => {
                if let Some(marker) = files
                    .get(sticky_ref.file_index)
                    .and_then(|f| f.trips.get(sticky_ref.trip_index))
                    .and_then(|t| t.generated_markers.get(sticky_ref.point_index))
                {
                    let kind_str = match marker.kind {
                        GeneratedMarkerKind::GpsFixLost => "GPS fix lost",
                        GeneratedMarkerKind::GpsFixRegained => "GPS fix regained",
                    };
                    egui::Grid::new("sticky_gen_grid")
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Time:");
                            ui.add(
                                egui::Label::new(
                                    marker.time.format("%Y-%m-%d %H:%M:%S").to_string(),
                                )
                                .selectable(true),
                            );
                            ui.end_row();
                            ui.label("Event:");
                            ui.add(egui::Label::new(kind_str).selectable(true));
                            ui.end_row();
                            ui.label("Position:");
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
            DataCategory::SatelliteReport | DataCategory::TripTrack => {}
        });
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
