//! Capture live IONEX files from the JPL archive.
//!
//! Requests each file of [`gt_ionex::FIXTURE_FILES`] into `tests/fixtures/`,
//! decompressed because the parser reads text, and records what the archive
//! served in `capture.json` alongside the capture date.
//!
//! Fixtures are frozen once committed. A re-capture's diff is reviewed like
//! code.
//!
//! Usage: `just ionex-fixtures [NAME...]`, or
//! `cargo run -p gt-ionex --example fetch_ionex_fixtures -- [NAME...]`.
//! Naming files captures only those, keeping the manifest entries of the
//! rest. No arguments re-captures everything.

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

use std::error::Error;
use std::io::Read as _;
use std::time::Duration;
use std::{env, fs, thread};

use chrono::Utc;
use flate2::read::GzDecoder;
use serde_json::{Value, json};

use gt_ionex::tec::TotalElectronContent;
use gt_ionex::{FIXTURE_FILES, FixtureFile, fixtures_dir, parse};

#[path = "shared/capture_manifest.rs"]
mod capture_manifest;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Pause between requests: the archive is a small public research host.
const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<(), Box<dyn Error>> {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir)?;

    // Positional args select a subset. No args re-captures every file.
    let args: Vec<String> = env::args().skip(1).collect();
    let selected: Vec<FixtureFile> = if args.is_empty() {
        FIXTURE_FILES.to_vec()
    } else {
        args.iter()
            .map(|name| {
                FIXTURE_FILES
                    .iter()
                    .copied()
                    .find(|fixture| fixture.name == name)
                    .unwrap_or_else(|| {
                        panic!("{name:?} is not a declared fixture - add it to FIXTURE_FILES")
                    })
            })
            .collect()
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;

    // Start from the existing entries, so a subset capture keeps the rest.
    let mut entries_by_name = capture_manifest::recorded_entries(&dir, "name");

    for (position, fixture) in selected.iter().enumerate() {
        if position > 0 {
            thread::sleep(REQUEST_INTERVAL);
        }
        let response = client.get(fixture.url).send()?;
        let status = response.status().as_u16();
        let compressed = response.bytes()?;
        let mut text = String::new();
        GzDecoder::new(compressed.as_ref()).read_to_string(&mut text)?;
        println!(
            "{}: HTTP {status} ({} bytes compressed, {} decompressed)",
            fixture.name,
            compressed.len(),
            text.len()
        );

        // Written only once it parses, so a fixture on disk is always one the
        // parser accepts.
        let maps = parse::global_ionosphere_maps(&text)
            .map_err(|err| format!("{}: not written, {err}", fixture.name))?;
        println!(
            "  {} maps, {} by {} nodes, peak {:?} TECU",
            maps.maps().len(),
            maps.grid().latitudes.node_count(),
            maps.grid().longitudes.node_count(),
            maps.peak_total_electron_content()
                .map(TotalElectronContent::tecu)
        );
        fs::write(dir.join(fixture.file_name), &text)?;

        entries_by_name.insert(
            fixture.name.to_owned(),
            capture_manifest::entry(naming_fields(fixture, status), &maps),
        );
    }

    // Declared order, not capture order, so a partial re-capture diffs
    // cleanly.
    let files: Vec<Value> = FIXTURE_FILES
        .iter()
        .filter_map(|fixture| entries_by_name.get(fixture.name).cloned())
        .collect();
    capture_manifest::write(&dir, &files)?;

    Ok(())
}

fn naming_fields(fixture: &FixtureFile, http_status: u16) -> Value {
    json!({
        "name": fixture.name,
        "file_name": fixture.file_name,
        "url": fixture.url,
        "captured_at": Utc::now().to_rfc3339(),
        "http_status": http_status,
    })
}
