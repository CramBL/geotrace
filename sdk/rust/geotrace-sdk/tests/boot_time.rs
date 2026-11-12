#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assertions"
)]

use geotrace_sdk::{
    Angle, BootTime, BootTimeError, Duration, Error, NavFile, NavFileBuilder, NavFix, NavFixTime,
};
use geotrace_sdk_test_util as test_util;
use hdf5_pure::{AttrValue, Datatype, DatatypeByteOrder, File, FileBuilder, FixedPointLayout};
use proptest::prelude::*;
use rstest::rstest;

#[rstest]
#[case::zero(0)]
#[case::maximum(u64::MAX - 1)]
fn boot_time_retains_exact_microseconds(#[case] micros: u64) -> Result<(), BootTimeError> {
    assert_eq!(BootTime::from_micros(micros)?.as_micros(), micros);
    Ok(())
}

#[test]
fn boot_time_rejects_the_absence_encoding() {
    assert_eq!(
        BootTime::from_micros(u64::MAX),
        Err(BootTimeError::ReservedValue)
    );
}

#[test]
fn boot_values_follow_fixes_through_sorting_and_round_trip()
-> Result<(), Box<dyn std::error::Error>> {
    let base = test_util::base();
    let inputs = [
        (
            NavFixTime::Receiver(base + Duration::seconds(2)),
            Some(u64::MAX - 1),
        ),
        (NavFixTime::Host(base), Some(0)),
        (
            NavFixTime::Both {
                gps: base + Duration::seconds(1),
                sys: base,
            },
            None,
        ),
        (NavFixTime::Receiver(base + Duration::seconds(2)), Some(0)),
    ];
    let mut recorder = NavFileBuilder::new().open();
    for (time, boot) in inputs {
        recorder.add(
            NavFix::builder()
                .time(time)
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .maybe_boot_time(boot.map(BootTime::from_micros).transpose()?)
                .build(),
        );
    }
    let original = recorder.finish()?;
    let bytes = test_util::to_bytes(&original)?;
    let decoded = NavFile::read(bytes.as_slice())?;
    assert!(original.equals_ignoring_build_provenance(&decoded));
    assert_eq!(decoded.nav_points().len(), inputs.len());
    let values: Vec<_> = decoded
        .nav_points()
        .iter()
        .map(|p| p.fix.boot_time.map(BootTime::as_micros))
        .collect();
    assert_eq!(values, vec![Some(0), None, Some(u64::MAX - 1), Some(0)]);
    let file = File::from_bytes(bytes)?;
    let column = file.group("nav_points")?.dataset("boot_time_us")?;
    assert_eq!(column.read_u64()?, vec![0, u64::MAX, u64::MAX - 1, 0]);
    assert_eq!(column.shape()?, vec![4]);
    assert_eq!(
        column.attrs()?.get("units").and_then(AttrValue::as_str),
        Some("microseconds since boot")
    );
    assert_eq!(
        file.root()
            .attrs()?
            .get("geotrace_version")
            .and_then(AttrValue::as_str),
        Some("3")
    );
    Ok(())
}

#[test]
fn writer_omits_an_all_absent_boot_column() -> Result<(), Box<dyn std::error::Error>> {
    let file = test_util::recorder_with_one_fix().finish()?;
    let bytes = test_util::to_bytes(&file)?;
    assert!(
        !File::from_bytes(bytes)?
            .group("nav_points")?
            .datasets()?
            .contains(&"boot_time_us".to_owned())
    );
    assert!(
        test_util::round_trip(&file)?
            .nav_points()
            .iter()
            .all(|p| p.fix.boot_time.is_none())
    );
    Ok(())
}

#[rstest]
#[case::version_one("1")]
#[case::version_two("2")]
fn older_recordings_have_absent_boot_readings(
    #[case] version: &'static str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = test_util::GtdFileContents {
        attrs: vec![("geotrace_version", version)],
        ..Default::default()
    }
    .into_gtd_bytes();
    let file = NavFile::read(bytes.as_slice())?;
    assert_eq!(file.nav_points()[0].fix.boot_time, None);
    Ok(())
}

