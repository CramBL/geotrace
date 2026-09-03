//! Reads the commit hash and the committer timestamp the release pipeline wrote
//! to `build_provenance.txt` into `GEOTRACE_SDK_GIT_COMMIT` and
//! `GEOTRACE_SDK_COMMIT_TIME`. A build without that file sets neither.

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

include!("build_script/provenance_file.rs");

const PROVENANCE_FILE: &str = "build_provenance.txt";

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join(PROVENANCE_FILE);
    if !path.is_file() {
        return Ok(());
    }
    println!("cargo::rerun-if-changed={PROVENANCE_FILE}");
    println!("cargo::rerun-if-changed=build_script/provenance_file.rs");

    let contents = fs::read_to_string(&path)?;
    let (commit, commit_time) =
        parse_provenance_file(&contents).map_err(|err| format!("{PROVENANCE_FILE} {err}"))?;

    println!("cargo::rustc-env=GEOTRACE_SDK_GIT_COMMIT={commit}");
    println!("cargo::rustc-env=GEOTRACE_SDK_COMMIT_TIME={commit_time}");

    Ok(())
}
