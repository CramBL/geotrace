//! Base tiles read from a directory of captured tiles, laid out the way a
//! tile server addresses them: `{zoom}/{x}/{y}.{extension}`, with a
//! `manifest.json` beside them stating what was captured, at which tile size
//! and in which image format.

use std::collections::BTreeSet;
use std::fmt;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs, io};

use egui::Context;
use rustc_hash::FxHashMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use walkers::sources::{Attribution, TileSource as _};
use walkers::{Style, Tile, TileId, TilePiece, Tiles};

use crate::mapbox_tiles;
use crate::test_tiles::FULL_TILE_UV;

/// A tile of the fixture directory. Ordered by zoom, then x, then y, so a set
/// of them reads in a stable order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FixtureTileId {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FixtureTileIdParseError {
    #[error("{0:?} is not three /-separated fields")]
    FieldCount(String),

    #[error("field {field:?} of the tile id {text:?} is not a number: {source}")]
    Field {
        text: String,
        field: String,
        source: ParseIntError,
    },
}

impl FixtureTileId {
    pub fn path_within(self, directory: &Path, format: CapturedTileFormat) -> PathBuf {
        directory
            .join(self.zoom.to_string())
            .join(self.x.to_string())
            .join(format!("{}.{}", self.y, format.extension()))
    }
}

/// The image format a tile server served the capture, which decides both the
/// extension the tiles are written under and how the map decodes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapturedTileFormat {
    Jpeg,
    Png,
}

impl CapturedTileFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    /// The format a `Content-Type` header names, ignoring any parameters
    /// after it. An unlisted type is one nothing here captures.
    pub fn from_content_type(content_type: &str) -> Option<Self> {
        let media_type = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        match media_type.as_str() {
            "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "image/png" => Some(Self::Png),
            _ => None,
        }
    }
}

impl From<TileId> for FixtureTileId {
    fn from(tile_id: TileId) -> Self {
        Self {
            zoom: tile_id.zoom,
            x: tile_id.x,
            y: tile_id.y,
        }
    }
}

impl From<FixtureTileId> for TileId {
    fn from(tile_id: FixtureTileId) -> Self {
        Self {
            zoom: tile_id.zoom,
            x: tile_id.x,
            y: tile_id.y,
        }
    }
}

impl fmt::Display for FixtureTileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { zoom, x, y } = self;
        write!(f, "{zoom}/{x}/{y}")
    }
}

impl FromStr for FixtureTileId {
    type Err = FixtureTileIdParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let field_count = || FixtureTileIdParseError::FieldCount(text.to_owned());
        let mut fields = text.split('/');
        let zoom = fields.next().ok_or_else(field_count)?;
        let x = fields.next().ok_or_else(field_count)?;
        let y = fields.next().ok_or_else(field_count)?;
        if fields.next().is_some() {
            return Err(field_count());
        }
        let field = |field: &str, source: ParseIntError| FixtureTileIdParseError::Field {
            text: text.to_owned(),
            field: field.to_owned(),
            source,
        };
        Ok(Self {
            zoom: zoom.parse().map_err(|source| field(zoom, source))?,
            x: x.parse().map_err(|source| field(x, source))?,
            y: y.parse().map_err(|source| field(y, source))?,
        })
    }
}

impl Serialize for FixtureTileId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FixtureTileId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

/// What a fixture directory records about its own capture. The access token
/// is never part of it: the host is written down, the URL is not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileFixtureManifest {
    /// The edge of every captured tile, which the map scales its base layer
    /// by. Mapbox serves 512, the slippy default is 256.
    pub tile_size_px: u32,
    /// Every tile is stored exactly as the host served it, in this format.
    pub tile_format: CapturedTileFormat,
    pub style: String,
    pub host: String,
    pub captured_at: String,
    pub tiles: BTreeSet<FixtureTileId>,
}

#[derive(Debug, thiserror::Error)]
pub enum TileFixtureManifestError {
    #[error("{path:?}: {source}")]
    File { path: PathBuf, source: io::Error },

    #[error("{path:?} is not a tile fixture manifest: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl TileFixtureManifest {
    pub const FILE_NAME: &'static str = "manifest.json";

    pub fn read(directory: &Path) -> Result<Self, TileFixtureManifestError> {
        let path = directory.join(Self::FILE_NAME);
        let text = fs::read_to_string(&path).map_err(|source| TileFixtureManifestError::File {
            path: path.clone(),
            source,
        })?;
        serde_json::from_str(&text)
            .map_err(|source| TileFixtureManifestError::Json { path, source })
    }

