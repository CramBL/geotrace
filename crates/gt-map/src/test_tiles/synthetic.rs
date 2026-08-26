//! Base tiles generated in process. Each one is labelled with its own id and
//! the degrees of its north-west corner, which states on the rendered map
//! where it is framed.

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};
use rustc_hash::FxHashMap;
use walkers::sources::Attribution;
use walkers::{Tile, TileId, TilePiece, Tiles};

use gt_types::mercator::{self, MercPoint};

use crate::test_tiles::{FULL_TILE_UV, TILE_SIZE_PX, glyph};

/// Muted enough that a track's own colours stay legible over it. The two
/// backgrounds alternate so neighbouring tiles show their shared edge.
mod palette {
    use egui::Color32;

    pub const BACKGROUND_EVEN: Color32 = Color32::from_rgb(0xE4, 0xE2, 0xDB);
    pub const BACKGROUND_ODD: Color32 = Color32::from_rgb(0xD6, 0xD4, 0xCB);
    pub const GRID_LINE: Color32 = Color32::from_rgb(0xC2, 0xBF, 0xB4);
    pub const BORDER: Color32 = Color32::from_rgb(0x93, 0x8F, 0x84);
    pub const LABEL: Color32 = Color32::from_rgb(0x4B, 0x48, 0x41);
}

const GRID_SPACING_PX: usize = 32;
/// Wide enough that nearest-neighbour sampling still catches a line where the
/// map draws a tile smaller than its texture.
const GRID_LINE_WIDTH_PX: usize = 2;
const BORDER_WIDTH_PX: usize = 3;
const GLYPH_SCALE: usize = 2;
const GLYPH_ADVANCE_PX: usize = (glyph::WIDTH_PX + 1) * GLYPH_SCALE;
const LABEL_ORIGIN: TilePixel = TilePixel { x: 10, y: 10 };
const LABEL_LINE_SPACING_PX: usize = glyph::HEIGHT_PX * GLYPH_SCALE + 4;

/// A pixel of a tile, x rightwards from its west edge and y downwards from its
/// north edge.
#[derive(Clone, Copy)]
struct TilePixel {
    x: usize,
    y: usize,
}

/// The north-west corner of a tile in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TileCorner {
    latitude_degrees: f64,
    longitude_degrees: f64,
}

impl TileCorner {
    fn north_west_of(tile_id: TileId) -> Self {
        let tiles_per_axis = f64::from(tile_id.zoom).exp2();
        let (latitude_degrees, longitude_degrees) = mercator::denormalize(MercPoint {
            x: f64::from(tile_id.x) / tiles_per_axis,
            y: f64::from(tile_id.y) / tiles_per_axis,
        });
        Self {
            latitude_degrees,
            longitude_degrees,
        }
    }

    /// Longitude then latitude, each to one decimal with its hemisphere, as in
    /// `179.5E 55.0N`.
    fn label(self) -> String {
        let east_west = if self.longitude_degrees < 0.0 {
            "W"
        } else {
            "E"
        };
        let north_south = if self.latitude_degrees < 0.0 {
            "S"
        } else {
            "N"
        };
        format!(
            "{:.1}{east_west} {:.1}{north_south}",
            self.longitude_degrees.abs(),
            self.latitude_degrees.abs()
        )
    }
}

struct TileCanvas {
    pixels: Vec<Color32>,
}

impl TileCanvas {
    fn filled(color: Color32) -> Self {
        Self {
            pixels: vec![color; TILE_SIZE_PX * TILE_SIZE_PX],
        }
    }

    fn set_pixel(&mut self, at: TilePixel, color: Color32) {
        if at.x >= TILE_SIZE_PX {
            return;
        }
        if let Some(pixel) = self.pixels.get_mut(at.y * TILE_SIZE_PX + at.x) {
            *pixel = color;
        }
    }

    fn fill_square(&mut self, top_left: TilePixel, side_px: usize, color: Color32) {
        for y in 0..side_px {
            for x in 0..side_px {
                self.set_pixel(
                    TilePixel {
                        x: top_left.x + x,
                        y: top_left.y + y,
                    },
                    color,
                );
            }
        }
    }

    fn draw_grid_and_border(&mut self) {
        for y in 0..TILE_SIZE_PX {
            for x in 0..TILE_SIZE_PX {
                let on_border = x < BORDER_WIDTH_PX
                    || y < BORDER_WIDTH_PX
                    || x >= TILE_SIZE_PX - BORDER_WIDTH_PX
                    || y >= TILE_SIZE_PX - BORDER_WIDTH_PX;
                let on_grid_line = x % GRID_SPACING_PX < GRID_LINE_WIDTH_PX
                    || y % GRID_SPACING_PX < GRID_LINE_WIDTH_PX;
                if on_border {
                    self.set_pixel(TilePixel { x, y }, palette::BORDER);
                } else if on_grid_line {
                    self.set_pixel(TilePixel { x, y }, palette::GRID_LINE);
                }
            }
        }
    }

