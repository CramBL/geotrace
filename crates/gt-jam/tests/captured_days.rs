//! Validate the committed dataset captures.
//!
//! Guards [`gt_jam::FIXTURE_DAYS`], the capture harness
//! (`examples/fetch_jam_fixtures.rs`), and the files under `tests/fixtures/`
//! against each other, and checks the captured day is still the shape the
//! parser is written for.

mod support;

use std::collections::BTreeSet;

use serde_json::Value;

use gt_jam::wire::{self, HexObservation, ParseWarningReporter};
use gt_jam::{FIXTURE_DAYS, FixtureDay, dataset_file_name, fixtures_dir, parse_day};

/// Floor for a world day, so a truncated re-capture fails here.
const MIN_WORLD_DAY_CELLS: usize = 40_000;

/// The days with a dataset on disk.
fn served_days() -> impl Iterator<Item = &'static FixtureDay> {
    FIXTURE_DAYS.iter().filter(|fixture| fixture.is_served())
}

/// The days that exist only in the capture manifest.
fn refused_days() -> impl Iterator<Item = &'static FixtureDay> {
    FIXTURE_DAYS.iter().filter(|fixture| !fixture.is_served())
}

/// Parse a captured day from disk.
fn parse_captured(day: &str) -> Result<(Vec<HexObservation>, ParseWarningReporter), String> {
    let csv = support::captured_csv(day)?;
    let reporter = ParseWarningReporter::default();
    let observations =
        wire::parse_dataset(&csv, &reporter).map_err(|err| format!("{day}: {err}"))?;
    Ok((observations, reporter))
}

/// The manifest agrees with the status each day declares.
#[test]
fn every_declared_day_has_a_matching_manifest_entry() {
    for fixture in FIXTURE_DAYS {
        let entry = support::manifest_entry(fixture.day).unwrap();
        assert_eq!(
            entry.get("http_status").and_then(Value::as_u64),
            Some(u64::from(fixture.http_status)),
            "{}: the capture recorded a different status than FIXTURE_DAYS declares",
            fixture.day
        );
        assert!(
            entry
                .get("captured_at")
                .and_then(Value::as_str)
                .is_some_and(|captured_at| !captured_at.is_empty()),
            "{} has no capture date",
            fixture.day
        );
    }
}

/// No entry survives a dropped day, and no day is captured undeclared.
#[test]
fn the_manifest_lists_exactly_the_declared_days() {
    let declared: BTreeSet<&str> = FIXTURE_DAYS.iter().map(|fixture| fixture.day).collect();
    let recorded: Vec<String> = support::manifest_entries()
        .unwrap()
        .iter()
        .filter_map(|entry| Some(entry.get("day")?.as_str()?.to_owned()))
        .collect();
    let recorded: BTreeSet<&str> = recorded.iter().map(String::as_str).collect();
    assert_eq!(declared, recorded);
}

/// A served day has its dataset on disk. A refused one has no file.
#[test]
fn only_served_days_have_a_dataset_on_disk() {
    for fixture in FIXTURE_DAYS {
        let day = parse_day(fixture.day).unwrap();
        let path = fixtures_dir().join(dataset_file_name(day));
        assert_eq!(
            path.exists(),
            fixture.is_served(),
            "{}: {}",
            fixture.day,
            fixture.purpose
        );
    }
}

/// The refusal body is kept, so the transport is tested against a real
/// refusal.
#[test]
fn a_refused_day_records_the_hosts_answer() {
    for fixture in refused_days() {
        let entry = support::manifest_entry(fixture.day).unwrap();
        assert!(
            entry
                .get("body")
                .and_then(Value::as_str)
                .is_some_and(|body| !body.is_empty()),
            "{} recorded no refusal body",
            fixture.day
        );
        assert_eq!(
            entry.get("rows").and_then(Value::as_u64),
            None,
            "{}: a refused day has no rows",
            fixture.day
        );
    }
}

/// The captured day parses with no warnings and the recorded cell count.
#[test]
fn the_captured_world_day_parses_cleanly() {
    for fixture in served_days() {
        let (observations, reporter) = parse_captured(fixture.day).unwrap();

        assert!(
            reporter.is_empty(),
            "{}: the host's own file has unusable rows: {:#?}",
            fixture.day,
            reporter.warnings()
        );
        assert!(
            observations.len() > MIN_WORLD_DAY_CELLS,
            "{}: {} cells is too few for a world day - was the capture truncated?",
            fixture.day,
            observations.len()
        );
        assert_eq!(
            observations.len(),
            support::manifest_entry(fixture.day)
                .unwrap()
                .get("rows")
                .and_then(Value::as_u64)
                .and_then(|rows| usize::try_from(rows).ok())
                .expect("a served day records its row count"),
            "{}: the file on disk is not the one the manifest describes",
            fixture.day
        );
    }
}

/// Every cell counted at least one aircraft, and its share is in 0..=1.
#[test]
fn the_captured_world_day_carries_usable_tallies() {
    for fixture in served_days() {
        let (observations, _) = parse_captured(fixture.day).unwrap();
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
