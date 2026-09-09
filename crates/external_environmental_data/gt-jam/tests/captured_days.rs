//! Validate the committed dataset captures.
//!
//! Guards [`gt_jam::CAPTURED_DAYS`], the capture harness
//! (`examples/fetch_jam_captures.rs`), and the files under `tests/captures/`
//! against each other, and checks the captured day is still the shape the
//! parser is written for.

use std::collections::BTreeSet;

use serde_json::Value;

use gt_jam::test_util;
use gt_jam::wire::{self, HexObservation, ParseWarningReporter};
use gt_jam::{CAPTURED_DAYS, CapturedDay};

/// Floor for a world day, so a truncated re-capture fails here.
const MIN_WORLD_DAY_CELLS: usize = 40_000;

/// The days with a dataset on disk.
fn served_days() -> impl Iterator<Item = &'static CapturedDay> {
    CAPTURED_DAYS.iter().filter(|capture| capture.is_served())
}

/// The days that exist only in the capture manifest.
fn refused_days() -> impl Iterator<Item = &'static CapturedDay> {
    CAPTURED_DAYS.iter().filter(|capture| !capture.is_served())
}

/// Parse a captured day from disk.
fn parse_captured(day: &str) -> Result<(Vec<HexObservation>, ParseWarningReporter), String> {
    let csv = test_util::captured_csv(day)?;
    let reporter = ParseWarningReporter::default();
    let observations =
        wire::parse_dataset(&csv, &reporter).map_err(|err| format!("{day}: {err}"))?;
    Ok((observations, reporter))
}

/// The manifest agrees with the status each day declares.
#[test]
fn every_declared_day_has_a_matching_manifest_entry() {
    for capture in CAPTURED_DAYS {
        let entry = test_util::manifest_entry(capture.day).unwrap();
        assert_eq!(
            entry.get("http_status").and_then(Value::as_u64),
            Some(u64::from(capture.http_status)),
            "{}: the capture recorded a different status than CAPTURED_DAYS declares",
            capture.day
        );
        assert!(
            entry
                .get("captured_at")
                .and_then(Value::as_str)
                .is_some_and(|captured_at| !captured_at.is_empty()),
            "{} has no capture date",
            capture.day
        );
    }
}

/// No entry survives a dropped day, and no day is captured undeclared.
#[test]
fn the_manifest_lists_exactly_the_declared_days() {
    let declared: BTreeSet<&str> = CAPTURED_DAYS.iter().map(|capture| capture.day).collect();
    let recorded: Vec<String> = test_util::manifest_entries()
        .unwrap()
        .iter()
        .filter_map(|entry| Some(entry.get("day")?.as_str()?.to_owned()))
        .collect();
    let recorded: BTreeSet<&str> = recorded.iter().map(String::as_str).collect();
    assert_eq!(declared, recorded);
}

/// A served day has its dataset on disk. A day the host refused has no file.
#[test]
fn only_served_days_have_a_dataset_on_disk() {
    for capture in CAPTURED_DAYS {
        let day = gt_jam::parse_day(capture.day).unwrap();
        let path = gt_jam::captures_dir().join(gt_jam::dataset_file_name(day));
        assert_eq!(
            path.exists(),
            capture.is_served(),
            "{}: {}",
            capture.day,
            capture.purpose
        );
    }
}

/// The body of the refusal from the host is kept, so the transport is tested
/// against a real one.
#[test]
fn a_refused_day_records_the_hosts_response() {
    for capture in refused_days() {
        let entry = test_util::manifest_entry(capture.day).unwrap();
        assert!(
            entry
                .get("body")
                .and_then(Value::as_str)
                .is_some_and(|body| !body.is_empty()),
            "{} recorded no refusal body",
            capture.day
        );
        assert_eq!(
            entry.get("rows").and_then(Value::as_u64),
            None,
            "{}: a refused day has no rows",
            capture.day
        );
    }
}

/// The captured day parses with no warnings and the recorded cell count.
#[test]
fn the_captured_world_day_parses_cleanly() {
    for capture in served_days() {
        let (observations, reporter) = parse_captured(capture.day).unwrap();

        assert!(
            reporter.is_empty(),
            "{}: the host's own file has unusable rows: {:#?}",
            capture.day,
            reporter.warnings()
        );
        assert!(
            observations.len() > MIN_WORLD_DAY_CELLS,
            "{}: {} cells is too few for a world day - was the capture truncated?",
            capture.day,
            observations.len()
        );
        assert_eq!(
            observations.len(),
            test_util::manifest_entry(capture.day)
                .unwrap()
                .get("rows")
                .and_then(Value::as_u64)
                .and_then(|rows| usize::try_from(rows).ok())
                .expect("a served day records its row count"),
            "{}: the file on disk is not the one the manifest describes",
            capture.day
        );
    }
}

/// Every cell counted at least one aircraft, and its share is in 0..=1.
#[test]
fn the_captured_world_day_carries_usable_tallies() {
    for capture in served_days() {
        let (observations, _) = parse_captured(capture.day).unwrap();
        for observation in &observations {
            let rate = observation
                .rate()
                .expect("a parsed observation always counted at least one aircraft");
            assert!(
                (0.0..=1.0).contains(&rate.bad_fraction),
                "{observation:?} produced a share outside 0..=1"
            );
            assert_eq!(rate.aircraft, observation.aircraft());
        }
    }
}
