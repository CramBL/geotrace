//! Check the CDDIS addressing against the live archive.
//!
//! Requests one day under every name [`gt_ionex::cddis`] files it under and
//! reads each served file through the fetch pipeline's own decoders. A day the
//! workspace holds a JPL capture of is read against that capture, since the
//! archive serves the same producer's file. Any other day is parsed as a
//! standalone IONEX file, held only to the parser's own header cross-checks,
//! and its grid and maps are reported.
//!
//! Usage: `EARTHDATA_TOKEN=... just cddis-verify [--day YYYY-MM-DD]
//! [--capture]`, or `cargo run -p gt-ionex --example verify_cddis_mirror --
//! [ARGS]`. `--day` reaches the legacy era as well as the current one: it
//! states the day to request, [`DEFAULT_DAY`] by default. `--capture` writes
//! each served file under `tests/fixtures/cddis/` and records it in the
//! manifest beside them. The token authenticates a request header. It is never
//! written to a manifest field.

// Examples favour brevity: the core's robustness restriction lints (no
// unwrap/expect/panic/indexing, no std::env::temp_dir) are not enforced on
// demonstration code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::cognitive_complexity,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "verification tool: development-only code"
)]

use std::error::Error;
use std::{env, fs};

use chrono::{NaiveDate, Utc};
use gt_fetch::{HttpRequest, HttpTransport, SecretToken, Transport};
use serde_json::{Value, json};

use gt_ionex::mirrors::FileCandidate;
use gt_ionex::tec::TotalElectronContent;
use gt_ionex::{
    CAPTURE_MANIFEST, IonexProduct, Mirror, MirrorLayout, captured_text, cddis_fixtures_dir,
    declared_fixture_for_day, parse, transport,
};

#[path = "shared/capture_manifest.rs"]
mod capture_manifest;

/// Holds the NASA Earthdata token the archive requires.
const TOKEN_ENV: &str = "EARTHDATA_TOKEN";

/// The day this runs against unless `--day` names another. The committed JPL
/// capture holds it, which gives the served files something to be read
/// against.
const DEFAULT_DAY: &str = "2024-05-10";

const DAY_FORMAT: &str = "%Y-%m-%d";

/// The product this addresses: the settled one, which every archived day has.
const REQUESTED_PRODUCT: IonexProduct = IonexProduct::Final;

struct Arguments {
    day: NaiveDate,
    capture: bool,
}

impl Arguments {
    fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut day = DEFAULT_DAY.to_owned();
        let mut capture = false;
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--capture" => capture = true,
                "--day" => {
                    day = arguments.next().ok_or("--day names a day, as YYYY-MM-DD")?;
                }
                other => {
                    return Err(format!("{other}: expected --day YYYY-MM-DD or --capture").into());
                }
            }
        }
        Ok(Self {
            day: NaiveDate::parse_from_str(&day, DAY_FORMAT)?,
            capture,
        })
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let token = env::var(TOKEN_ENV)
        .ok()
        .and_then(|entered| SecretToken::new(&entered))
        .ok_or_else(|| format!("set {TOKEN_ENV} to a NASA Earthdata token"))?;
    let Arguments { day, capture } = Arguments::parse(env::args().skip(1))?;

    let expected = match declared_fixture_for_day(REQUESTED_PRODUCT, day) {
        Some(fixture) => {
            let maps = parse::global_ionosphere_maps(&captured_text(fixture)?)?;
            println!(
                "The committed JPL capture of {day} holds {} maps on a {} by {} grid",
                maps.maps().len(),
                maps.grid().latitudes.node_count(),
                maps.grid().longitudes.node_count(),
            );
            Some(maps)
        }
        None => {
            println!(
                "No JPL capture of {day} is committed: each served file is read as an IONEX file in its own right"
            );
            None
        }
    };

    let directory = cddis_fixtures_dir();
    let mut entries_by_file_name = capture_manifest::recorded_entries(&directory, "file_name");
    let transport = HttpTransport::new(Some(transport::REQUEST_INTERVAL))?;
    let mirror = Mirror::publishing(MirrorLayout::Cddis);
    let mut served = 0_usize;
    let mut mismatched = 0_usize;

    for FileCandidate { url, compression } in mirror.file_candidates(REQUESTED_PRODUCT, day) {
        let request = HttpRequest::get_with_bearer_token(url.clone(), token.clone());
        let response = Transport::<Vec<u8>>::send(&transport, &request)
            .map_err(|err| token.redact(&format!("{err:#}")))?;
        println!(
            "\n{url}\n  HTTP {} ({} bytes)",
            response.status,
            response.body.len()
        );
        if response.status != 200 {
            continue;
        }
        served = served.saturating_add(1);

        let maps = match transport::read_served_file(&response.body, compression) {
            Ok(maps) => maps,
            Err(err) => {
                mismatched = mismatched.saturating_add(1);
                println!("  UNREADABLE: {err}");
                continue;
            }
        };
        println!(
            "  {} maps, {} by {} grid, interval {} s, peak {:?} TECU",
            maps.maps().len(),
            maps.grid().latitudes.node_count(),
            maps.grid().longitudes.node_count(),
            maps.interval().num_seconds(),
            maps.peak_total_electron_content()
                .map(TotalElectronContent::tecu),
        );
        match &expected {
            Some(expected) if maps == *expected => {
                println!("  reads as the committed JPL capture of the same day");
            }
            Some(_) => {
                mismatched = mismatched.saturating_add(1);
                println!("  DIFFERS from the committed JPL capture of the same day");
            }
            None => {}
        }

        if capture {
            let name = file_name(&url)?;
            let path = directory.join(name);
            fs::create_dir_all(&directory)?;
            fs::write(&path, &response.body)?;
            entries_by_file_name.insert(
                name.to_owned(),
                capture_manifest::entry(
                    json!({
                        "file_name": name,
                        "url": url,
                        "day": day.to_string(),
                        "product": REQUESTED_PRODUCT.to_string(),
                        "captured_at": Utc::now().to_rfc3339(),
                        "http_status": response.status,
                    }),
                    &maps,
                ),
            );
            println!("  written to {}", path.display());
        }
    }

    if capture {
        let entries: Vec<Value> = entries_by_file_name.values().cloned().collect();
        capture_manifest::write(&directory, &entries)?;
        println!(
            "\n{} files recorded in {}",
            entries_by_file_name.len(),
            directory.join(CAPTURE_MANIFEST).display()
        );
    }
    println!(
        "\n{served} of the addressed names served a file, {mismatched} of them unreadable or differing"
    );
    if served == 0 {
        return Err("the archive served none of the names this addressing builds".into());
    }
    if mismatched > 0 {
        return Err(
            "a served file was unreadable or differed from the committed JPL capture".into(),
        );
    }
    Ok(())
}

fn file_name(url: &str) -> Result<&str, Box<dyn Error>> {
    url.rsplit('/')
        .next()
        .ok_or_else(|| format!("{url} names no file").into())
}
