pub mod event_marker_renderer;
pub mod generated_marker_renderer;
mod hover_labels;
mod icons;
pub mod marker_renderer;
mod polyline;
#[cfg(test)]
mod test_harness;
pub mod tpv_renderer;
mod track_layers;
pub mod track_renderer;
mod transform;
mod viewport;

pub use icons::register_marker_icons;
pub use viewport::GeoBounds;

use egui::Context;

use gt_filter::GlobalFilter;
use gt_types::{DataCategory, FileIdx, LoadedFile, SpatialPoint, TrackRef};
use gt_ui_types::{
    DataPointRef, EventMarkerVisibility, HighlightScope, MapHighlight, TrackDataVisibility,
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
            hover_fade: HoverFadeState::default(),
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

        if zoom_to_visible
            && let Some(bbox) = compute_visible_bounding_box(files, visibility, filter)
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
            if let Some(bbox) = compute_visible_bounding_box(files, visibility, filter) {
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
        let plan = viewport::TrackPlan::compute(files, visibility, filter, self.map_memory.zoom());
        let visible = viewport::collect_visible_points(
            &self.global_tree,
            &plan,
            &transform_estimate,
            map_rect_estimate,
        );

        let is_offline = std::env::var("GEOTRACE_OFFLINE").is_ok();
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
        let map = map
            .with_plugin(
                TrackLayers::builder()
                    .files(files)
                    .plan(&plan)
                    .highlight(highlight)
                    .filter(filter)
                    .tpv_by_track(visible.tpv_by_track)
                    .new_file_boundary(self.new_file_boundary)
                    .blink_alpha(blink_alpha)
                    .hover_fade_alpha(hover_fade_progress)
                    .build(),
            )
            .with_plugin(MarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                visible.custom,
            ))
            .with_plugin(GeneratedMarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                visible.generated,
            ))
            .with_plugin(EventMarkerRenderer::new(
                files,
                visibility,
                highlight,
                filter,
                event_marker_visibility,
                visible.event,
            ));

        let map_response = ui.add(map);

        // Double-click anywhere on the map: zoom out to fit the visible tracks.
        if map_response.double_clicked()
            && let Some(bbox) = compute_visible_bounding_box(files, visibility, filter)
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
                    if !is_spatial_point_visible(sp, files, visibility, filter) {
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
            egui::Area::new(egui::Id::new("map_multi_hover_labels"))
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
fn show_sticky_popup(
    ctx: &egui::Context,
    files: &[LoadedFile],
    sticky_ref: DataPointRef,
    default_pos: egui::Pos2,
) {
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
                    // Cap the window height so satellite tables never overflow the
                    // screen. The ScrollArea only activates past the cap.
                    let max_h = (ui.ctx().viewport_rect().height() * 0.75).min(500.0);
                    egui::ScrollArea::vertical()
                        .max_height(max_h)
                        .show(ui, |ui| {
                            crate::tpv_renderer::show_sticky_tpv_content(ui, point);
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
                    let kind_str = marker.kind.to_string();
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
                            crate::tpv_renderer::show_sticky_tpv_content(ui, point);
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

#[cfg(test)]
mod tests {
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
                ..FileMetadata::default()
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
            recording_meta: None,
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
            &GlobalFilter::default()
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
            &GlobalFilter::default()
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
            &GlobalFilter::default()
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
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
        }
    }

    fn file_with_tracks(tracks: Vec<LoadedTrack>) -> LoadedFile {
        LoadedFile {
            metadata: FileMetadata::default(),
            identity: "auto:test.gtd".to_string(),
            tracks,
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(std::path::PathBuf::from("test.gtd")),
            load_warnings: vec![],
            db_ref: None,
            recording_meta: None,
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
            compute_visible_bounding_box(&files, &vis, &filter).expect("visible data has a bbox");
        assert_eq!(all_visible, (55.0, 56.0, 12.0, 13.0));

        // Hide the north-east track: its corner drops out of the box.
        vis.files[0].tracks[1].enabled = false;
        let only_first =
            compute_visible_bounding_box(&files, &vis, &filter).expect("track 0 still visible");
        assert_eq!(only_first, (55.0, 55.0, 12.0, 12.0));
    }

    /// Regression test: turning off the TPV layer must prevent hover on TPV points.
    #[test]
    fn hidden_tpv_layer_blocks_hover() {
        let sp = tpv_spatial_point(0, 0, 0);
        let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
        let mut vis = vis_all_visible();
        vis.files[0].tracks[0].tpv_visible = false;
        assert!(!is_spatial_point_visible(
            &sp,
            &files,
            &vis,
            &GlobalFilter::default()
        ));
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
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
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
            !is_spatial_point_visible(&tpv_spatial_point(0, 0, 0), &files, &vis, &filter),
            "the pre-window point must not be hoverable"
        );
        assert!(
            is_spatial_point_visible(&tpv_spatial_point(0, 0, 1), &files, &vis, &filter),
            "the in-window point must stay hoverable"
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
        let filter = GlobalFilter::default();
        let found = tree
            .nearest_neighbor_iter([0.5_f64, 0.5_f64])
            .take_while(|sp| sp.distance_2(&[0.5, 0.5]) <= f64::MAX)
            .find(|sp| is_spatial_point_visible(sp, &files, &vis, &filter));
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
        };
        let file = LoadedFile {
            metadata: FileMetadata {
                filename: "test.gtd".to_string(),
                total_distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0),
                total_duration: chrono::Duration::seconds(1),
                time_range: TimeRange::new(now, now + chrono::Duration::seconds(1)),
                ..FileMetadata::default()
            },
            identity: "auto:test.gtd".to_string(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(std::path::PathBuf::from("test.gtd")),
            load_warnings: vec![],
            db_ref: None,
            recording_meta: None,
        };

        let candidate = gt_ui_types::DataPointRef {
            track: gt_types::TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::GeneratedMarker,
            point_index: PointIdx::new(0),
        };
        let expected = crate::generated_marker_renderer::generated_marker_header(
            GeneratedMarkerKind::GnssFixRegained {
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
                generated_marker_count: 1,
                event_marker_count: 1,
                ..TrackMetadata::default()
            },
            points,
            lod: gt_types::TrackLod::default(),
            custom_markers: vec![custom_marker],
            generated_markers: vec![generated_marker],
            event_markers: vec![event_marker],
        };

        LoadedFile {
            metadata: FileMetadata {
                filename: "snapshot_test.gtd".to_string(),
                total_distance_km: Length::new::<kilometer>(5.0),
                total_duration: chrono::Duration::seconds(n as i64),
                time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
                ..FileMetadata::default()
            },
            identity: "auto:snapshot_test.gtd".to_string(),
            tracks: vec![track],
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: vec![],
            source: gt_types::FileSource::GtdPath(std::path::PathBuf::from("snapshot_test.gtd")),
            load_warnings: vec![],
            db_ref: None,
            recording_meta: None,
        }
    }

    /// Snapshot: the stacked multi-hover label popup for TPV + event marker +
    /// custom marker simultaneously within cursor radius.  Calls the real
    /// production function so the test stays in sync with the code.
    #[test]
    fn snap_multi_hover_stacked_label() {
        let files = vec![make_snapshot_file()];
        let candidates = [Some(tpv_ref()), Some(event_ref()), Some(custom_ref()), None];

        let mut harness = TestHarness::builder()
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

        let mut harness = TestHarness::builder()
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

        let mut harness = TestHarness::builder()
            .size(egui::vec2(300.0, 90.0))
            .ui(move |ui| {
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