    pub fn write(&self, directory: &Path) -> Result<(), TileFixtureManifestError> {
        let path = directory.join(Self::FILE_NAME);
        let text = serde_json::to_string_pretty(self).map_err(|source| {
            TileFixtureManifestError::Json {
                path: path.clone(),
                source,
            }
        })?;
        fs::write(&path, format!("{text}\n"))
            .map_err(|source| TileFixtureManifestError::File { path, source })
    }
}

pub struct FixtureTiles {
    directory: PathBuf,
    tile_size_px: u32,
    tile_format: CapturedTileFormat,
    egui_ctx: Context,
    decoded_tiles: FxHashMap<TileId, Option<Tile>>,
    missing_tiles: BTreeSet<FixtureTileId>,
}

impl FixtureTiles {
    /// Reads the manifest the capture wrote, which states the size and the
    /// format the tiles were served in. A directory without one holds no
    /// usable capture.
    pub fn new(directory: PathBuf, egui_ctx: Context) -> Result<Self, TileFixtureManifestError> {
        let manifest = TileFixtureManifest::read(&directory)?;
        Ok(Self {
            directory,
            tile_size_px: manifest.tile_size_px,
            tile_format: manifest.tile_format,
            egui_ctx,
            decoded_tiles: FxHashMap::default(),
            missing_tiles: BTreeSet::new(),
        })
    }

    /// Every tile asked for since the last [`FixtureTiles::forget_missing_tiles`]
    /// that the directory could not serve, which is every tile the base layer
    /// left blank.
    pub fn missing_tiles(&self) -> &BTreeSet<FixtureTileId> {
        &self.missing_tiles
    }

    pub fn forget_missing_tiles(&mut self) {
        self.missing_tiles.clear();
    }

    fn read_and_decode_tile(&self, tile_id: TileId) -> Option<Tile> {
        let path = FixtureTileId::from(tile_id).path_within(&self.directory, self.tile_format);
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
        let tile = match self.decoded_tiles.get(&tile_id) {
            Some(cached) => cached.clone(),
            None => {
                let decoded = self.read_and_decode_tile(tile_id);
                self.decoded_tiles.insert(tile_id, decoded.clone());
                decoded
            }
        };
        if tile.is_none() {
            self.missing_tiles.insert(FixtureTileId::from(tile_id));
        }
        tile.map(|tile| TilePiece::new(tile, FULL_TILE_UV))
    }

    fn attribution(&self) -> Attribution {
        // The token shapes the tile URL alone: this call reads the
        // attribution text and nothing else.
        mapbox_tiles::satellite_source(String::new()).attribution()
    }

    fn tile_size(&self) -> u32 {
        self.tile_size_px
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const TILE_ID: TileId = TileId {
        x: 4,
        y: 5,
        zoom: 3,
    };

    fn manifest(tile_size_px: u32, tile_format: CapturedTileFormat) -> TileFixtureManifest {
        TileFixtureManifest {
            tile_size_px,
            tile_format,
            style: "satellite-v9".to_owned(),
            host: "api.mapbox.com".to_owned(),
            captured_at: "2026-08-26T10:00:00+00:00".to_owned(),
            tiles: BTreeSet::from([FixtureTileId::from(TILE_ID)]),
        }
    }

    fn write_tile_file(
        directory: &Path,
        tile_id: TileId,
        format: CapturedTileFormat,
        bytes: &[u8],
    ) {
        let path = FixtureTileId::from(tile_id).path_within(directory, format);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the fixture directory");
        }
        fs::write(path, bytes).expect("write the fixture tile");
    }

    /// A tile the way a host serves one, encoded in the format the capture
    /// would have stored byte for byte.
    fn captured_tile_bytes(size_px: u32, format: CapturedTileFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbImage::from_pixel(size_px, size_px, image::Rgb([9, 99, 199]))
            .write_to(
                &mut Cursor::new(&mut bytes),
                match format {
                    CapturedTileFormat::Jpeg => image::ImageFormat::Jpeg,
                    CapturedTileFormat::Png => image::ImageFormat::Png,
                },
            )
            .expect("encode the captured tile");
        bytes
    }

    /// A tile the directory holds nothing for and one it holds something
    /// undecodable for both leave the base layer blank and both are recorded.
    #[rstest::rstest]
    #[case::no_file(None)]
    #[case::undecodable_file(Some(b"not an image".as_slice()))]
    fn a_tile_the_directory_cannot_serve_stays_blank_and_is_recorded(
        #[case] written: Option<&[u8]>,
    ) {
        let directory = tempfile::tempdir().expect("temp dir");
        manifest(512, CapturedTileFormat::Jpeg)
            .write(directory.path())
            .expect("write the manifest");
        if let Some(bytes) = written {
            write_tile_file(directory.path(), TILE_ID, CapturedTileFormat::Jpeg, bytes);
        }
        let mut tiles = FixtureTiles::new(directory.path().to_owned(), Context::default())
            .expect("read the manifest");

        assert!(tiles.at(TILE_ID).is_none());
        assert_eq!(
            tiles.missing_tiles(),
            &BTreeSet::from([FixtureTileId::from(TILE_ID)])
        );
    }

