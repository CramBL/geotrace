//! Check the CDDIS addressing against the live archive.
//!
//! Requests one settled day under every name [`gt_ionex::cddis`] files it
//! under, reads each served file through the fetch pipeline's own decoders,
//! and compares the maps against the JPL capture of the same day committed
//! under `tests/fixtures/`. The archive serves the same producer's file under
//! a long IGS name and a legacy `.Z` one, so both must read as that capture.
//!
//! Usage: `EARTHDATA_TOKEN=... just cddis-verify [--capture]`, or
//! `cargo run -p gt-ionex --example verify_cddis_mirror -- [--capture]`.
//! `--capture` writes the served files under `tests/fixtures/cddis/` for
//! review.

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

use chrono::NaiveDate;
use gt_fetch::{HttpRequest, HttpTransport, SecretToken, Transport};

use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::mirrors::FileCandidate;
use gt_ionex::{
    FIXTURE_FILES, IonexProduct, Mirror, MirrorLayout, STORM_CAPTURE, fixtures_dir, parse,
    transport,
};

/// Holds the NASA Earthdata token the archive requires.
const TOKEN_ENV: &str = "EARTHDATA_TOKEN";

/// Where `--capture` writes the served files, under [`fixtures_dir`].
const CAPTURE_DIR: &str = "cddis";

/// The day this runs against: the one the committed JPL capture holds, so the
/// files the archive serves have something to be read against.
const VERIFIED_DAY: (i32, u32, u32) = (2024, 5, 10);

fn main() -> Result<(), Box<dyn Error>> {
    let token = env::var(TOKEN_ENV)
        .ok()
        .and_then(|entered| SecretToken::new(&entered))
        .ok_or_else(|| format!("set {TOKEN_ENV} to a NASA Earthdata token"))?;
    let capture = env::args().skip(1).any(|argument| argument == "--capture");

    let (year, month, day_of_month) = VERIFIED_DAY;
    let day = NaiveDate::from_ymd_opt(year, month, day_of_month)
        .ok_or("VERIFIED_DAY must name a real calendar date")?;
    let expected = captured_maps()?;
    println!(
        "The committed JPL capture of {day} holds {} maps on a {} by {} grid",
        expected.maps().len(),
        expected.grid().latitudes.node_count(),
        expected.grid().longitudes.node_count(),
    );

    let transport = HttpTransport::new(Some(transport::REQUEST_INTERVAL))?;
    let mirror = Mirror::publishing(MirrorLayout::Cddis);
    let mut served = 0_usize;
    let mut mismatched = 0_usize;

    for candidate in mirror.file_candidates(IonexProduct::Final, day) {
        let FileCandidate { url, compression } = &candidate;
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

        match transport::read_served_file(&response.body, *compression) {
            Ok(maps) => {
                if maps == expected {
                    println!("  reads as the committed JPL capture of the same day");
                } else {
                    mismatched = mismatched.saturating_add(1);
                    println!(
                        "  DIFFERS from the committed JPL capture: {} maps, {} by {} grid, peak {:?} TECU",
                        maps.maps().len(),
                        maps.grid().latitudes.node_count(),
                        maps.grid().longitudes.node_count(),
                        maps.peak_total_electron_content(),
                    );
                }
            }
            Err(err) => {
                mismatched = mismatched.saturating_add(1);
                println!("  UNREADABLE: {err}");
            }
        }

        if capture {
            let path = fixtures_dir().join(CAPTURE_DIR).join(file_name(url)?);
            fs::create_dir_all(path.parent().ok_or("the capture path has a parent")?)?;
            fs::write(&path, &response.body)?;
            println!("  written to {}", path.display());
        }
    }

    println!(
        "\n{served} of the addressed names served a file, {mismatched} of them unreadable or differing"
    );
    if served == 0 {
        return Err("the archive served none of the names this addressing builds".into());
    }
    if mismatched > 0 {
        return Err("a served file did not read as the committed JPL capture".into());
    }
    Ok(())
}

/// The committed JPL capture of [`VERIFIED_DAY`].
fn captured_maps() -> Result<GlobalIonosphereMaps, Box<dyn Error>> {
    let fixture = FIXTURE_FILES
        .iter()
        .find(|fixture| fixture.name == STORM_CAPTURE)
        .ok_or("the storm capture is declared in FIXTURE_FILES")?;
    let text = fs::read_to_string(fixtures_dir().join(fixture.file_name))?;
    Ok(parse::global_ionosphere_maps(&text)?)
}

fn file_name(url: &str) -> Result<&str, Box<dyn Error>> {
    url.rsplit('/')
        .next()
        .ok_or_else(|| format!("{url} names no file").into())
}
