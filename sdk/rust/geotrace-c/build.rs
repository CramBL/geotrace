//! Keep the C header's `GtdStatus` in lockstep with the `#[repr(C)]` enum in
//! `src/error.rs`. The two are hand-written parallel lists, so a mismatch (or a
//! forgotten header update) is a silent ABI break. Fail the build instead.
//!
//! The Rust discriminants are pinned by `const _` assertions in `src/error.rs`,
//! so checking the header against the same literals here locks both sides.

use std::error::Error;
use std::fs;

// (C macro name, ABI value). Must equal the `GtdStatus` discriminants in src/error.rs.
const STATUS_CODES: &[(&str, u32)] = &[
    ("GTD_OK", 0),
    ("GTD_ERR_NULL_ARGUMENT", 1),
    ("GTD_ERR_INVALID_PATH", 2),
    ("GTD_ERR_NO_NAV_FIXES", 3),
    ("GTD_ERR_ANNOTATIONS_OOB", 4),
    ("GTD_ERR_IO", 5),
    ("GTD_ERR_HDF5", 6),
    ("GTD_ERR_VERSION", 7),
    ("GTD_ERR_UTF8", 8),
    ("GTD_ERR_PARSE", 9),
    ("GTD_ERR_INTERNAL", 99),
];

const HEADER: &str = "../../c/geotrace.h";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={HEADER}");
    let header = fs::read_to_string(HEADER)?;
    for (name, value) in STATUS_CODES {
        let needle = format!("{name} = {value}");
        if !header.contains(&needle) {
            return Err(format!(
                "{HEADER}: GtdStatus out of sync with src/error.rs (expected `{needle}`)"
            )
            .into());
        }
    }
    Ok(())
}
