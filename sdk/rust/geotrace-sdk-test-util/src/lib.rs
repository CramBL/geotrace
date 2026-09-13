//! Test helpers shared by the integration test binaries of geotrace-sdk, which
//! import the crate as `test_util`.

use std::path::{Path, PathBuf};

use geotrace_sdk::{
    Angle, DateTime, Duration, Error, NavFile, NavFileBuilder, NavFix, NavFixTime, NavRecorder, Utc,
};

/// A recorder with the one fix `fix_at(0, 55.0, 12.0)`.
pub fn recorder_with_one_fix() -> NavRecorder {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(0, 55.0, 12.0));
    recorder
}

/// A fix at `(lat, lon)` heading north, with the receiver time
/// `t_ms(offset_ms)`.
pub fn fix_at(offset_ms: i64, lat: f64, lon: f64) -> NavFix {
    NavFix::builder()
        .time(NavFixTime::Receiver(t_ms(offset_ms)))
        .lat(Angle::degrees(lat))
        .lon(Angle::degrees(lon))
        .heading(Angle::degrees(0.0))
        .build()
}

/// [`base`] plus `offset_ms` milliseconds.
pub fn t_ms(offset_ms: i64) -> DateTime<Utc> {
    base() + Duration::milliseconds(offset_ms)
}

/// [`base`] plus `offset_secs` seconds.
pub fn t_s(offset_secs: i64) -> DateTime<Utc> {
    base() + Duration::seconds(offset_secs)
}

/// 2025-05-23 11:33:20 UTC, the instant [`t_ms`] and [`t_s`] count from.
#[expect(clippy::expect_used, reason = "the fixed timestamp is in range")]
pub fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("in range")
}

/// `nav_file` written to bytes and read back.
pub fn round_trip(nav_file: &NavFile) -> Result<NavFile, Error> {
    NavFile::read(to_bytes(nav_file)?.as_slice())
}

/// The bytes the writer writes for `nav_file`.
pub fn to_bytes(nav_file: &NavFile) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes)?;
    Ok(bytes)
}

/// The path of `relative_path` under the repository's `tests/fixtures/`.
pub fn fixture_path(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures")
        .join(relative_path)
}
