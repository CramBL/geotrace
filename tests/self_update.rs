//! End-to-end check of the headless self-updater.
//!
//! The GUI drives `axoupdater` through `run_self_update`, reachable from the
//! command line as `geotrace --update` (see `main.rs`). This test installs the
//! built binary into a throwaway directory with a fake "very old" install
//! receipt, runs `--update`, and asserts axoupdater downloaded the latest
//! release and replaced the binary in place. It is the same shape as
//! axoupdater's own `perform_runtest`, but sandboxed to temporary
//! directories so it never touches `~/.cargo` or the real receipt.
//!
//! Gated two ways so it never runs on an ordinary `cargo test`:
//! - the `self-update` feature (a `required-features` test target in
//!   `Cargo.toml`), so the `--update` flag exists in the binary under test.
//! - the `GEOTRACE_RUN_UPDATE_TEST` environment variable, because it reaches
//!   out to GitHub and mutates a file. CI sets both (see `ci_self_update.yml`).

use std::{fs, process::Command};

use tempfile::tempdir;

/// Must match the package/binary name and the repository `axoupdater` queries.
const APP: &str = "geotrace";
const OWNER: &str = "CramBL";

/// Environment variable that opts a run in to actually performing the network update.
const RUN_VAR: &str = "GEOTRACE_RUN_UPDATE_TEST";

#[test]
fn headless_update_replaces_the_binary() {
    if std::env::var_os(RUN_VAR).is_none() {
        eprintln!("skipping: set {RUN_VAR}=1 to run (downloads the latest release)");
        return;
    }

    let bin_dir = tempdir().expect("temp bin dir");
    let config_dir = tempdir().expect("temp config dir");

    // Install the freshly built binary into the sandbox.
    let installed = bin_dir.path().join(APP);
    fs::copy(env!("CARGO_BIN_EXE_geotrace"), &installed).expect("copy binary");
    let before = fs::read(&installed).expect("read installed binary");

    // A cargo-dist install receipt claiming version 0.0.1, so axoupdater always
    // considers an update necessary. Fields mirror what the real installer
    // writes. `install_prefix` is the sandbox so the replace stays contained.
    let prefix = bin_dir.path().display().to_string().replace('\\', "\\\\");
    let receipt = format!(
        r#"{{"binaries":["{APP}"],"install_prefix":"{prefix}","provider":{{"source":"cargo-dist","version":"0.10.0"}},"source":{{"app_name":"{APP}","name":"{APP}","owner":"{OWNER}","release_type":"github"}},"version":"0.0.1"}}"#
    );
    fs::write(
        config_dir.path().join(format!("{APP}-receipt.json")),
        receipt,
    )
    .expect("write receipt");

    // Point axoupdater at the sandboxed receipt and run the real `--update`.
    let output = Command::new(&installed)
        .arg("--update")
        .env("AXOUPDATER_CONFIG_PATH", config_dir.path())
        .output()
        .expect("run --update");

    assert!(
        output.status.success(),
        "--update failed ({})\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // A successful update replaced the debug binary with the released one, so
    // the bytes on disk must have changed.
    let after = fs::read(&installed).expect("read updated binary");
    assert_ne!(before, after, "binary was not replaced by the update");
}
