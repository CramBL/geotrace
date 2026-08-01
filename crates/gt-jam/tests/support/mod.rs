//! Shared access to the captured fixtures and their manifest.

// Each test binary compiles this module independently and uses a different
// subset, so "unused" here only means "unused by this binary".
#![allow(dead_code, reason = "shared across binaries with different needs")]

use std::fs;

use serde_json::Value;

use gt_jam::{
    CAPTURE_MANIFEST, FIXTURE_DAYS, FixtureDay, dataset_file_name, fixtures_dir, parse_day,
};

/// The declared day the host served.
pub fn served_day() -> Result<&'static FixtureDay, String> {
    FIXTURE_DAYS
        .iter()
        .find(|fixture| fixture.is_served())
        .ok_or_else(|| "no served day is declared in FIXTURE_DAYS".to_owned())
}

/// The declared day the host refused.
pub fn refused_day() -> Result<&'static FixtureDay, String> {
    FIXTURE_DAYS
        .iter()
        .find(|fixture| !fixture.is_served())
        .ok_or_else(|| "no refused day is declared in FIXTURE_DAYS".to_owned())
}

pub fn manifest() -> Result<Value, String> {
    let path = fixtures_dir().join(CAPTURE_MANIFEST);
    let contents =
        fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))?;
    serde_json::from_str(&contents).map_err(|err| format!("{CAPTURE_MANIFEST}: {err}"))
}

pub fn manifest_entries() -> Result<Vec<Value>, String> {
    manifest()?
        .get("days")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{CAPTURE_MANIFEST} has no days array"))
}

pub fn manifest_entry(day: &str) -> Result<Value, String> {
    manifest_entries()?
        .into_iter()
        .find(|entry| entry.get("day").and_then(Value::as_str) == Some(day))
        .ok_or_else(|| format!("{day} has no manifest entry - run `just jam-fixtures {day}`"))
}

/// The dataset captured for `day`.
pub fn captured_csv(day: &str) -> Result<String, String> {
    let date = parse_day(day).map_err(|err| format!("{day} is not a calendar date: {err}"))?;
    let path = fixtures_dir().join(dataset_file_name(date));
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}
