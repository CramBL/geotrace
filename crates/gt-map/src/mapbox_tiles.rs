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

pub(crate) fn satellite_source(access_token: String) -> Mapbox {
    Mapbox {
        style: MapboxStyle::Satellite,
        high_resolution: false,
        access_token,
    }
}

pub fn token_test_tile_url(access_token: &str) -> String {
    satellite_source(access_token.to_owned()).tile_url(TOKEN_TEST_TILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_test_url_requests_a_satellite_tile_with_the_token() {
        assert_eq!(
            token_test_tile_url("tok"),
            "https://api.mapbox.com/styles/v1/mapbox/satellite-v9/tiles/512/2/0/0?access_token=tok"
        );
    }
}
