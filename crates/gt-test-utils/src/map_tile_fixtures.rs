//! The captured base tiles the fixture-backed map snapshots draw, and the
//! check that the capture covers every tile they request.

use std::env;
use std::fmt::Display;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The environment variable holding the path a snapshot run appends the tiles
/// its map could not draw to. Defined here, so no library code reads it.
const RECORD_MISSES_ENV: &str = "GEOTRACE_RECORD_TILE_MISSES";

/// Where `just map-tile-fixtures` writes the captured tiles and their
/// manifest.
pub fn map_tile_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/map_tiles")
}

/// Fails listing every base tile `snapshot_name`'s map left blank, so no
/// snapshot quietly loses the ground under its track.
///
/// Under `GEOTRACE_RECORD_TILE_MISSES` those tiles are appended to the file at
/// that path before the failure, which is the wanted list
/// `just map-tile-fixtures` captures.
#[expect(
    clippy::expect_used,
    reason = "a recording run that cannot write its list has nothing to offer"
)]
pub fn assert_map_tile_fixture_is_complete(
    snapshot_name: &str,
    missing_tiles: impl IntoIterator<Item = impl Display>,
) {
    let missing: Vec<String> = missing_tiles
        .into_iter()
        .map(|tile_id| tile_id.to_string())
        .collect();
    if !missing.is_empty()
        && let Ok(path) = env::var(RECORD_MISSES_ENV)
    {
        let mut block = format!("# {snapshot_name}\n");
        for tile_id in &missing {
            block.push_str(tile_id);
            block.push('\n');
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| file.write_all(block.as_bytes()))
            .expect("append the wanted tiles to the recorded list");
    }
    assert!(
        missing.is_empty(),
        "{snapshot_name} drew {} base tiles the capture under {:?} does not hold: {}. \
         Record the wanted list with {RECORD_MISSES_ENV}=<path>, then capture it with \
         `just map-tile-fixtures <path>`.",
        missing.len(),
        map_tile_fixture_dir(),
        missing.join(" ")
    );
}