    /// The map reads a tile back from the path the manifest's format spells
    /// and draws it at the size the manifest states, whatever the host served.
    #[rstest::rstest]
    #[case::mapbox_jpeg(512, CapturedTileFormat::Jpeg)]
    #[case::slippy_png(256, CapturedTileFormat::Png)]
    fn a_captured_tile_is_served_whole_in_the_size_and_format_the_manifest_states(
        #[case] tile_size_px: u32,
        #[case] tile_format: CapturedTileFormat,
    ) {
        let directory = tempfile::tempdir().expect("temp dir");
        manifest(tile_size_px, tile_format)
            .write(directory.path())
            .expect("write the manifest");
        write_tile_file(
            directory.path(),
            TILE_ID,
            tile_format,
            &captured_tile_bytes(tile_size_px, tile_format),
        );
        let mut tiles = FixtureTiles::new(directory.path().to_owned(), Context::default())
            .expect("read the manifest");

        let piece = tiles.at(TILE_ID).expect("the captured tile is served");
        let Tile::Raster(texture) = piece.tile;
        let expected_size = usize::try_from(tile_size_px).expect("a tile size fits a texture");
        assert_eq!(texture.size(), [expected_size; 2]);
        assert_eq!(piece.uv, FULL_TILE_UV);
        assert_eq!(tiles.tile_size(), tile_size_px);
        assert!(tiles.missing_tiles().is_empty());
    }

    /// A tile written under the other format's extension is one the map never
    /// looks for.
    #[test]
    fn a_tile_stored_in_another_format_than_the_manifest_states_is_missing() {
        let directory = tempfile::tempdir().expect("temp dir");
        manifest(512, CapturedTileFormat::Jpeg)
            .write(directory.path())
            .expect("write the manifest");
        write_tile_file(
            directory.path(),
            TILE_ID,
            CapturedTileFormat::Png,
            &captured_tile_bytes(512, CapturedTileFormat::Png),
        );
        let mut tiles = FixtureTiles::new(directory.path().to_owned(), Context::default())
            .expect("read the manifest");

        assert!(tiles.at(TILE_ID).is_none());
        assert_eq!(
            tiles.missing_tiles(),
            &BTreeSet::from([FixtureTileId::from(TILE_ID)])
        );
    }

    #[rstest::rstest]
    #[case::jpeg("image/jpeg", Some(CapturedTileFormat::Jpeg))]
    #[case::png_with_parameters("image/png; charset=binary", Some(CapturedTileFormat::Png))]
    #[case::webp("image/webp", None)]
    fn a_content_type_names_the_format_the_capture_stores(
        #[case] content_type: &str,
        #[case] expected: Option<CapturedTileFormat>,
    ) {
        assert_eq!(
            CapturedTileFormat::from_content_type(content_type),
            expected
        );
    }

    #[test]
    fn a_directory_without_a_manifest_holds_no_capture() {
        let directory = tempfile::tempdir().expect("temp dir");

        let error = FixtureTiles::new(directory.path().to_owned(), Context::default())
            .err()
            .expect("a directory without a manifest is refused");

        assert!(matches!(error, TileFixtureManifestError::File { .. }));
    }

    #[test]
    fn a_written_manifest_reads_back_unchanged() {
        let directory = tempfile::tempdir().expect("temp dir");
        let written = manifest(512, CapturedTileFormat::Jpeg);

        written.write(directory.path()).expect("write");

        assert_eq!(
            TileFixtureManifest::read(directory.path()).expect("read"),
            written
        );
    }

    #[rstest::rstest]
    #[case::too_few_fields("3/4")]
    #[case::too_many_fields("3/4/5/6")]
    #[case::not_a_number("3/x/5")]
    #[case::zoom_past_a_byte("300/4/5")]
    fn a_malformed_tile_id_is_refused(#[case] text: &str) {
        text.parse::<FixtureTileId>()
            .expect_err("a malformed tile id is refused");
    }

    proptest::proptest! {
        /// The wanted-tile list a snapshot run records is written and read
        /// back through this pair.
        #[test]
        fn a_tile_id_reads_back_from_its_text(
            zoom in 0u8..=22,
            x in 0u32..1 << 22,
            y in 0u32..1 << 22,
        ) {
            let tile_id = FixtureTileId { zoom, x, y };
            proptest::prop_assert_eq!(
                tile_id.to_string().parse::<FixtureTileId>().ok(),
                Some(tile_id)
            );
        }
    }
}
