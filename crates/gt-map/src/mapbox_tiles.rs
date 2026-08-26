//! The satellite layer's tile source, shared by the map's tile fetcher and the
//! settings window's token test: both build their requests from it.

use walkers::TileId;
use walkers::sources::{Mapbox, MapboxStyle, TileSource as _};

use crate::MAPBOX_MIN_SAFE_ZOOM;

/// The tile a token test requests: the top-left one at the lowest zoom the
/// satellite layer draws.
const TOKEN_TEST_TILE: TileId = TileId {
    x: 0,
    y: 0,
    zoom: MAPBOX_MIN_SAFE_ZOOM,
};

/// The style slug the satellite layer requests, which a capture of its tiles
/// records in its manifest.
pub const SATELLITE_STYLE: &str = "satellite-v9";

/// The variables an access token is taken from, in the order they are read.
pub const TOKEN_ENVS: [&str; 2] = ["MAPBOX_TOKEN", "MAPBOX_ACCESS_TOKEN"];

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

#[cfg(test)]
mod tests {
    use super::*;

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
