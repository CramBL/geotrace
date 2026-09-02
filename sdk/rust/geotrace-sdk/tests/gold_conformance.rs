//! Cross-SDK conformance. Given the shared gold-dataset fixtures, every SDK
//! (Rust, C, C++, Python) must produce a `.gtd` file that decodes to the same
//! `NavFile`. The byte layout may differ (HDF5 serializes logically-identical
//! content with different addressing). The decoded content must not.
//!
//! Only `gold.gtd` (the Rust output) is committed. `gold_c.gtd`, `gold_cpp.gtd`,
//! and `gold_py.gtd` are generated artifacts (gitignored), written by each
//! SDK's `gold_dataset` example. `just test-gold-all` regenerates all four and
//! then runs this test (`test-gold-compare`), so the comparison is enforced
//! there. It removes the three generated files once they match, and keeps them
//! for inspection when they do not. When the per-language files are absent (a
//! plain `cargo test` checkout) the missing ones are skipped.

use std::env;
use std::path::{Path, PathBuf};

use geotrace_sdk::NavFile;

#[expect(
    clippy::expect_used,
    reason = "cargo sets CARGO_MANIFEST_DIR for the test it runs"
)]
fn gold_dir() -> PathBuf {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    Path::new(&manifest_dir).join("../../../tests/fixtures/gold_dataset")
}

#[test]
fn all_sdks_decode_to_the_same_nav_file() {
    let dir = gold_dir();
    let canonical = NavFile::open(dir.join("gold.gtd")).unwrap();
    for name in ["gold_c.gtd", "gold_cpp.gtd", "gold_py.gtd"] {
        let path = dir.join(name);
        if !path.exists() {
            eprintln!("skipping {name}: not generated (run `just test-gold-all`)");
            continue;
        }
        let other = NavFile::open(&path).unwrap();
        assert!(
            canonical == other,
            "{name} decodes to a different NavFile than gold.gtd: cross-SDK drift. \
             Regenerate the gold fixtures with `just test-gold-all`."
        );
    }
}