    fn draw_text(&mut self, text: &str, top_left: TilePixel, color: Color32) {
        for (position, character) in text.chars().enumerate() {
            let Some(rows) = glyph::bitmap(character) else {
                continue;
            };
            let left = top_left.x + position * GLYPH_ADVANCE_PX;
            for (row_index, row) in rows.into_iter().enumerate() {
                for column in 0..glyph::WIDTH_PX {
                    if row & (1u8 << (glyph::WIDTH_PX - 1 - column)) == 0 {
                        continue;
                    }
                    self.fill_square(
                        TilePixel {
                            x: left + column * GLYPH_SCALE,
                            y: top_left.y + row_index * GLYPH_SCALE,
                        },
                        GLYPH_SCALE,
                        color,
                    );
                }
            }
        }
    }

    fn into_image(self) -> ColorImage {
        ColorImage::new([TILE_SIZE_PX; 2], self.pixels)
    }
}

pub struct SyntheticTiles {
    egui_ctx: Context,
    textures: FxHashMap<TileId, TextureHandle>,
}

impl SyntheticTiles {
    pub fn new(egui_ctx: Context) -> Self {
        Self {
            egui_ctx,
            textures: FxHashMap::default(),
        }
    }

    /// The same pixels for the same tile id on every call and every machine.
    pub fn tile_image(tile_id: TileId) -> ColorImage {
        let background = if tile_id.x % 2 == tile_id.y % 2 {
            palette::BACKGROUND_EVEN
        } else {
            palette::BACKGROUND_ODD
        };
        let mut canvas = TileCanvas::filled(background);
        canvas.draw_grid_and_border();
        let TileId { x, y, zoom } = tile_id;
        canvas.draw_text(&format!("{zoom}/{x}/{y}"), LABEL_ORIGIN, palette::LABEL);
        canvas.draw_text(
            &TileCorner::north_west_of(tile_id).label(),
            TilePixel {
                x: LABEL_ORIGIN.x,
                y: LABEL_ORIGIN.y + LABEL_LINE_SPACING_PX,
            },
            palette::LABEL,
        );
        canvas.into_image()
    }
}

impl Tiles for SyntheticTiles {
    fn at(&mut self, tile_id: TileId) -> Option<TilePiece> {
        let Self { egui_ctx, textures } = self;
        let texture = textures.entry(tile_id).or_insert_with(|| {
            let TileId { x, y, zoom } = tile_id;
            egui_ctx.load_texture(
                format!("synthetic tile {zoom}/{x}/{y}"),
                SyntheticTiles::tile_image(tile_id),
                TextureOptions::NEAREST,
            )
        });
        Some(TilePiece::new(Tile::Raster(texture.clone()), FULL_TILE_UV))
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            text: "Synthetic tiles",
            url: "",
            logo_light: None,
            logo_dark: None,
        }
    }

    fn tile_size(&self) -> u32 {
        TILE_SIZE_PX as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corner the label states is the one the slippy tile scheme puts at
    /// the tile's north-west, wherever that lands.
    #[rstest::rstest]
    #[case::whole_world(0, 0, 0, "180.0W 85.1N")]
    #[case::prime_meridian(1, 1, 0, "0.0E 85.1N")]
    #[case::antimeridian_at_the_equator(1, 0, 1, "180.0W 0.0N")]
    #[case::northern_hemisphere(3, 7, 3, "135.0E 41.0N")]
    #[case::southern_hemisphere(2, 3, 3, "90.0E 66.5S")]
    fn the_label_states_the_north_west_corner(
        #[case] zoom: u8,
        #[case] x: u32,
        #[case] y: u32,
        #[case] expected: &str,
    ) {
        assert_eq!(
            TileCorner::north_west_of(TileId { x, y, zoom }).label(),
            expected
        );
    }

    /// Every character the font claims draws: it has a glyph, and drawing it
    /// puts ink on the canvas. The space is the one that leaves the
    /// background as it found it.
    #[test]
    fn every_drawable_character_draws() {
        for character in glyph::DRAWABLE_CHARACTERS.chars() {
            assert!(
                glyph::bitmap(character).is_some(),
                "{character:?} has no glyph"
            );
            let mut canvas = TileCanvas::filled(palette::BACKGROUND_EVEN);
            canvas.draw_text(&character.to_string(), LABEL_ORIGIN, palette::LABEL);
            let inked = canvas
                .pixels
                .iter()
                .filter(|pixel| **pixel == palette::LABEL)
                .count();
            if character == ' ' {
                assert_eq!(inked, 0, "the space inked {inked} pixels");
            } else {
                assert!(inked > 0, "{character:?} left no ink");
            }
        }
    }

    #[test]
    fn a_tile_id_always_draws_the_same_pixels() {
        let tile_id = TileId {
            x: 9,
            y: 6,
            zoom: 4,
        };
        let first = SyntheticTiles::tile_image(tile_id);
        let second = SyntheticTiles::tile_image(tile_id);
        assert_eq!(first.size, [TILE_SIZE_PX; 2]);
        assert_eq!(first.pixels, second.pixels);
    }

    proptest::proptest! {
        /// The font covers the label of every tile the map requests, at any
        /// zoom: a character outside it drops out of the tile unnoticed.
        #[test]
        fn every_corner_label_is_drawable(
            zoom in 0u8..=19,
            x in 0u32..1 << 19,
            y in 0u32..1 << 19,
        ) {
            let label = TileCorner::north_west_of(TileId { x, y, zoom }).label();
            for character in label.chars() {
                proptest::prop_assert!(
                    glyph::bitmap(character).is_some(),
                    "{character:?} of {label:?} has no glyph"
                );
            }
        }
    }
}
