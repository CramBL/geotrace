//! Access to the captured files and their manifest, for the gt-ionex tests.
//!
//! The integration test binaries reach it as `gt_ionex::test_util`, through the
//! `test-util` feature gt-ionex's dev-dependency on itself enables.

use std::path::{Path, PathBuf};
use std::{fs, io};

use serde_json::Value;

use crate::maps::GlobalIonosphereMaps;
use crate::{CAPTURE_MANIFEST, CaptureError, CapturedFile};

pub fn declared_capture(name: &str) -> Result<&'static CapturedFile, String> {
    crate::declared_capture(name).ok_or_else(|| {
        CaptureError::Undeclared {
            name: name.to_owned(),
        }
        .to_string()
    })
}

pub fn captured_text(capture: &CapturedFile) -> Result<String, String> {
    crate::captured_text(capture).map_err(|error| error.to_string())
}

pub fn captured_maps(name: &str) -> Result<GlobalIonosphereMaps, String> {
    crate::captured_maps(name).map_err(|error| error.to_string())
}

/// One of the `.Z` streams `just qa::generate-unix-compress-fixtures`
/// constructs under `tests/fixtures/unix_compress/`.
pub fn compressed_fixture(name: &str) -> Result<Vec<u8>, String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("unix_compress")
        .join(name);
    fs::read(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

/// The bytes [`COMPRESSED_CAPTURE`] holds, which the streams decode to.
pub fn compressed_capture_bytes() -> Result<Vec<u8>, String> {
    let path = crate::captures_dir().join(COMPRESSED_CAPTURE);
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
    manifest_entries_in(&crate::captures_dir())
}

pub fn manifest_entry(name: &str) -> Result<Value, String> {
    manifest_entries()?
        .into_iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| format!("{name} has no manifest entry - run `just ionex-captures {name}`"))
}

/// What `just cddis-verify --capture` recorded about the files the archive
/// served.
pub fn cddis_manifest_entries() -> Result<Vec<Value>, String> {
    manifest_entries_in(&crate::cddis_captures_dir())
}

/// The files the archive served, as they arrived: still compressed, under the
/// name they were requested under.
pub fn cddis_capture_file_names() -> Result<Vec<String>, String> {
    let directory = crate::cddis_captures_dir();
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
    let path = crate::cddis_captures_dir().join(file_name);
    fs::read(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

/// The capture the generated streams hold, and how much of it the partial
/// ones do, declared the same way on the generator's side.
pub const COMPRESSED_CAPTURE: &str = "JPLG0920.24I";
pub const COMPRESSED_HEAD_BYTES: usize = 65_536;
