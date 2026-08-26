//! Base tiles read from a directory of captured PNGs, laid out the way a tile
//! server addresses them: `{zoom}/{x}/{y}.png`.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use egui::Context;
use rustc_hash::FxHashMap;
use walkers::sources::{Attribution, TileSource as _};
use walkers::{Style, Tile, TileId, TilePiece, Tiles};

use crate::mapbox_tiles;
use crate::test_tiles::{FULL_TILE_UV, TILE_SIZE_PX};

/// A tile the fixture directory served no readable PNG for. Ordered by zoom,
/// then x, then y, so a set of them reads in a stable order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MissingTile {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
}

impl From<TileId> for MissingTile {
    fn from(tile_id: TileId) -> Self {
        Self {
            zoom: tile_id.zoom,
            x: tile_id.x,
            y: tile_id.y,
        }
    }
}

pub struct FixtureTiles {
    directory: PathBuf,
    egui_ctx: Context,
    decoded_tiles: FxHashMap<TileId, Option<Tile>>,
    missing_tiles: BTreeSet<MissingTile>,
}

impl FixtureTiles {
    pub fn new(directory: PathBuf, egui_ctx: Context) -> Self {
        Self {
            directory,
            egui_ctx,
            decoded_tiles: FxHashMap::default(),
            missing_tiles: BTreeSet::new(),
        }
    }

    /// Every tile the directory could not serve, which is every tile the base
    /// layer left blank.
    pub fn missing_tiles(&self) -> &BTreeSet<MissingTile> {
        &self.missing_tiles
    }

    fn read_and_decode_tile(&self, tile_id: TileId) -> Option<Tile> {
        let path = self
            .directory
            .join(tile_id.zoom.to_string())
            .join(tile_id.x.to_string())
            .join(format!("{}.png", tile_id.y));
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                log::debug!("no fixture tile at {path:?}: {err}");
                return None;
            }
        };
        match Tile::new(&bytes, &Style, tile_id.zoom, &self.egui_ctx) {
            Ok(tile) => Some(tile),
            Err(err) => {
                log::debug!("the fixture tile at {path:?} did not decode: {err}");
                None
            }
        }
    }
}

impl Tiles for FixtureTiles {
    fn at(&mut self, tile_id: TileId) -> Option<TilePiece> {
        if let Some(cached) = self.decoded_tiles.get(&tile_id) {
            return cached
                .clone()
                .map(|tile| TilePiece::new(tile, FULL_TILE_UV));
        }
        let tile = self.read_and_decode_tile(tile_id);
        if tile.is_none() {
            self.missing_tiles.insert(MissingTile::from(tile_id));
        }
        self.decoded_tiles.insert(tile_id, tile.clone());
        tile.map(|tile| TilePiece::new(tile, FULL_TILE_UV))
    }

    fn attribution(&self) -> Attribution {
        // The token shapes the tile URL alone: this call reads the
        // attribution text and nothing else.
        mapbox_tiles::satellite_source(String::new()).attribution()
    }

    fn tile_size(&self) -> u32 {
        TILE_SIZE_PX as u32
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::Path;

    use super::*;

    const TILE_ID: TileId = TileId {
        x: 4,
        y: 5,
        zoom: 3,
    };

    fn write_tile_file(directory: &Path, tile_id: TileId, bytes: &[u8]) {
        let directory = directory
            .join(tile_id.zoom.to_string())
            .join(tile_id.x.to_string());
        fs::create_dir_all(&directory).expect("create the fixture directory");
        fs::write(directory.join(format!("{}.png", tile_id.y)), bytes)
            .expect("write the fixture tile");
    }

    fn captured_tile_png() -> Vec<u8> {
        let size_px = u32::try_from(TILE_SIZE_PX).expect("the tile size fits an image dimension");
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(size_px, size_px, image::Rgba([9, 99, 199, 255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("encode the captured tile");
        bytes
    }

    /// A tile the directory holds nothing for and one it holds something
    /// undecodable for both leave the base layer blank and both are recorded.
    #[rstest::rstest]
    #[case::no_file(None)]
    #[case::undecodable_file(Some(b"not a png".as_slice()))]
    fn a_tile_the_directory_cannot_serve_stays_blank_and_is_recorded(
        #[case] written: Option<&[u8]>,
    ) {
        let directory = tempfile::tempdir().expect("temp dir");
        if let Some(bytes) = written {
            write_tile_file(directory.path(), TILE_ID, bytes);
        }
        let mut tiles = FixtureTiles::new(directory.path().to_owned(), Context::default());

        assert!(tiles.at(TILE_ID).is_none());
        assert_eq!(
            tiles.missing_tiles(),
            &BTreeSet::from([MissingTile::from(TILE_ID)])
        );
    }

    #[test]
    fn a_captured_tile_is_served_whole() {
        let directory = tempfile::tempdir().expect("temp dir");
        write_tile_file(directory.path(), TILE_ID, &captured_tile_png());
        let mut tiles = FixtureTiles::new(directory.path().to_owned(), Context::default());

        let piece = tiles.at(TILE_ID).expect("the captured tile is served");
        let Tile::Raster(texture) = piece.tile;
        assert_eq!(texture.size(), [TILE_SIZE_PX; 2]);
        assert_eq!(piece.uv, FULL_TILE_UV);
        assert!(tiles.missing_tiles().is_empty());
    }
}
