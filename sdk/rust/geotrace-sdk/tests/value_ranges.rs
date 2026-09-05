//! A latitude, longitude or heading outside its expected range is data, not a
//! parse error.
//! The SDK writes any value it is given.
//! It reads a latitude or longitude back unchanged, NaN included.

#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use std::path::Path;

use geotrace_sdk::{Angle, DateTime, NavFile, NavFileBuilder, NavFix, NavFixTime, Utc};

#[test]
#[expect(
    clippy::float_cmp,
    reason = "the fixture values must come back exactly"
)]
fn out_of_range_coordinates_read_verbatim() -> Result<(), Box<dyn std::error::Error>> {
    let nav_file = NavFile::open(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../c/tests/fixtures/out_of_range_values.gtd"),
    )?;
    let points = nav_file.nav_points();
    assert_eq!(points.len(), 4);

    assert!(points[0].fix.lat.as_degrees().is_nan());
    assert_eq!(points[1].fix.lat.as_degrees(), 91.0);
    assert_eq!(points[2].fix.lon.as_degrees(), -181.0);
    assert_eq!(points[3].fix.heading.map(Angle::as_degrees), Some(675.0));
    Ok(())
}

#[test]
#[expect(clippy::float_cmp, reason = "the written value must come back exactly")]
fn out_of_range_latitude_writes_and_reads_back() -> Result<(), Box<dyn std::error::Error>> {
    let time = "2024-06-01T08:00:00Z".parse::<DateTime<Utc>>()?;
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(
        NavFix::builder()
            .time(NavFixTime::Receiver(time))
            .lat(Angle::degrees(91.0))
            .lon(Angle::degrees(-0.1278))
            .build(),
    );

    let mut bytes = Vec::new();
    recorder.finish()?.write(&mut bytes)?;
    let read_back = NavFile::read(bytes.as_slice())?;

    assert_eq!(read_back.nav_points()[0].fix.lat.as_degrees(), 91.0);
    Ok(())
}
