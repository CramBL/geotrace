//! Readers for the captured datasets under `tests/captures/` and their
//! manifest.
//!
//! The integration test binaries reach these as `gt_jam::test_util`, through
//! the `test-util` feature gt-jam's dev-dependency on itself enables.

use std::fs;

use serde_json::Value;

use crate::{CAPTURE_MANIFEST, CAPTURED_DAYS, CapturedDay};

/// The declared day the host served.
pub fn served_day() -> Result<&'static CapturedDay, String> {
    CAPTURED_DAYS
        .iter()
        .find(|capture| capture.is_served())
        .ok_or_else(|| "no served day is declared in CAPTURED_DAYS".to_owned())
}

/// The declared day the host refused.
pub fn refused_day() -> Result<&'static CapturedDay, String> {
    CAPTURED_DAYS
        .iter()
        .find(|capture| !capture.is_served())
        .ok_or_else(|| "no refused day is declared in CAPTURED_DAYS".to_owned())
}

pub fn manifest() -> Result<Value, String> {
    let path = crate::captures_dir().join(CAPTURE_MANIFEST);
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
        .ok_or_else(|| format!("{day} has no manifest entry - run `just jam-captures {day}`"))
}

/// The dataset captured for `day`.
pub fn captured_csv(day: &str) -> Result<String, String> {
    let date =
        crate::parse_day(day).map_err(|err| format!("{day} is not a calendar date: {err}"))?;
    let path = crate::captures_dir().join(crate::dataset_file_name(date));
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}
