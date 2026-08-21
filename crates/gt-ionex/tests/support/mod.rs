//! Shared access to the captured files and their manifest.

// Each test binary compiles this module independently and uses a different
// subset, so "unused" here only means "unused by this binary".
#![allow(dead_code, reason = "shared across binaries with different needs")]

use std::path::Path;
use std::{fs, io};

use serde_json::Value;

use gt_ionex::maps::GlobalIonosphereMaps;
use gt_ionex::{CAPTURE_MANIFEST, CaptureError, FixtureFile, cddis_fixtures_dir, fixtures_dir};

pub fn declared_fixture(name: &str) -> Result<&'static FixtureFile, String> {
    gt_ionex::declared_fixture(name).ok_or_else(|| {
        CaptureError::Undeclared {
            name: name.to_owned(),
        }
        .to_string()
    })
}

pub fn captured_text(fixture: &FixtureFile) -> Result<String, String> {
    gt_ionex::captured_text(fixture).map_err(|error| error.to_string())
}

pub fn captured_maps(name: &str) -> Result<GlobalIonosphereMaps, String> {
    gt_ionex::captured_maps(name).map_err(|error| error.to_string())
}

/// Directory of the streams `just qa::generate-unix-compress-fixtures` writes.
const COMPRESSED_DIR: &str = "unix_compress";

/// The capture those streams hold, and how much of it the partial ones do,
/// declared the same way on the generator's side.
pub const COMPRESSED_CAPTURE: &str = "JPLG0920.24I";
pub const COMPRESSED_HEAD_BYTES: usize = 65_536;

pub fn compressed_fixture(name: &str) -> Result<Vec<u8>, String> {
    let path = fixtures_dir().join(COMPRESSED_DIR).join(name);
    fs::read(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

/// The bytes [`COMPRESSED_CAPTURE`] holds, which the streams decode to.
pub fn compressed_capture_bytes() -> Result<Vec<u8>, String> {
    let path = fixtures_dir().join(COMPRESSED_CAPTURE);
    fs::read(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

/// The entries of the manifest `directory` holds, which the capture tools all
/// write in the same shape.
fn manifest_entries_in(directory: &Path) -> Result<Vec<Value>, String> {
    let path = directory.join(CAPTURE_MANIFEST);
    let contents =
        fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))?;
    let manifest: Value =
        serde_json::from_str(&contents).map_err(|err| format!("{}: {err}", path.display()))?;
    manifest
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| format!("{} has no files array", path.display()))
}

pub fn manifest_entries() -> Result<Vec<Value>, String> {
    manifest_entries_in(&fixtures_dir())
}

pub fn manifest_entry(name: &str) -> Result<Value, String> {
    manifest_entries()?
        .into_iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| format!("{name} has no manifest entry - run `just ionex-fixtures {name}`"))
}

/// What `just cddis-verify --capture` recorded about the files the archive
/// served.
pub fn cddis_manifest_entries() -> Result<Vec<Value>, String> {
    manifest_entries_in(&cddis_fixtures_dir())
}

/// The files the archive served, as they arrived: still compressed, under the
/// name they were requested under.
pub fn cddis_capture_file_names() -> Result<Vec<String>, String> {
    let directory = cddis_fixtures_dir();
    let mut names: Vec<String> = fs::read_dir(&directory)
        .and_then(|entries| entries.collect::<Result<Vec<_>, io::Error>>())
        .map_err(|err| format!("reading {}: {err}", directory.display()))?
        .iter()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != CAPTURE_MANIFEST)
        .collect();
    names.sort();
    Ok(names)
}

pub fn cddis_capture_bytes(file_name: &str) -> Result<Vec<u8>, String> {
    let path = cddis_fixtures_dir().join(file_name);
    fs::read(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}
