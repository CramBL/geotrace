//! Shared access to the captured fixtures and their manifest.

// Each test binary compiles this module independently and uses a different
// subset, so "unused" here only means "unused by this binary".
#![allow(dead_code, reason = "shared across binaries with different needs")]

use std::fs;

use serde_json::Value;

use gt_solar::{CAPTURE_MANIFEST, FIXTURE_WINDOWS, FixtureWindow, fixtures_dir};

/// The declared window named `name`.
pub fn declared_window(name: &str) -> Result<&'static FixtureWindow, String> {
    FIXTURE_WINDOWS
        .iter()
        .find(|fixture| fixture.name == name)
        .ok_or_else(|| format!("{name} is not declared in FIXTURE_WINDOWS"))
}

pub fn manifest() -> Result<Value, String> {
    let path = fixtures_dir().join(CAPTURE_MANIFEST);
    let contents =
        fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))?;
    serde_json::from_str(&contents).map_err(|err| format!("{CAPTURE_MANIFEST}: {err}"))
}

pub fn manifest_entries() -> Result<Vec<Value>, String> {
    manifest()?
        .get("windows")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{CAPTURE_MANIFEST} has no windows array"))
}

pub fn manifest_entry(name: &str) -> Result<Value, String> {
    manifest_entries()?
        .into_iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| format!("{name} has no manifest entry - run `just solar-fixtures {name}`"))
}

/// The response captured for `fixture`.
pub fn captured_response(fixture: &FixtureWindow) -> Result<String, String> {
    let path = fixtures_dir().join(fixture.file_name());
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}
