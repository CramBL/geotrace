#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use geotrace_sdk::{Angle, DateTime, NavFile, NavFileBuilder, NavFix, NavFixTime, Utc};
use hdf5_pure::{AttrValue, FileBuilder};

#[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid timestamp")
}

fn read_file_with_attrs(
    set_attrs: impl FnOnce(&mut FileBuilder),
) -> Result<NavFile, Box<dyn std::error::Error>> {
    let mut fb = FileBuilder::new();
    fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    set_attrs(&mut fb);

    let mut nav_points = fb.create_group("nav_points");
    nav_points
        .create_dataset("time")
        .with_i64_data(&[])
        .with_shape(&[0]);
    for name in ["lat", "lon", "heading", "speed_mps"] {
        nav_points
            .create_dataset(name)
            .with_f64_data(&[])
            .with_shape(&[0]);
    }
    fb.add_group(nav_points.finish());

    Ok(NavFile::read(fb.finish()?.as_slice())?)
}

#[test]
fn a_written_file_carries_the_sdk_version() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(base()))
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );

    let mut bytes = Vec::new();
    recorder.finish()?.write(&mut bytes)?;

    let nav_file = NavFile::read(bytes.as_slice())?;
    assert_eq!(nav_file.meta().sdk_version(), Some(geotrace_sdk::VERSION));
    Ok(())
}

#[test]
fn a_scrubbed_file_has_the_placeholder_version_and_no_commit()
-> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().with_scrubbed_provenance().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .time(NavFixTime::Receiver(base()))
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
    );

    let nav_file = recorder.finish()?;
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes)?;

    assert_eq!(
        nav_file.meta().sdk_version(),
        Some(geotrace_sdk::SCRUBBED_SDK_VERSION)
    );
    assert_eq!(nav_file.meta().sdk_git_commit(), None);
    assert_eq!(nav_file.meta().sdk_commit_time(), None);
    assert_eq!(NavFile::read(bytes.as_slice())?, nav_file);
    Ok(())
}

#[test]
fn a_file_with_the_provenance_attributes_reads_them_back() -> Result<(), Box<dyn std::error::Error>>
{
    let nav_file = read_file_with_attrs(|fb| {
        fb.set_attr("sdk_version", AttrValue::String("0.4.2".into()));
        fb.set_attr(
            "sdk_git_commit",
            AttrValue::String("0123456789abcdef0123456789abcdef01234567".into()),
        );
        fb.set_attr(
            "sdk_commit_time",
            AttrValue::String("2026-02-01T15:00:00Z".into()),
        );
    })?;

    let meta = nav_file.meta();
    assert_eq!(meta.sdk_version(), Some("0.4.2"));
    assert_eq!(
        meta.sdk_git_commit(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(
        meta.sdk_commit_time(),
        Some("2026-02-01T15:00:00Z".parse::<DateTime<Utc>>()?)
    );
    Ok(())
}

#[test]
fn a_file_written_without_the_provenance_attributes_reads_them_as_none()
-> Result<(), Box<dyn std::error::Error>> {
    let nav_file = read_file_with_attrs(|_| {})?;

    let meta = nav_file.meta();
    assert_eq!(meta.sdk_version(), None);
    assert_eq!(meta.sdk_git_commit(), None);
    assert_eq!(meta.sdk_commit_time(), None);
    Ok(())
}
