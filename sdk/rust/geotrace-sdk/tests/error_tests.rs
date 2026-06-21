use geotrace_sdk::{Angle, DateTime, Duration, NavFile, NavFileBuilder, NavFix, Utc};
use hdf5_pure::{AttrValue, FileBuilder};

fn base() -> DateTime<Utc> {
    #[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

fn minimal_gtd_bytes() -> Vec<u8> {
    let t = base();
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t)
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .heading(Angle::degrees(90.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t + Duration::seconds(60))
            .lat(Angle::degrees(51.6))
            .lon(Angle::degrees(-0.2))
            .heading(Angle::degrees(270.0))
            .build(),
    );
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    #[expect(clippy::expect_used, reason = "test setup must succeed")]
    nav_file.write(&mut bytes).expect("write");
    bytes
}

#[test]
fn read_non_hdf5_bytes_returns_error() {
    let garbage = b"this is not an HDF5 file at all";
    let result = NavFile::read(garbage.as_slice());
    assert!(
        result.is_err(),
        "expected an error reading non-HDF5 bytes, got Ok"
    );
}

#[test]
fn read_empty_bytes_returns_error() {
    let result = NavFile::read([].as_slice());
    assert!(result.is_err(), "expected an error reading empty bytes");
}

#[test]
fn read_truncated_hdf5_magic_returns_error() {
    // HDF5 magic is "\x89HDF\r\n\x1a\n" - truncate after 4 bytes.
    let truncated = b"\x89HDF";
    let result = NavFile::read(truncated.as_slice());
    assert!(
        result.is_err(),
        "expected an error reading truncated HDF5 header"
    );
}

#[test]
fn valid_gtd_round_trips_without_error() {
    let bytes = minimal_gtd_bytes();
    let nav_file = NavFile::read(bytes.as_slice()).expect("valid .gtd bytes must parse");
    assert_eq!(nav_file.nav_points().len(), 2);
}

/// A file that is valid HDF5 but has no `geotrace_version` attribute should fail.
#[test]
fn missing_version_attribute_returns_error() {
    let mut fb = FileBuilder::new();
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("lat").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("lon").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("heading")
        .with_f64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[])
        .with_shape(&[0]);
    fb.add_group(np.finish());
    let bytes = fb.finish().expect("build");

    let result = NavFile::read(bytes.as_slice());
    assert!(
        result.is_err(),
        "expected an error reading a file without geotrace_version"
    );
}

/// A file that uses an unrecognised version string should return UnsupportedVersion.
#[test]
fn unrecognised_version_string_returns_unsupported_version() {
    use geotrace_sdk::Error;

    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("99".into()));
    let mut np = fb.create_group("nav_points");
    np.create_dataset("time")
        .with_i64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("lat").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("lon").with_f64_data(&[]).with_shape(&[0]);
    np.create_dataset("heading")
        .with_f64_data(&[])
        .with_shape(&[0]);
    np.create_dataset("speed_mps")
        .with_f64_data(&[])
        .with_shape(&[0]);
    fb.add_group(np.finish());
    let bytes = fb.finish().expect("build");

    let result = NavFile::read(bytes.as_slice());
    assert!(
        matches!(result, Err(Error::UnsupportedVersion { ref version }) if version == "99"),
        "expected UnsupportedVersion(\"99\"), got: {result:?}"
    );
}
