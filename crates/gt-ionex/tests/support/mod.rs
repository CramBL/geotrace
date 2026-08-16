//! Shared access to the captured files and their manifest.

// Each test binary compiles this module independently and uses a different
// subset, so "unused" here only means "unused by this binary".
#![allow(dead_code, reason = "shared across binaries with different needs")]

use std::fs;

use serde_json::Value;

use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::{CAPTURE_MANIFEST, FIXTURE_FILES, FixtureFile, fixtures_dir, parse};

/// The May 2024 storm day.
pub const STORM_CAPTURE: &str = "jpl-final-storm";

/// The geomagnetically quiet day on the same grid.
pub const QUIET_CAPTURE: &str = "jpl-final-quiet";

pub fn declared_fixture(name: &str) -> Result<&'static FixtureFile, String> {
    FIXTURE_FILES
        .iter()
        .find(|fixture| fixture.name == name)
        .ok_or_else(|| format!("{name} is not declared in FIXTURE_FILES"))
}

pub fn captured_text(fixture: &FixtureFile) -> Result<String, String> {
    let path = fixtures_dir().join(fixture.file_name);
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

pub fn captured_maps(name: &str) -> Result<GlobalIonosphereMaps, String> {
    let fixture = declared_fixture(name)?;
    parse::global_ionosphere_maps(&captured_text(fixture)?)
        .map_err(|err| format!("{}: {err}", fixture.name))
}

pub fn manifest() -> Result<Value, String> {
    let path = fixtures_dir().join(CAPTURE_MANIFEST);
    let contents =
        fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))?;
    serde_json::from_str(&contents).map_err(|err| format!("{CAPTURE_MANIFEST}: {err}"))
}

pub fn manifest_entries() -> Result<Vec<Value>, String> {
    manifest()?
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{CAPTURE_MANIFEST} has no files array"))
}

pub fn manifest_entry(name: &str) -> Result<Value, String> {
    manifest_entries()?
        .into_iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| format!("{name} has no manifest entry - run `just ionex-fixtures {name}`"))
}
