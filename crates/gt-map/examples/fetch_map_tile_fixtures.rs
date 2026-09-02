//! Capture the Mapbox satellite tiles the fixture-backed map snapshots draw.
//!
//! Writes `tests/fixtures/map_tiles/{zoom}/{x}/{y}.{extension}` and the
//! `manifest.json` beside them, which records the tile size, image format,
//! style, host and capture date. The token is never written down.
//!
//! Fixtures are frozen once committed: a tile already on disk is kept and a
//! re-run only fills the gaps. A re-capture's diff is reviewed like code.
//!
//! Usage: `MAPBOX_TOKEN=... just map-tile-fixtures [WANTED...]`, or
//! `cargo run -p gt-map --example fetch_map_tile_fixtures -- [WANTED...]`.
//! Each `WANTED` is a file a snapshot run recorded under
//! `GEOTRACE_RECORD_TILE_MISSES`, one `zoom/x/y` tile id per line. Its tiles
//! are added to the ones the manifest already lists.
//!
//! Each response body is written byte for byte as the host served it: the
//! content type decides the extension, and the manifest records the format so
//! the map reads the tiles back from the same paths.

// Examples favour brevity: the core's robustness restriction lints (no
// unwrap/expect/panic/indexing, no std::env::temp_dir) are not enforced on
// demonstration code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::cognitive_complexity,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "capture tool: development-only code"
)]

use std::collections::BTreeSet;
use std::error::Error;
use std::time::Duration;
use std::{env, fs, thread};

use chrono::Utc;
use reqwest::header::CONTENT_TYPE;
use walkers::TileId;

use gt_map::mapbox_tiles;
use gt_map::test_tiles::{CapturedTileFormat, FixtureTileId, TileFixtureManifest};
use gt_test_utils::map_tile_fixture_dir;

/// Recorded in the manifest in place of the URL, which holds the token.
const HOST: &str = "api.mapbox.com";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Pause between requests, far below what a Mapbox account is allowed.
const REQUEST_INTERVAL: Duration = Duration::from_millis(200);

fn main() -> Result<(), Box<dyn Error>> {
    let token = mapbox_tiles::TOKEN_ENVS
        .iter()
        .find_map(|name| env::var(name).ok())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            format!(
                "set {} to a Mapbox access token",
                mapbox_tiles::TOKEN_ENVS[0]
            )
        })?;
    let dir = map_tile_fixture_dir();
    fs::create_dir_all(&dir)?;

    let existing = TileFixtureManifest::read(&dir).ok();
    let existing_format = existing.as_ref().map(|manifest| manifest.tile_format);
    let mut wanted: BTreeSet<FixtureTileId> =
        existing.map(|manifest| manifest.tiles).unwrap_or_default();
    for path in env::args().skip(1) {
        let recorded = fs::read_to_string(&path)?;
        for line in recorded.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            wanted.insert(line.parse()?);
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;
    let mut kept = 0_usize;
    let mut captured = 0_usize;
    // The first response's format is held for the rest of the capture: every
    // tile of one capture is stored in one format.
    let mut tile_format = existing_format;
    for tile_id in &wanted {
        if let Some(format) = tile_format
            && tile_id.path_within(&dir, format).exists()
        {
            kept += 1;
            continue;
        }
        if captured > 0 {
            thread::sleep(REQUEST_INTERVAL);
        }

        // Every error is stripped of its URL before it is shown: the URL
        // holds the token.
        let response = client
            .get(mapbox_tiles::satellite_tile_url(
                &token,
                TileId::from(*tile_id),
            ))
            .send()
            .map_err(reqwest::Error::without_url)?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let served = response
            .bytes()
            .map_err(reqwest::Error::without_url)?
            .to_vec();
        if !status.is_success() {
            return Err(format!("{tile_id}: {HOST} returned HTTP {status}").into());
        }
        let format = CapturedTileFormat::from_content_type(&content_type).ok_or_else(|| {
            format!("{tile_id}: {HOST} served {content_type:?}, which is neither JPEG nor PNG")
        })?;
        if let Some(recorded) = tile_format
            && recorded != format
        {
            return Err(format!(
                "{tile_id}: {HOST} served {content_type:?}, the capture holds {} tiles",
                recorded.extension()
            )
            .into());
        }
        tile_format = Some(format);

        let path = tile_id.path_within(&dir, format);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &served)?;
        captured += 1;
        println!("{tile_id}: {content_type} {} bytes", served.len());
    }

    let Some(tile_format) = tile_format else {
        return Err("no tile wanted: name a file a snapshot run recorded".into());
    };
    let manifest = TileFixtureManifest {
        tile_size_px: mapbox_tiles::satellite_tile_size_px(),
        tile_format,
        style: mapbox_tiles::SATELLITE_STYLE.to_owned(),
        host: HOST.to_owned(),
        captured_at: Utc::now().to_rfc3339(),
        tiles: wanted,
    };
    manifest.write(&dir)?;

    let total_bytes: u64 = manifest
        .tiles
        .iter()
        .filter_map(|tile_id| fs::metadata(tile_id.path_within(&dir, tile_format)).ok())
        .map(|metadata| metadata.len())
        .sum();
    println!(
        "{} {} tiles: {captured} captured, {kept} already on disk, {} KB in {}",
        manifest.tiles.len(),
        tile_format.extension(),
        total_bytes / 1024,
        dir.display()
    );
    Ok(())
}
