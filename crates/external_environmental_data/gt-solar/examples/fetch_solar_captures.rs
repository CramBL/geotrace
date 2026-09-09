//! Capture live index responses from the GFZ Potsdam service.
//!
//! Requests each window of [`gt_solar::CAPTURED_WINDOWS`] into
//! `tests/captures/`, one file per window, and records what the service
//! returned in `capture.json` alongside the capture date and host.
//!
//! Captures are frozen once committed. A re-capture's diff is reviewed like
//! code.
//!
//! Usage: `just solar-captures [NAME...]`, or
//! `cargo run -p gt-solar --example fetch_solar_captures -- [NAME...]`.
//! Naming windows captures only those, keeping the manifest entries of the
//! rest. No arguments re-captures everything.
//! Point it at a mirror with `GEOTRACE_SOLAR_HOST=https://mirror.example`.

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

use gt_solar::wire;
use gt_solar::{
    CAPTURE_MANIFEST, CAPTURED_WINDOWS, CapturedWindow, DEFAULT_BASE_URL, GeomagneticIndex,
    captures_dir, index_url,
};

/// Points the capture at a mirror. The capture requests from `DEFAULT_BASE_URL`
/// when it is unset.
const HOST_ENV: &str = "GEOTRACE_SOLAR_HOST";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Pause between requests: the service is a small public research endpoint.
const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<(), Box<dyn Error>> {
    let host = env::var(HOST_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
    let dir = captures_dir();
    fs::create_dir_all(&dir)?;

    // Positional arguments select a subset. Without them the capture covers
    // every window.
    let args: Vec<String> = env::args().skip(1).collect();
    let selected: Vec<CapturedWindow> = if args.is_empty() {
        CAPTURED_WINDOWS.to_vec()
    } else {
        args.iter()
            .map(|name| {
                CAPTURED_WINDOWS
                    .iter()
                    .copied()
                    .find(|capture| capture.name == name)
                    .unwrap_or_else(|| {
                        panic!(
                            "{name:?} is not a declared capture window - add it to CAPTURED_WINDOWS"
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

    for (position, capture) in selected.iter().enumerate() {
        if position > 0 {
            thread::sleep(REQUEST_INTERVAL);
        }
        let response = client
            .get(index_url(&host, capture.index, capture.window()?))
            .send()?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response.text()?;
        println!("{}: HTTP {status} ({} bytes)", capture.name, body.len());

        // Written only once it parses, so a capture on disk is always one the
        // parser accepts.
        let samples = match capture.index {
            GeomagneticIndex::Kp => wire::parse_kp_series(&body).map(|series| series.samples.len()),
            GeomagneticIndex::Hp30 => {
                wire::parse_hp30_series(&body).map(|series| series.samples.len())
            }
        }
        .map_err(|err| format!("{}: not written, {err}", capture.name))?;
        println!("  {samples} samples");
        fs::write(dir.join(capture.file_name()), &body)?;

        let meta = serde_json::from_str::<Value>(&body)?;
        let meta_field = |field: &str| {
            meta.get("meta")
                .and_then(|meta| meta.get(field))
                .cloned()
                .unwrap_or(Value::Null)
        };
        entries_by_name.insert(
            capture.name.to_owned(),
            json!({
                "name": capture.name,
                "index": capture.index.wire_name(),
                "start": capture.start,
                "end": capture.end,
                "captured_at": Utc::now().to_rfc3339(),
                "host": host,
                "http_status": status,
                "content_type": content_type,
                "samples": samples,
                "license": meta_field("license"),
                "source": meta_field("source"),
            }),
        );
    }

    // Declared order, not capture order, so a partial re-capture diffs
    // cleanly.
    let windows: Vec<Value> = CAPTURED_WINDOWS
        .iter()
        .filter_map(|capture| entries_by_name.get(capture.name).cloned())
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
