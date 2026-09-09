//! Capture live IONEX files from the JPL archive.
//!
//! Requests each file of [`gt_ionex::CAPTURED_FILES`] into `tests/captures/`,
//! decompressed because the parser reads text, and records what the archive
//! served in `capture.json` alongside the capture date.
//!
//! Captures are frozen once committed. A re-capture's diff is reviewed like
//! code.
//!
//! Usage: `just ionex-captures [NAME...]`, or
//! `cargo run -p gt-ionex --example fetch_ionex_captures -- [NAME...]`.
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
use gt_ionex::{CAPTURED_FILES, CapturedFile, captures_dir, parse};

#[path = "shared/capture_manifest.rs"]
mod capture_manifest;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Pause between requests: the archive is a small public research host.
const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<(), Box<dyn Error>> {
    let dir = captures_dir();
    fs::create_dir_all(&dir)?;

    // Positional arguments select a subset. Without them the capture covers
    // every file.
    let args: Vec<String> = env::args().skip(1).collect();
    let selected: Vec<CapturedFile> = if args.is_empty() {
        CAPTURED_FILES.to_vec()
    } else {
        args.iter()
            .map(|name| {
                CAPTURED_FILES
                    .iter()
                    .copied()
                    .find(|capture| capture.name == name)
                    .unwrap_or_else(|| {
                        panic!("{name:?} is not a declared capture - add it to CAPTURED_FILES")
                    })
            })
            .collect()
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;

    // Start from the existing entries, so a subset capture keeps the rest.
    let mut entries_by_name = capture_manifest::recorded_entries(&dir, "name");

    for (position, capture) in selected.iter().enumerate() {
        if position > 0 {
            thread::sleep(REQUEST_INTERVAL);
        }
        let response = client.get(capture.url).send()?;
        let status = response.status().as_u16();
        let compressed = response.bytes()?;
        let mut text = String::new();
        GzDecoder::new(compressed.as_ref()).read_to_string(&mut text)?;
        println!(
            "{}: HTTP {status} ({} bytes compressed, {} decompressed)",
            capture.name,
            compressed.len(),
            text.len()
        );

        // Written only once it parses, so a capture on disk is always one the
        // parser accepts.
        let maps = parse::global_ionosphere_maps(&text)
            .map_err(|err| format!("{}: not written, {err}", capture.name))?;
        println!(
            "  {} maps, {} by {} nodes, peak {:?} TECU",
            maps.maps().len(),
            maps.grid().latitudes.node_count(),
            maps.grid().longitudes.node_count(),
            maps.peak_total_electron_content()
                .map(TotalElectronContent::tecu)
        );
        fs::write(dir.join(capture.file_name), &text)?;

        entries_by_name.insert(
            capture.name.to_owned(),
            capture_manifest::entry(naming_fields(capture, status), &maps),
        );
    }

    // Declared order, not capture order, so a partial re-capture diffs
    // cleanly.
    let files: Vec<Value> = CAPTURED_FILES
        .iter()
        .filter_map(|capture| entries_by_name.get(capture.name).cloned())
        .collect();
    capture_manifest::write(&dir, &files)?;

    Ok(())
}

fn naming_fields(capture: &CapturedFile, http_status: u16) -> Value {
    json!({
        "name": capture.name,
        "file_name": capture.file_name,
        "url": capture.url,
        "captured_at": Utc::now().to_rfc3339(),
        "http_status": http_status,
    })
}
