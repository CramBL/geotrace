//! Resolving the manifest directory of the crate under test from the
//! environment at runtime. Reading it this way keeps a cached test binary's
//! fixture and asset paths matched to the checkout it runs in.

use std::env;
use std::path::PathBuf;

/// The manifest directory of the crate under test, read from the caller's
/// environment. Cargo sets `CARGO_MANIFEST_DIR` per invocation. `env!` would
/// name the crate this function is written in.
#[expect(
    clippy::expect_used,
    reason = "cargo sets CARGO_MANIFEST_DIR for every test it runs"
)]
pub fn cargo_manifest_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
}
