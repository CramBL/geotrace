//! Read the committed CDDIS captures the way the fetch reads a served file.
//!
//! Every file under `tests/fixtures/cddis/` is what the archive served for the
//! day and product its manifest entry records, written by
//! `just cddis-verify --capture`. Addressing that day again names the file, the
//! name it was requested under decides how it is decompressed, and the decoded
//! text goes through the parser: the whole ingest path, over archive bytes.
//!
//! A capture of a day the workspace also holds a JPL capture of must read as
//! that capture. Capturing another day extends this without a change here:
//! every entry of the manifest is read.

mod support;

use std::collections::BTreeSet;

use chrono::NaiveDate;
use serde_json::Value;

use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::tec::TotalElectronContent;
use gt_ionex::{IonexProduct, Mirror, MirrorLayout, transport};

/// How far a recorded peak may stand from the one the file parses to.
const TECU_TOLERANCE: f64 = 1e-9;

/// One captured file, read through the addressing that names it.
struct CapturedFile {
    file_name: String,
    day: NaiveDate,
    product: IonexProduct,
    entry: Value,
    maps: GlobalIonosphereMaps,
}

/// Every committed capture, decompressed and parsed the way the fetch does it.
///
/// A capture whose name the CDDIS addressing no longer produces is an error
/// here: the compression comes from the candidate that addressing builds for
/// the recorded day, found by the URL the file was served from.
fn captured_files() -> Result<Vec<CapturedFile>, String> {
    let mut captures = Vec::new();
    for entry in support::cddis_manifest_entries()? {
        let file_name = recorded_str(&entry, "file_name")?.to_owned();
        let url = recorded_str(&entry, "url")?.to_owned();
        let day: NaiveDate = recorded_str(&entry, "day")?
            .parse()
            .map_err(|err| format!("{file_name}: the recorded day, {err}"))?;
        let recorded_product = recorded_str(&entry, "product")?;
        let product: IonexProduct = recorded_product
            .parse()
            .map_err(|_err| format!("{file_name}: no product is named {recorded_product}"))?;

        let candidate = Mirror::publishing(MirrorLayout::Cddis)
            .file_candidates(product, day)
            .into_iter()
            .find(|candidate| candidate.url == url)
            .ok_or_else(|| {
                format!(
                    "{file_name}: the CDDIS addressing no longer builds the URL it was served from"
                )
            })?;
        if candidate.url.rsplit('/').next() != Some(file_name.as_str()) {
            return Err(format!(
                "{file_name}: the capture is stored under another name than it was served under"
            ));
        }

        let served = support::cddis_capture_bytes(&file_name)?;
        let maps = transport::read_served_file(&served, candidate.compression)
            .map_err(|err| format!("{file_name}: {err}"))?;
        captures.push(CapturedFile {
            file_name,
            day,
            product,
            entry,
            maps,
        });
    }
    Ok(captures)
}

fn recorded_str<'entry>(entry: &'entry Value, field: &str) -> Result<&'entry str, String> {
    entry
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("a manifest entry records no {field}"))
}

/// No entry survives a dropped capture, and no file is captured unrecorded.
#[test]
fn the_manifest_lists_exactly_the_captured_files() {
    let captured: BTreeSet<String> = support::cddis_capture_file_names()
        .unwrap()
        .into_iter()
        .collect();
    let recorded: BTreeSet<String> = captured_files()
        .unwrap()
        .into_iter()
        .map(|capture| capture.file_name)
        .collect();

    assert!(
        !captured.is_empty(),
        "no file is captured - run `just cddis-verify --capture`"
    );
    assert_eq!(captured, recorded);
}

/// The parse of each file agrees with what the archive served at capture time,
/// starting on the day the file was addressed for.
#[test]
fn every_capture_holds_the_day_and_maps_its_manifest_entry_records() {
    for capture in captured_files().unwrap() {
        let recorded = |field: &str| capture.entry.get(field).and_then(Value::as_u64);
        let maps = &capture.maps;
        assert_eq!(
            maps.epoch_of_first_map().map(|epoch| epoch.date_naive()),
            Some(capture.day),
            "{}",
            capture.file_name
        );
        assert_eq!(
            recorded("maps"),
            u64::try_from(maps.maps().len()).ok(),
            "{}",
            capture.file_name
        );
        assert_eq!(
            recorded("latitude_nodes"),
            u64::try_from(maps.grid().latitudes.node_count()).ok(),
            "{}",
            capture.file_name
        );
        assert_eq!(
            recorded("longitude_nodes"),
            u64::try_from(maps.grid().longitudes.node_count()).ok(),
            "{}",
            capture.file_name
        );
        assert_eq!(
            capture
                .entry
                .get("interval_seconds")
                .and_then(Value::as_i64),
            Some(maps.interval().num_seconds()),
            "{}",
            capture.file_name
        );
        let peak = maps
            .peak_total_electron_content()
            .map(TotalElectronContent::tecu);
        let recorded_peak = capture.entry.get("peak_tecu").and_then(Value::as_f64);
        assert!(
            peak.zip(recorded_peak)
                .is_some_and(|(peak, recorded)| (peak - recorded).abs() < TECU_TOLERANCE),
            "{}: {peak:?} TECU against the recorded {recorded_peak:?}",
            capture.file_name
        );
    }
}

/// A capture of a day the workspace holds a JPL file of must read as that
/// file: the archive files the same producer's day.
#[test]
fn a_capture_of_a_day_with_a_jpl_file_reads_as_that_file() {
    let mut compared = 0_usize;
    for capture in captured_files().unwrap() {
        let Some(fixture) = gt_ionex::declared_fixture_for_day(capture.product, capture.day) else {
            continue;
        };
        assert!(
            capture.maps == gt_ionex::captured_maps(fixture.name).unwrap(),
            "{}: the archive served other maps than JPL published in {}",
            capture.file_name,
            fixture.file_name
        );
        compared = compared.saturating_add(1);
    }
    assert!(
        compared > 0,
        "no capture covers a day the workspace holds a JPL file of"
    );
}
