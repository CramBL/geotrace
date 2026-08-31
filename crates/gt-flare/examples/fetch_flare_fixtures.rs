//! Capture live flare responses from the NASA DONKI catalog.
//!
//! Requests each window of [`gt_flare::FIXTURE_WINDOWS`] into
//! `tests/fixtures/`, one file per window, and records what the endpoint
//! returned in `capture.json` alongside the capture date and host. The key is
//! never written down: the manifest records the host, not the URL.
//!
//! Fixtures are frozen once committed. A re-capture's diff is reviewed like
//! code.
//!
//! Usage: `GEOTRACE_FLARE_API_KEY=... just flare-fixtures [NAME...]`, or
//! `cargo run -p gt-flare --example fetch_flare_fixtures -- [NAME...]`.
//! Naming windows captures only those, keeping the manifest entries of the
//! rest. No arguments re-captures everything.
//! Point it at a proxy with `GEOTRACE_FLARE_HOST=https://proxy.example`.

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

use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;
use std::{env, fs, thread};

use chrono::Utc;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};

use gt_flare::wire;
use gt_flare::{
    ApiKey, CAPTURE_MANIFEST, DEFAULT_BASE_URL, FIXTURE_WINDOWS, FixtureWindow, fixtures_dir,
    flare_url,
};

/// Holds the key the endpoint needs. `DEMO_KEY` works for a handful of
/// requests an hour, which is what a capture costs.
const API_KEY_ENV: &str = "GEOTRACE_FLARE_API_KEY";

/// Points the capture at a proxy instead of the default host.
const HOST_ENV: &str = "GEOTRACE_FLARE_HOST";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Pause between requests, well inside what a registered key is allowed.
const REQUEST_INTERVAL: Duration = Duration::from_secs(4);

fn main() -> Result<(), Box<dyn Error>> {
    let key = env::var(API_KEY_ENV)
        .ok()
        .and_then(|entered| ApiKey::new(&entered))
        .ok_or_else(|| format!("set {API_KEY_ENV} to an api.nasa.gov key"))?;
    let host = env::var(HOST_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
    let dir = fixtures_dir();
    fs::create_dir_all(&dir)?;

    // Positional args select a subset. No args re-captures every window.
    let args: Vec<String> = env::args().skip(1).collect();
    let selected: Vec<FixtureWindow> = if args.is_empty() {
        FIXTURE_WINDOWS.to_vec()
    } else {
        args.iter()
            .map(|name| {
                FIXTURE_WINDOWS
                    .iter()
                    .copied()
                    .find(|fixture| fixture.name == name)
                    .unwrap_or_else(|| {
                        panic!(
                            "{name:?} is not a declared fixture window - add it to FIXTURE_WINDOWS"
                        )
                    })
            })
            .collect()
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;

    // Start from the existing entries, so a subset capture keeps the rest.
    let mut entries_by_name: BTreeMap<String, Value> =
        match fs::read_to_string(dir.join(CAPTURE_MANIFEST)) {
            Ok(existing) => serde_json::from_str::<Value>(&existing)?
                .get("windows")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| Some((entry.get("name")?.as_str()?.to_owned(), entry.clone())))
                .collect(),
            Err(_) => BTreeMap::new(),
        };

    for (position, fixture) in selected.iter().enumerate() {
        if position > 0 {
            thread::sleep(REQUEST_INTERVAL);
        }
        // Failures are reported through the key's own redaction: the client
        // quotes the URL it tried, and the URL holds the key.
        let response = client
            .get(flare_url(&host, fixture.window()?, &key))
            .send()
            .map_err(|err| key.redact(&format!("{err:#}")))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response
            .text()
            .map_err(|err| key.redact(&format!("{err:#}")))?;
        println!("{}: HTTP {status} ({} bytes)", fixture.name, body.len());

        // Written only once it parses, so a fixture on disk is always one the
        // parser accepts.
        let flares = wire::parse_flares(&body)
            .map_err(|err| format!("{}: not written, {err}", fixture.name))?;
        let strongest = flares
            .iter()
            .map(|flare| flare.classification)
            .max()
            .map(|classification| classification.to_string());
        println!(
            "  {} flares, strongest {}",
            flares.len(),
            strongest.as_deref().unwrap_or("none")
        );
        fs::write(dir.join(fixture.file_name()), &body)?;

        entries_by_name.insert(
            fixture.name.to_owned(),
            json!({
                "name": fixture.name,
                "start": fixture.start,
                "end": fixture.end,
                "captured_at": Utc::now().to_rfc3339(),
                "host": host,
                "http_status": status,
                "content_type": content_type,
                "flares": flares.len(),
                "strongest_class": strongest,
            }),
        );
    }

    // Declared order, not capture order, so a partial re-capture diffs
    // cleanly.
    let windows: Vec<Value> = FIXTURE_WINDOWS
        .iter()
        .filter_map(|fixture| entries_by_name.get(fixture.name).cloned())
        .collect();
    fs::write(
        dir.join(CAPTURE_MANIFEST),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({ "windows": windows }))?
        ),
    )?;

    Ok(())
}
