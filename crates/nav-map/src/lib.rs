pub mod generated_marker_renderer;
pub mod marker_renderer;
pub mod tpv_renderer;
pub mod track_renderer;

use egui::Context;
use nav_types::{
    DataPointRef, GlobalFilter, HighlightScope, LoadedFile, MapHighlight, TripDataVisibility,
};
use std::cell::Cell;
use std::rc::Rc;
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

        let hover_ref: Rc<Cell<Option<(DataPointRef, f32)>>> = Rc::new(Cell::new(None));

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
            .with_plugin(TrackRenderer::new(files, visibility, highlight, filter))
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

        highlight.hover = if map_response.hovered() {
            hover_ref
                .get()
                .map(|(point_ref, _)| HighlightScope::Point(point_ref))
        } else {
            None
        };
    }
}
