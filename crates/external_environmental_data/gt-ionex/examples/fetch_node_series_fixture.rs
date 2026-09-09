//! Capture the per-node TEC series of the May 2024 storm from the JPL
//! archive.
//!
//! Requests every day of [`gt_ionex::NODE_SERIES_DAYS`], parses each published
//! file with the crate's own parser, and keeps only what
//! [`gt_ionex::FIXTURE_NODES`] carries at each of that day's epochs, written
//! to `tests/fixtures/node_series.json`. A month of whole files is 30 MB: this
//! is the part the storm index reads.
//!
//! Fixtures are frozen once committed. A re-capture's diff is reviewed like
//! code.
//!
//! Usage: `just ionex-node-series`, or
//! `cargo run -p gt-ionex --example fetch_node_series_fixture`.

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
use std::io::Read as _;
use std::time::Duration;
use std::{fs, thread};

use chrono::{NaiveDate, Utc};
use flate2::read::GzDecoder;
use gt_types::{Latitude, Longitude};

use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::node_series::{CapturedNodeDay, NodeSeriesCapture};
use gt_ionex::tec::TotalElectronContent;
use gt_ionex::{
    DEFAULT_BASE_URL, FIXTURE_NODES, FixtureNode, IonexProduct, NODE_SERIES_CAPTURE,
    NODE_SERIES_DAYS, fixtures_dir, parse,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Pause between requests: the archive is a small public research host.
const REQUEST_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> Result<(), Box<dyn Error>> {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()?;

    for node in FIXTURE_NODES {
        println!(
            "{}: {:.1} N, {:.1} E - {}",
            node.name, node.latitude_degrees, node.longitude_degrees, node.purpose
        );
    }

    let (first, last) = NODE_SERIES_DAYS;
    let mut days = Vec::new();
    let mut day = first;
    while day <= last {
        if !days.is_empty() {
            thread::sleep(REQUEST_INTERVAL);
        }
        days.push(capture_day(&client, day)?);
        day = day.succ_opt().ok_or("a calendar date")?;
    }

    let capture = NodeSeriesCapture {
        captured_at: Utc::now().to_rfc3339(),
        base_url: DEFAULT_BASE_URL.to_owned(),
        days,
    };
    let path = dir.join(NODE_SERIES_CAPTURE);
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&capture)?),
    )?;
    println!(
        "wrote {} ({} bytes)",
        path.display(),
        fs::metadata(&path)?.len()
    );
    Ok(())
}

fn capture_day(
    client: &reqwest::blocking::Client,
    day: NaiveDate,
) -> Result<CapturedNodeDay, Box<dyn Error>> {
    let url = IonexProduct::Final.file_url(DEFAULT_BASE_URL, day);
    let response = client.get(&url).send()?;
    let http_status = response.status().as_u16();
    let compressed = response.bytes()?;
    let mut text = String::new();
    GzDecoder::new(compressed.as_ref()).read_to_string(&mut text)?;
    let maps = parse::global_ionosphere_maps(&text).map_err(|err| format!("{day}: {err}"))?;

    let values_tecu: BTreeMap<String, Vec<Option<f64>>> = FIXTURE_NODES
        .iter()
        .map(|node| (node.name.to_owned(), node_values(&maps, node)))
        .collect();
    let peak_tecu = maps
        .peak_total_electron_content()
        .map(TotalElectronContent::tecu);
    println!(
        "{day}: HTTP {http_status}, {} maps, peak {peak_tecu:?} TECU",
        maps.maps().len()
    );

    Ok(CapturedNodeDay {
        day,
        file_name: IonexProduct::Final
            .file_name(day)
            .trim_end_matches(gt_ionex::COMPRESSED_SUFFIX)
            .to_owned(),
        url,
        http_status,
        interval_seconds: maps.interval().num_seconds(),
        peak_tecu,
        values_tecu,
    })
}

/// The node's value in every map of the day, in epoch order. A value the
/// producer left unpublished is kept as a gap.
fn node_values(maps: &GlobalIonosphereMaps, node: &FixtureNode) -> Vec<Option<f64>> {
    let point = maps.grid().nearest_node(
        Latitude::new(node.latitude_degrees),
        Longitude::new(node.longitude_degrees),
    );
    maps.maps()
        .iter()
        .map(|map| {
            point
                .and_then(|point| map.value_at(point))
                .map(TotalElectronContent::tecu)
        })
        .collect()
}
