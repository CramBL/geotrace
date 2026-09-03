//! Base tile sources that reach no tile server: a grid generated in process
//! and a directory of captured tiles. A snapshot taken over either one
//! shows where the map was framed. A blank base layer shows nothing of it.

pub mod fixture;
pub mod glyph;
pub mod synthetic;

use std::collections::BTreeSet;

use egui::{Context, Rect, pos2};
use walkers::sources::Attribution;
use walkers::{TileId, TilePiece, Tiles};

use crate::TileAccess;
pub use crate::test_tiles::fixture::{
    CapturedTileFormat, FixtureTileId, FixtureTiles, TileFixtureManifest, TileFixtureManifestError,
};
pub use crate::test_tiles::synthetic::SyntheticTiles;

/// A piece covers its texture entirely: both sources serve whole tiles.
const FULL_TILE_UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

pub(crate) enum TestTileSource {
    Synthetic(SyntheticTiles),
    Fixture(FixtureTiles),
}

impl TestTileSource {
    pub(crate) fn for_tile_access(tile_access: &TileAccess, egui_ctx: &Context) -> Option<Self> {
        match tile_access {
            TileAccess::Network | TileAccess::Offline => None,
            TileAccess::Synthetic => Some(Self::Synthetic(SyntheticTiles::new(egui_ctx.clone()))),
            TileAccess::Fixture(directory) => {
                match FixtureTiles::new(directory.clone(), egui_ctx.clone()) {
                    Ok(tiles) => Some(Self::Fixture(tiles)),
                    Err(err) => {
                        log::error!("the map draws no base layer: {err}");
                        None
                    }
                }
            }
        }
    }

    pub(crate) fn missing_tiles(&self) -> Option<&BTreeSet<FixtureTileId>> {
        match self {
            Self::Synthetic(_) => None,
            Self::Fixture(tiles) => Some(tiles.missing_tiles()),
        }
    }

    pub(crate) fn forget_missing_tiles(&mut self) {
        match self {
            Self::Synthetic(_) => {}
            Self::Fixture(tiles) => tiles.forget_missing_tiles(),
        }
    }
}

impl Tiles for TestTileSource {
    fn at(&mut self, tile_id: TileId) -> Option<TilePiece> {
        match self {
            Self::Synthetic(tiles) => tiles.at(tile_id),
            Self::Fixture(tiles) => tiles.at(tile_id),
        }
    }

    fn attribution(&self) -> Attribution {
        match self {
            Self::Synthetic(tiles) => tiles.attribution(),
            Self::Fixture(tiles) => tiles.attribution(),
        }
    }

    fn tile_size(&self) -> u32 {
        match self {
            Self::Synthetic(tiles) => tiles.tile_size(),
            Self::Fixture(tiles) => tiles.tile_size(),
        }
    }
}

#[cfg(test)]
mod tests {
    use gt_types::LoadedFile;
    use gt_ui_types::TrackDataVisibility;

    use super::*;
    use crate::{DrawState, NavMap};

    /// A map built on the synthetic source draws its frame through the same
    /// path the application's tile fetcher takes.
    #[test]
    fn the_map_draws_a_frame_over_the_synthetic_tiles() {
        let files: Vec<LoadedFile> = Vec::new();
        let visibility = TrackDataVisibility::from_loaded(&files);
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(400.0, 400.0))
            .ui_state(
                |ui, map: &mut Option<NavMap>| {
                    let map = map.get_or_insert_with(|| {
                        NavMap::new(ui.ctx().clone(), TileAccess::Synthetic)
                    });
                    let mut state = DrawState::default();
                    map.draw(ui, state.context(&files, &visibility));
                },
                None,
            );

        harness.inner.run_steps(3);

        assert!(
            harness
                .state()
                .as_ref()
                .and_then(NavMap::viewport_geo_bounds)
                .is_some(),
            "the map framed no viewport"
        );
    }
}
