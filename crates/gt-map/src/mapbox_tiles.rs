//! The satellite layer's tile source, shared by the map's tile fetcher and the
//! settings window's token test: both build their requests from it.

use walkers::TileId;
use walkers::sources::{Mapbox, MapboxStyle, TileSource as _};

pub(crate) fn satellite_source(access_token: String) -> Mapbox {
    Mapbox {
        style: MapboxStyle::Satellite,
        high_resolution: false,
        access_token,
    }
}

pub fn satellite_tile_url(access_token: &str, tile_id: TileId) -> String {
    satellite_source(access_token.to_owned()).tile_url(tile_id)
}

/// The edge of a tile the satellite source serves, which a capture of those
/// tiles records in its manifest and is read back at.
pub fn satellite_tile_size_px() -> u32 {
    satellite_source(String::new()).tile_size()
}

pub fn token_test_tile_url(access_token: &str) -> String {
    satellite_tile_url(access_token, TOKEN_TEST_TILE)
}

const TOKEN_TEST_TILE: TileId = TileId {
    x: 0,
    y: 0,
    zoom: 2,
};

/// The style slug the satellite layer requests, which a capture of its tiles
/// records in its manifest.
pub const SATELLITE_STYLE: &str = "satellite-v9";

/// The variables an access token is taken from, in the order they are read.
pub const TOKEN_ENVS: [&str; 2] = ["MAPBOX_TOKEN", "MAPBOX_ACCESS_TOKEN"];

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::rc::Rc;

    use egui::epaint::Primitive;
    use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
    use rstest::rstest;
    use walkers::sources::Attribution;
    use walkers::{Tile, TilePiece, Tiles};

    use super::*;
    use crate::test_tiles::{CapturedTileId, FULL_TILE_UV};
    use crate::test_util::MapScene;
    use crate::{MapLayer, TileAccess};

    #[rstest]
    #[case::minimum(0.0)]
    #[case::fractional(0.25)]
    #[case::rounding_boundary(0.5)]
    #[case::below_one_and_a_half(1.49)]
    fn satellite_raster_tiles_are_requested_and_drawn_at_low_zoom(#[case] zoom: f64) {
        let mut rendered = MapScene::of(Vec::new())
            .tiles(TileAccess::Offline)
            .centred_on((0.0, 0.0))
            .zoomed_to(zoom)
            .render();
        let texture = rendered.harness.inner.ctx.load_texture(
            "satellite raster fixture",
            ColorImage::filled([satellite_tile_size_px() as usize; 2], Color32::WHITE),
            TextureOptions::NEAREST,
        );
        assert_eq!(texture.size(), [512; 2]);
        let texture_id = texture.id();
        let requested = Rc::new(RefCell::new(BTreeSet::new()));
        let map = rendered.map_mut().expect("the rendered map");
        map.mapbox_tiles = Some(Box::new(RecordingSatelliteTiles {
            texture,
            requested: Rc::clone(&requested),
        }));
        map.set_layer(MapLayer::Satellite);
        rendered.render_one_more_frame();

        assert_eq!(
            rendered.map().expect("the rendered map").layer(),
            MapLayer::Satellite
        );
        assert_eq!(
            *requested.borrow(),
            BTreeSet::from([CapturedTileId {
                zoom: 0,
                x: 0,
                y: 0
            }]),
        );
        let output = rendered.harness.inner.output();
        let primitives = rendered
            .harness
            .inner
            .ctx
            .tessellate(output.shapes.clone(), output.pixels_per_point);
        let mesh = primitives
            .iter()
            .find_map(|primitive| match &primitive.primitive {
                Primitive::Mesh(mesh) if mesh.texture_id == texture_id => Some(mesh),
                Primitive::Mesh(_) | Primitive::Callback(_) => None,
            })
            .expect("the satellite raster texture is drawn");
        assert!(!mesh.indices.is_empty());
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| vertex.pos.is_finite() && vertex.uv.is_finite())
        );
        let bounds = mesh.calc_bounds();
        let expected_side = 256.0 * zoom.exp2() as f32;
        assert!((bounds.width() - expected_side).abs() < 0.01);
        assert!((bounds.height() - expected_side).abs() < 0.01);

        requested.borrow_mut().clear();
        rendered
            .map_mut()
            .expect("the rendered map")
            .set_layer(MapLayer::OpenStreetMap);
        rendered.render_one_more_frame();
        assert!(requested.borrow().is_empty());
    }

    #[test]
    fn an_unavailable_satellite_source_preserves_offline_layer_selection() {
        let mut rendered = MapScene::of(Vec::new())
            .tiles(TileAccess::Offline)
            .zoomed_to(0.0)
            .render();
        rendered
            .map_mut()
            .expect("the rendered map")
            .set_layer(MapLayer::Satellite);
        rendered.render_one_more_frame();
        let map = rendered.map().expect("the rendered map");
        assert_eq!(map.layer(), MapLayer::Satellite);
        assert!(map.mapbox_tiles.is_none());
        assert!(map.osm_tiles.is_none());
        assert!(map.viewport_geo_bounds().is_some());
    }

    struct RecordingSatelliteTiles {
        texture: TextureHandle,
        requested: Rc<RefCell<BTreeSet<CapturedTileId>>>,
    }

    impl Tiles for RecordingSatelliteTiles {
        fn at(&mut self, tile: TileId) -> Option<TilePiece> {
            self.requested
                .borrow_mut()
                .insert(CapturedTileId::from(tile));
            Some(TilePiece::new(
                Tile::Raster(self.texture.clone()),
                FULL_TILE_UV,
            ))
        }

        fn attribution(&self) -> Attribution {
            satellite_source(String::new()).attribution()
        }

        fn tile_size(&self) -> u32 {
            satellite_tile_size_px()
        }
    }

    #[test]
    fn the_token_test_url_requests_a_satellite_tile_with_the_token() {
        let url = token_test_tile_url("tok");

        assert_eq!(
            url,
            "https://api.mapbox.com/styles/v1/mapbox/satellite-v9/tiles/512/2/0/0?access_token=tok"
        );
        assert!(url.contains(SATELLITE_STYLE));
        assert!(url.contains(&format!("/{}/", satellite_tile_size_px())));
    }
}
