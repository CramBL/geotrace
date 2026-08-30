//! The `.gtd` decoder must never panic on malformed input: every byte string,
//! valid or not, must yield `Ok` or `Err`, never a panic or an abort. This is
//! the always-on, stable-toolchain counterpart to the cargo-fuzz `decode`
//! target under `fuzz/` (same entry point, `NavFile::read`).

use std::io::Cursor;
use std::path::Path;

use geotrace_sdk::{Angle, DateTime, NavFile, NavFileBuilder, NavFix, Utc};

/// The committed gold fixture, the same seed the cargo-fuzz workflow uses.
fn gold_bytes() -> std::io::Result<Vec<u8>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(std::io::Error::other)?;
    let path = Path::new(&manifest_dir).join("../../../tests/fixtures/gold_dataset/gold.gtd");
    std::fs::read(path)
}

fn valid_gtd_bytes() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let t = "2024-06-01T08:00:00Z".parse::<DateTime<Utc>>()?;
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(
        NavFix::builder()
            .gps_time(t)
            .lat(Angle::degrees(51.5074))
            .lon(Angle::degrees(-0.1278))
            .build(),
    );
    let nav_file = recorder.finish()?;
    let mut buf = Vec::new();
    nav_file.write(&mut buf)?;
    Ok(buf)
}

fn read(bytes: Vec<u8>) {
    // Only the absence of a panic matters. The decoded result is irrelevant.
    let _result = NavFile::read(Cursor::new(bytes));
}

#[test]
fn arbitrary_garbage_never_panics() {
    read(Vec::new());
    read(vec![0u8; 64]);
    read(vec![0xffu8; 1024]);
    read(b"\x89HDF\r\n\x1a\n".to_vec()); // HDF5 signature, nothing after it
    read(b"definitely not an hdf5 file".to_vec());
}

#[test]
fn truncated_valid_file_never_panics() {
    let valid = valid_gtd_bytes().unwrap();
    for len in 0..valid.len() {
        read(valid[..len].to_vec());
    }
}

#[test]
fn single_byte_mutations_never_panic() {
    let valid = valid_gtd_bytes().unwrap();
    for i in (0..valid.len()).step_by(7) {
        let mut mutated = valid.clone();
        mutated[i] ^= 0xff;
        read(mutated);
    }
}

/// Sweep truncations and byte flips over the real gold fixture, so the stable
/// test covers the same corpus the fuzz workflow seeds from.
#[test]
fn gold_corpus_truncations_and_mutations_never_panic() {
    let gold = gold_bytes().unwrap();
    for len in (0..gold.len()).step_by(101) {
        read(gold[..len].to_vec());
    }
    for i in (0..gold.len()).step_by(97) {
        let mut mutated = gold.clone();
        mutated[i] ^= 0xff;
        read(mutated);
    }
}
