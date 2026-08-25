//! The `.gtd` decoder must never panic on malformed input: every byte string,
//! valid or not, must yield `Ok` or `Err`, never a panic or an abort. This is
//! the always-on, stable-toolchain counterpart to the cargo-fuzz `decode`
//! target under `fuzz/` (same entry point, `NavFile::read`).

use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};

use geotrace_sdk::{Angle, DateTime, Duration, Error, NavFile, NavFileBuilder, NavFix, Utc};

/// A fuzz-found crash input, a mutated gold fixture whose `nav_points/time`
/// dataspace declares 717 259 538 631 elements. Reading it as `i64` would
/// allocate 5.7 TB from those 7 602 bytes.
fn implausible_extent_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/implausible_nav_points_time_extent.gtd")
}

/// The committed gold fixture, the same seed the cargo-fuzz workflow uses.
fn gold_bytes() -> std::io::Result<Vec<u8>> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/gold_dataset/gold.gtd");
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

/// `NavFile::inspect` reads through its own set of functions and takes a path.
/// The same bytes are swept over it from a temporary file.
fn inspect(bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    // Only the absence of a panic matters. The rendered result is irrelevant.
    let _result = NavFile::inspect(tmp.path());
    Ok(())
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

#[test]
fn implausible_declared_extent_is_refused_when_reading() {
    let bytes = std::fs::read(implausible_extent_path()).unwrap();
    let err = NavFile::read(Cursor::new(bytes)).unwrap_err();
    assert!(
        matches!(err, Error::ImplausibleDatasetSize { .. }),
        "expected a declared-size refusal, got {err:#}"
    );
}

#[test]
fn implausible_declared_extent_is_refused_when_inspecting() {
    let err = NavFile::inspect(implausible_extent_path()).unwrap_err();
    assert!(
        matches!(err, Error::ImplausibleDatasetSize { .. }),
        "expected a declared-size refusal, got {err:#}"
    );
}

/// The declared-size bound scales with the file's own byte length. A long
/// recording of constant values stresses it hardest: deflate shrinks constant
/// data furthest, leaving the fewest file bytes to back the declared extent.
#[test]
fn long_recording_of_constant_values_still_decodes() {
    const POINTS: i64 = 100_000;

    let t = "2024-06-01T08:00:00Z".parse::<DateTime<Utc>>().unwrap();
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..POINTS {
        recorder.add(
            NavFix::builder()
                .gps_time(t + Duration::seconds(i))
                .lat(Angle::degrees(51.5074))
                .lon(Angle::degrees(-0.1278))
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();

    let decoded = NavFile::read(Cursor::new(bytes)).unwrap();
    assert_eq!(decoded.nav_points().len(), POINTS as usize);
}

#[test]
fn inspecting_garbage_truncations_and_mutations_never_panics() -> std::io::Result<()> {
    inspect(&[])?;
    inspect(&[0u8; 64])?;
    inspect(b"definitely not an hdf5 file")?;

    let gold = gold_bytes()?;
    for len in (0..gold.len()).step_by(101) {
        inspect(&gold[..len])?;
    }
    for i in (0..gold.len()).step_by(97) {
        let mut mutated = gold.clone();
        mutated[i] ^= 0xff;
        inspect(&mutated)?;
    }
    Ok(())
}
