//! Readers for the captured responses under `tests/captures/` and their
//! manifest.
//!
//! The integration test binaries reach these as `gt_flare::test_util`, through
//! the `test-util` feature gt-flare's dev-dependency on itself enables.

use std::fs;

use serde_json::Value;

use crate::{CAPTURE_MANIFEST, CAPTURED_WINDOWS, CapturedWindow};

/// The declared window named `name`.
pub fn declared_window(name: &str) -> Result<&'static CapturedWindow, String> {
    CAPTURED_WINDOWS
        .iter()
        .find(|capture| capture.name == name)
        .ok_or_else(|| format!("{name} is not declared in CAPTURED_WINDOWS"))
}

pub fn manifest() -> Result<Value, String> {
    let path = crate::captures_dir().join(CAPTURE_MANIFEST);
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
        .ok_or_else(|| format!("{name} has no manifest entry - run `just flare-captures {name}`"))
}

/// The response the endpoint returned when `capture` was recorded.
pub fn captured_response(capture: &CapturedWindow) -> Result<String, String> {
    let path = crate::captures_dir().join(capture.file_name());
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}
