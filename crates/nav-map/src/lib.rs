pub mod marker_renderer;
pub mod tpv_renderer;

use egui::Context;
use nav_types::{CustomMarker, NavPoint};
use walkers::sources::OpenStreetMap;
use walkers::{HttpTiles, Map, MapMemory};

use crate::marker_renderer::MarkerRenderer;
use crate::tpv_renderer::TpvRenderer;

pub struct NavMap {
    points: Vec<NavPoint>,
    markers: Vec<CustomMarker>,
    tiles: HttpTiles,
    map_memory: MapMemory,
}

impl NavMap {
    pub fn new(egui_ctx: Context) -> Self {
        Self {
            points: nav_types::test_data::nav_test_data(),
            markers: nav_types::marker_test_data(),
            tiles: HttpTiles::new(OpenStreetMap, egui_ctx),
            map_memory: MapMemory::default(),
        }
    }

    pub fn add_points(&mut self, mut points: Vec<NavPoint>) {
        self.points.append(&mut points);
    }

    pub fn add_markers(&mut self, mut markers: Vec<CustomMarker>) {
        self.markers.append(&mut markers);
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let map = Map::new(
            Some(&mut self.tiles),
            &mut self.map_memory,
            walkers::lat_lon(55.676, 12.565),
        )
        .with_plugin(TpvRenderer::new(&self.points))
        .with_plugin(MarkerRenderer::new(&self.markers));

        ui.add(map);
    }
}
