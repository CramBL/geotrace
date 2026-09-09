//! Capture live interference datasets from the publisher.
//!
//! Requests each day of [`gt_jam::FIXTURE_DAYS`] into `tests/fixtures/`.
//! A served day is written as its own file. A day the host refused has no
//! dataset, so only its `capture.json` entry records the status, alongside the
//! capture date and host.
//!
//! Fixtures are frozen once committed. A re-capture's diff is reviewed like
//! code.
//!
//! Usage: `just jam-fixtures [DAY...]`, or
//! `cargo run -p gt-jam --example fetch_jam_fixtures -- [DAY...]`.
//! Naming days captures only those, keeping the manifest entries of the
//! rest. No arguments re-captures everything.
//! Point it at a mirror with `GEOTRACE_JAM_HOST=https://mirror.example`.

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

use gt_jam::wire::{self, ParseWarningReporter};
use gt_jam::{
    CAPTURE_MANIFEST, DEFAULT_BASE_URL, FIXTURE_DAYS, FixtureDay, dataset_file_name, dataset_url,
    fixtures_dir, parse_day,
};

/// Points the capture at a mirror. The capture requests from `DEFAULT_BASE_URL`
/// when it is unset.
const HOST_ENV: &str = "GEOTRACE_JAM_HOST";

/// A full day is about 900 KiB uncompressed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Pause between requests: the host is a volunteer-run static site.
const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<(), Box<dyn Error>> {
    let host = env::var(HOST_ENV).unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
    let dir = fixtures_dir();
    fs::create_dir_all(&dir)?;

    // Positional arguments select a subset. Without them the capture covers
    // every day.
    let args: Vec<String> = env::args().skip(1).collect();
    let selected: Vec<FixtureDay> = if args.is_empty() {
        FIXTURE_DAYS.to_vec()
    } else {
        args.iter()
            .map(|day| {
                FIXTURE_DAYS
                    .iter()
                    .copied()
                    .find(|fixture| fixture.day == day)
                    .unwrap_or_else(|| {
                        panic!("{day:?} is not a declared fixture day - add it to FIXTURE_DAYS")
                    })
            })
            .collect()
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;

    // Start from the existing entries, so a subset capture keeps the rest.
    let mut entries_by_day: BTreeMap<String, Value> =
        match fs::read_to_string(dir.join(CAPTURE_MANIFEST)) {
            Ok(existing) => serde_json::from_str::<Value>(&existing)?
                .get("days")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry| Some((entry.get("day")?.as_str()?.to_owned(), entry.clone())))
                .collect(),
            Err(_) => BTreeMap::new(),
        };

    for (index, fixture) in selected.iter().enumerate() {
        if index > 0 {
            thread::sleep(REQUEST_INTERVAL);
        }
        let day = parse_day(fixture.day)?;
        let response = client.get(dataset_url(&host, day)).send()?;
        let served = response.status().is_success();
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response.text()?;
        println!("{}: HTTP {status} ({} bytes)", fixture.day, body.len());
        if status != fixture.http_status {
            println!(
                "  note: FIXTURE_DAYS pins HTTP {} for this day - update it or drop the day",
                fixture.http_status
            );
        }

        // Written verbatim, but only once it parses: the host gzip-encodes
        // regardless of Accept-Encoding, so a client that does not decode
        // gets compressed bytes that still look like a body and would
        // overwrite the fixture with them.
        //
        // Keyed on what the host returned now, not on what FIXTURE_DAYS
        // declares. tests/captured_days.rs catches the disagreement.
        let (rows, recorded_body) = if served {
            let reporter = ParseWarningReporter::default();
            let observations = wire::parse_dataset(&body, &reporter).map_err(|err| {
                format!(
                    "{}: not written, the response did not parse: {err}",
                    fixture.day
                )
            })?;
            let warnings = reporter.warnings().len() + reporter.suppressed();
            if warnings > 0 {
                println!("  note: {warnings} unusable rows");
            }
            fs::write(dir.join(dataset_file_name(day)), &body)?;
            (json!(observations.len()), Value::Null)
        } else {
            (Value::Null, json!(body))
        };

        entries_by_day.insert(
            fixture.day.to_owned(),
            json!({
                "day": fixture.day,
                "captured_at": Utc::now().to_rfc3339(),
                "host": host,
                "http_status": status,
                "content_type": content_type,
                "rows": rows,
                "body": recorded_body,
            }),
        );
    }

    // Declared order, not capture order, so a partial re-capture diffs
    // cleanly.
    let days: Vec<Value> = FIXTURE_DAYS
        .iter()
        .filter_map(|fixture| entries_by_day.get(fixture.day).cloned())
        .collect();
    fs::write(
        dir.join(CAPTURE_MANIFEST),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({ "days": days }))?
        ),
    )?;

    Ok(())
}