struct BootColumnFixture {
    version: &'static str,
    values: Vec<u64>,
    shape: Vec<u64>,
    signed: bool,
    byte_order: DatatypeByteOrder,
}

impl Default for BootColumnFixture {
    fn default() -> Self {
        Self {
            version: "2",
            values: vec![0],
            shape: vec![1],
            signed: false,
            byte_order: DatatypeByteOrder::LittleEndian,
        }
    }
}

impl BootColumnFixture {
    fn to_bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut file = FileBuilder::new();
        file.set_attr("geotrace_version", AttrValue::String(self.version.into()));
        let mut group = file.create_group("nav_points");
        group
            .create_dataset("time")
            .with_i64_data(&[test_util::base().timestamp_micros()]);
        for name in ["lat", "lon", "heading", "speed_mps"] {
            group.create_dataset(name).with_f64_data(&[0.0]);
        }
        let bytes: Vec<u8> = self
            .values
            .iter()
            .flat_map(|value| match self.byte_order {
                DatatypeByteOrder::BigEndian => value.to_be_bytes(),
                _ => value.to_le_bytes(),
            })
            .collect();
        group
            .create_dataset("boot_time_us")
            .with_raw_data(
                Datatype::FixedPoint {
                    size: 8,
                    byte_order: self.byte_order,
                    layout: FixedPointLayout {
                        signed: self.signed,
                        bit_offset: 0,
                        bit_precision: 64,
                    },
                },
                bytes,
                self.values.len() as u64,
            )
            .with_shape(&self.shape);
        file.add_group(group.finish());
        Ok(file.finish()?)
    }
}

#[rstest]
#[case::little_endian(DatatypeByteOrder::LittleEndian)]
#[case::big_endian(DatatypeByteOrder::BigEndian)]
fn decoder_accepts_unsigned_64_bit_boot_time(
    #[case] byte_order: DatatypeByteOrder,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = BootColumnFixture {
        values: vec![u64::MAX - 1],
        byte_order,
        ..Default::default()
    }
    .to_bytes()?;
    let file = NavFile::read(bytes.as_slice())?;
    assert_eq!(
        file.nav_points()[0].fix.boot_time.map(BootTime::as_micros),
        Some(u64::MAX - 1)
    );
    Ok(())
}

#[rstest]
#[case::rank(BootColumnFixture { shape: vec![1, 1], ..Default::default() })]
#[case::length(BootColumnFixture { shape: vec![2], values: vec![0, 1], ..Default::default() })]
fn malformed_boot_column_shape_has_context(
    #[case] fixture: BootColumnFixture,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fixture.to_bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::InvalidDatasetShape { group: "nav_points", dataset: "boot_time_us", expected, actual } if expected == vec![1] && actual == fixture.shape)
    );
    Ok(())
}

#[test]
fn signed_boot_column_is_rejected_with_context() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = BootColumnFixture {
        signed: true,
        ..Default::default()
    }
    .to_bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidNavigationColumnType {
            dataset: "boot_time_us",
            ..
        }
    ));
    Ok(())
}

proptest! {
    #[test]
    fn arbitrary_boot_readings_round_trip(values in prop::collection::vec(any::<u64>(), 1..20)) {
        let mut recorder = NavFileBuilder::new().open();
        for (index, value) in values.iter().enumerate() {
            recorder.add(NavFix::builder()
                .time(NavFixTime::Host(test_util::base() + Duration::microseconds(index as i64)))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(0.0))
                .maybe_boot_time(BootTime::from_micros(*value).ok())
                .build());
        }
        let original = recorder.finish().unwrap();
        let decoded = test_util::round_trip(&original).unwrap();
        let actual: Vec<_> = decoded.nav_points().iter().map(|p| p.fix.boot_time.map_or(u64::MAX, BootTime::as_micros)).collect();
        prop_assert_eq!(actual, values);
    }
}
