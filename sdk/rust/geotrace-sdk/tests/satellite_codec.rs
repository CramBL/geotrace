#![expect(
    clippy::panic_in_result_fn,
    reason = "test assertions accompany fallible setup"
)]

use geotrace_sdk::{
    Constellation, ConstellationSet, DebugTimeRepair, Duration, Error, NavFile, NavFileBuilder,
    NavFileOpenMode, NavFixTime, Satellite, SatelliteCoverage, SatelliteEvidence,
    SatelliteIdentity, SatelliteMembership, SatelliteObservation, SatelliteObservationReport,
    SatelliteReport, SatelliteScope, SatelliteUsage, SatelliteUsageReport,
};
use geotrace_sdk_test_util as test_util;
use hdf5_pure::{AttrValue, Datatype, DatatypeByteOrder, File, FileBuilder, FixedPointLayout};
use rstest::rstest;

fn independent_evidence() -> SatelliteEvidence {
    let time = NavFixTime::Both {
        gps: test_util::t_ms(400),
        sys: test_util::t_ms(10400),
    };
    let gps = SatelliteIdentity::new(Constellation::Gps, 7);
    SatelliteEvidence::new(
        vec![
            SatelliteUsageReport::builder()
                .time(time)
                .scope(SatelliteScope::Constellations(ConstellationSet::new(
                    Constellation::Gps,
                    [],
                )))
                .coverage(SatelliteCoverage::Partial)
                .reported_count(8)
                .satellites(vec![
                    SatelliteUsage::new(gps, SatelliteMembership::Unknown),
                    SatelliteUsage::new(gps, SatelliteMembership::Unused),
                    SatelliteUsage::new(gps, SatelliteMembership::Used),
                ])
                .build(),
            SatelliteUsageReport::builder()
                .time(NavFixTime::Host(test_util::t_ms(10500)))
                .scope(SatelliteScope::Global)
                .coverage(SatelliteCoverage::Complete)
                .reported_count(0)
                .satellites(Vec::new())
                .build(),
        ],
        vec![
            SatelliteObservationReport::builder()
                .time(time)
                .scope(SatelliteScope::Global)
                .coverage(SatelliteCoverage::Complete)
                .reported_count(12)
                .satellites(vec![
                    SatelliteObservation::builder()
                        .identity(gps)
                        .snr(0.0)
                        .build(),
                    SatelliteObservation::builder()
                        .identity(gps)
                        .elevation(-5.0)
                        .azimuth(361.0)
                        .snr(42.0)
                        .build(),
                ])
                .build(),
            SatelliteObservationReport::builder()
                .time(NavFixTime::Receiver(test_util::t_ms(600)))
                .scope(SatelliteScope::Constellations(ConstellationSet::new(
                    Constellation::Gps,
                    [Constellation::Glonass],
                )))
                .coverage(SatelliteCoverage::Partial)
                .satellites(Vec::new())
                .build(),
        ],
    )
}

fn file_with_evidence(evidence: SatelliteEvidence) -> Result<NavFile, Box<dyn std::error::Error>> {
    let mut file = test_util::recorder_with_one_fix().finish()?;
    file.nav_points_mut()
        .first_mut()
        .ok_or("missing fixture point")?
        .set_satellite_evidence(evidence);
    Ok(file)
}

#[test]
fn independent_samples_preserve_times_rows_scopes_and_conflicting_counts()
-> Result<(), Box<dyn std::error::Error>> {
    let evidence = independent_evidence();
    let original = file_with_evidence(evidence.clone())?;
    let decoded = test_util::round_trip(&original)?;
    assert_eq!(decoded.nav_points()[0].satellite_evidence(), &evidence);
    assert_eq!(decoded.nav_points()[0].satellites(), &None);
    assert_eq!(decoded.nav_points()[0].fix, original.nav_points()[0].fix);

    let file = File::from_bytes(test_util::to_bytes(&original)?)?;
    file.group("sat_reports").unwrap_err();
    for group in ["satellite_usage_reports", "satellite_observation_reports"] {
        assert_eq!(
            file.group(group)?.dataset("nav_point_idx")?.read_u64()?,
            [0, 0]
        );
    }
    let usage = file.group("satellite_usage_reports")?;
    assert_eq!(usage.dataset("scope")?.read_u32()?, [1, 0]);
    assert_eq!(usage.dataset("coverage")?.read_u8()?, [1, 0]);
    assert_eq!(usage.dataset("reported_count")?.read_u32()?, [8, 0]);
    assert_eq!(usage.dataset("reported_count_present")?.read_u8()?, [1, 1]);
    assert_eq!(
        file.group("satellite_usage")?
            .dataset("membership")?
            .read_u8()?,
        [0, 1, 2]
    );
    assert_eq!(
        file.group("satellite_observations")?
            .dataset("snr")?
            .read_f32()?,
        [0.0, 42.0]
    );
    Ok(())
}

#[rstest]
#[case::absent(None)]
#[case::complete(Some(SatelliteCoverage::Complete))]
#[case::partial(Some(SatelliteCoverage::Partial))]
fn empty_usage_and_observations_preserve_presence_and_coverage(
    #[case] coverage: Option<SatelliteCoverage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let evidence = coverage
        .map(|coverage| {
            SatelliteEvidence::new(
                vec![
                    SatelliteUsageReport::builder()
                        .time(NavFixTime::Receiver(test_util::base()))
                        .scope(SatelliteScope::Global)
                        .coverage(coverage)
                        .satellites(Vec::new())
                        .build(),
                ],
                vec![
                    SatelliteObservationReport::builder()
                        .time(NavFixTime::Receiver(test_util::base()))
                        .scope(SatelliteScope::Global)
                        .coverage(coverage)
                        .satellites(Vec::new())
                        .build(),
                ],
            )
        })
        .unwrap_or_default();
    let decoded = test_util::round_trip(&file_with_evidence(evidence.clone())?)?;
    assert_eq!(decoded.nav_points()[0].satellite_evidence(), &evidence);
    assert_eq!(
        decoded.nav_points()[0].satellites().is_some(),
        coverage == Some(SatelliteCoverage::Complete)
    );
    Ok(())
}

#[rstest]
#[case::usage_only(true)]
#[case::observations_only(false)]
fn one_report_kind_has_no_legacy_combined_projection(
    #[case] usage_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut evidence = independent_evidence();
    if usage_only {
        evidence.observations.clear();
    } else {
        evidence.usage.clear();
    }
    let decoded = test_util::round_trip(&file_with_evidence(evidence.clone())?)?;
    assert_eq!(decoded.nav_points()[0].satellite_evidence(), &evidence);
    assert_eq!(decoded.nav_points()[0].satellites(), &None);
    Ok(())
}

#[test]
fn a_complete_constellation_set_remains_distinct_from_global_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let mut evidence = independent_evidence();
    evidence.usage[0].scope = SatelliteScope::Constellations(ConstellationSet::new(
        Constellation::Gps,
        [
            Constellation::Glonass,
            Constellation::Galileo,
            Constellation::Beidou,
            Constellation::Navic,
            Constellation::Qzss,
        ],
    ));
    let decoded = test_util::round_trip(&file_with_evidence(evidence.clone())?)?;
    assert_eq!(decoded.nav_points()[0].satellite_evidence(), &evidence);
    Ok(())
}

#[derive(Default)]
enum ColumnChange {
    #[default]
    Missing,
    Shape(Vec<u64>),
    Bytes(Vec<u8>),
    Datatype(Datatype),
    AddLegacyGroup,
    WithoutPlacements,
}

struct ModifiedColumn<'a> {
    group: &'a str,
    dataset: &'a str,
    change: ColumnChange,
}

impl ModifiedColumn<'_> {
    fn bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let source = File::from_bytes(test_util::to_bytes(&file_with_evidence(
            independent_evidence(),
        )?)?)?;
        let mut output = FileBuilder::new();
        for (name, value) in source.root().attrs()? {
            output.set_attr(&name, value);
        }
        for group_name in source.root().groups()? {
            if group_name == self.group && self.dataset.is_empty() {
                continue;
            }
            let input_group = source.group(&group_name)?;
            let mut group = output.create_group(&group_name);
            for name in input_group.datasets()? {
                let input = input_group.dataset(&name)?;
                if matches!(self.change, ColumnChange::WithoutPlacements)
                    && matches!(name.as_str(), "normalized_time_us" | "input_order")
                {
                    continue;
                }
                let changed = group_name == self.group && name == self.dataset;
                if changed && matches!(self.change, ColumnChange::Missing) {
                    continue;
                }
                let mut datatype = input.datatype()?;
                let mut shape = input.shape()?;
                let mut data = input.read_raw()?;
                if changed {
                    match &self.change {
                        ColumnChange::Missing
                        | ColumnChange::AddLegacyGroup
                        | ColumnChange::WithoutPlacements => {}
                        ColumnChange::Shape(value) => {
                            shape = value.clone();
                            data.resize(
                                usize::try_from(
                                    shape.iter().product::<u64>() * input.element_size()?,
                                )?,
                                0,
                            );
                        }
                        ColumnChange::Bytes(value) => data = value.clone(),
                        ColumnChange::Datatype(value) => datatype = value.clone(),
                    }
                }
                let count = shape.iter().product::<u64>();
                group
                    .create_dataset(&name)
                    .with_raw_data(datatype, data, count)
                    .with_shape(&shape);
            }
            output.add_group(group.finish());
        }
        if matches!(self.change, ColumnChange::AddLegacyGroup) {
            let mut legacy = output.create_group("sat_reports");
            legacy.create_dataset("nav_point_idx").with_u64_data(&[0]);
            output.add_group(legacy.finish());
        }
        Ok(output.finish()?)
    }
}

#[rstest]
#[case("satellite_usage_reports", "nav_point_idx")]
#[case("satellite_usage_reports", "gps_time_us")]
#[case("satellite_usage_reports", "sys_time_us")]
#[case("satellite_usage_reports", "scope")]
#[case("satellite_usage_reports", "coverage")]
#[case("satellite_usage_reports", "reported_count")]
#[case("satellite_usage_reports", "reported_count_present")]
#[case("satellite_usage_reports", "normalized_time_us")]
#[case("satellite_usage_reports", "input_order")]
#[case("satellite_observation_reports", "nav_point_idx")]
#[case("satellite_observation_reports", "gps_time_us")]
#[case("satellite_observation_reports", "sys_time_us")]
#[case("satellite_observation_reports", "scope")]
#[case("satellite_observation_reports", "coverage")]
#[case("satellite_observation_reports", "reported_count")]
#[case("satellite_observation_reports", "reported_count_present")]
#[case("satellite_observation_reports", "normalized_time_us")]
#[case("satellite_observation_reports", "input_order")]
#[case("satellite_usage", "report_idx")]
#[case("satellite_usage", "constellation")]
#[case("satellite_usage", "prn")]
#[case("satellite_usage", "membership")]
#[case("satellite_observations", "report_idx")]
#[case("satellite_observations", "constellation")]
#[case("satellite_observations", "prn")]
#[case("satellite_observations", "elevation")]
#[case("satellite_observations", "azimuth")]
#[case("satellite_observations", "snr")]
fn missing_columns_identify_the_satellite_field(
    #[case] group: &str,
    #[case] dataset: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group,
        dataset,
        change: ColumnChange::Missing,
    }
    .bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::MissingSatelliteColumn { group: actual_group, dataset: actual_dataset } if (actual_group, actual_dataset) == (group, dataset)),
        "{error}"
    );
    Ok(())
}

#[rstest]
#[case("satellite_usage_reports", "coverage", vec![2, 0], 2)]
#[case("satellite_observation_reports", "coverage", vec![0, 2], 2)]
#[case("satellite_usage_reports", "reported_count_present", vec![3, 1], 3)]
#[case("satellite_observation_reports", "reported_count_present", vec![1, 3], 3)]
#[case("satellite_usage", "membership", vec![0, 1, 3], 3)]
#[case("satellite_usage", "constellation", vec![0, 6, 0], 6)]
#[case("satellite_observations", "constellation", vec![0, 6], 6)]
#[case("satellite_usage_reports", "scope", [128_u32.to_le_bytes(), 0_u32.to_le_bytes()].concat(), 128)]
#[case("satellite_observation_reports", "scope", [0_u32.to_le_bytes(), 64_u32.to_le_bytes()].concat(), 64)]
fn invalid_tags_identify_the_satellite_field(
    #[case] group: &str,
    #[case] dataset: &str,
    #[case] data: Vec<u8>,
    #[case] code: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group,
        dataset,
        change: ColumnChange::Bytes(data),
    }
    .bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::InvalidSatelliteTag { group: actual_group, dataset: actual_dataset, code: actual_code, .. } if (actual_group, actual_dataset, actual_code) == (group, dataset, code)),
        "{error}"
    );
    Ok(())
}

#[rstest]
#[case("satellite_usage_reports", "nav_point_idx", vec![1_u64, 0], "nav_points")]
#[case("satellite_observation_reports", "nav_point_idx", vec![0_u64, u64::MAX], "nav_points")]
#[case("satellite_usage", "report_idx", vec![0_u64, 2, 0], "satellite_usage_reports")]
#[case("satellite_observations", "report_idx", vec![u64::MAX, 0], "satellite_observation_reports")]
fn invalid_references_identify_the_source_and_destination(
    #[case] group: &str,
    #[case] dataset: &str,
    #[case] indices: Vec<u64>,
    #[case] table: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group,
        dataset,
        change: ColumnChange::Bytes(indices.into_iter().flat_map(u64::to_le_bytes).collect()),
    }
    .bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::IndexPastTable { group: actual_group, dataset: actual_dataset, table: actual_table, .. } if (actual_group, actual_dataset, actual_table) == (group, dataset, table)),
        "{error}"
    );
    Ok(())
}

#[rstest]
#[case("satellite_usage_reports", "scope", vec![1, 2])]
#[case("satellite_observation_reports", "nav_point_idx", vec![1, 2])]
#[case("satellite_usage", "membership", vec![1, 3])]
#[case("satellite_observations", "elevation", vec![2, 1])]
fn satellite_columns_require_one_dimensional_shapes(
    #[case] group: &str,
    #[case] dataset: &str,
    #[case] shape: Vec<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group,
        dataset,
        change: ColumnChange::Shape(shape.clone()),
    }
    .bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::InvalidDatasetShape { group: actual_group, dataset: actual_dataset, actual, .. } if (actual_group, actual_dataset) == (group, dataset) && actual == shape)
    );
    Ok(())
}

#[rstest]
#[case("satellite_usage_reports", "coverage", 1)]
#[case("satellite_usage_reports", "scope", 4)]
#[case("satellite_observation_reports", "reported_count", 4)]
#[case("satellite_usage", "report_idx", 8)]
#[case("satellite_usage", "prn", 4)]
#[case("satellite_observations", "snr", 4)]
fn numeric_columns_require_exact_wire_types(
    #[case] group: &str,
    #[case] dataset: &str,
    #[case] width: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let datatype = Datatype::FixedPoint {
        size: width,
        byte_order: DatatypeByteOrder::LittleEndian,
        layout: FixedPointLayout {
            signed: true,
            bit_offset: 0,
            bit_precision: u16::try_from(width * 8)?,
        },
    };
    let bytes = ModifiedColumn {
        group,
        dataset,
        change: ColumnChange::Datatype(datatype),
    }
    .bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::InvalidSatelliteColumnType { group: actual_group, dataset: actual_dataset, .. } if (actual_group, actual_dataset) == (group, dataset)),
        "{error}"
    );
    Ok(())
}

#[rstest]
#[case("satellite_usage_reports", "gps_time_us")]
#[case("satellite_usage_reports", "normalized_time_us")]
#[case("satellite_observation_reports", "input_order")]
#[case("satellite_observation_reports", "scope")]
#[case("satellite_usage", "prn")]
#[case("satellite_observations", "snr")]
fn mismatched_column_lengths_identify_the_field(
    #[case] group: &str,
    #[case] dataset: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group,
        dataset,
        change: ColumnChange::Shape(vec![1]),
    }
    .bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::InvalidDatasetShape { group: actual_group, dataset: actual_dataset, .. } if (actual_group, actual_dataset) == (group, dataset)),
        "{error}"
    );
    Ok(())
}

#[rstest]
#[case("satellite_usage_reports", "satellite_usage_reports")]
#[case("satellite_usage", "satellite_usage")]
#[case("satellite_observation_reports", "satellite_observation_reports")]
#[case("satellite_observations", "satellite_observations")]
fn incomplete_table_pairs_identify_the_missing_group(
    #[case] removed: &str,
    #[case] expected: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group: removed,
        dataset: "",
        change: ColumnChange::Missing,
    }
    .bytes()?;
    let error = NavFile::read(bytes.as_slice()).unwrap_err();
    assert!(
        matches!(error, Error::MissingSatelliteGroup { group } if group == expected),
        "{error}"
    );
    Ok(())
}

#[test]
fn repeated_legacy_rows_preserve_individual_membership_and_measurements()
-> Result<(), Box<dyn std::error::Error>> {
    let rows = vec![
        Satellite::builder()
            .constellation(Constellation::Gps)
            .prn(7)
            .in_fix(false)
            .snr(0.0)
            .build(),
        Satellite::builder()
            .constellation(Constellation::Gps)
            .prn(7)
            .in_fix(true)
            .snr(42.0)
            .build(),
    ];
    let legacy = SatelliteReport::builder()
        .time(NavFixTime::Receiver(test_util::base()))
        .tracked(rows)
        .build();
    let mut recorder = test_util::recorder_with_one_fix();
    recorder.add_satellite_report(legacy.clone());
    let original = recorder.finish()?;
    let decoded = test_util::round_trip(&original)?;
    assert_eq!(decoded.nav_points()[0].satellites(), &Some(legacy));
    assert_eq!(
        decoded.nav_points()[0].satellite_evidence(),
        original.nav_points()[0].satellite_evidence()
    );
    Ok(())
}

#[test]
fn time_repair_shifts_each_independent_sample_once() -> Result<(), Box<dyn std::error::Error>> {
    let mut recorder = NavFileBuilder::new().open();
    for second in [0, 10, 20, 30] {
        recorder.add_nav_fix(test_util::fix_at(
            second * 1000,
            test_util::Lat(55.0),
            test_util::Lon(12.0),
        ));
    }
    let mut original = recorder.finish()?;
    original.nav_points_mut()[2].fix.time = NavFixTime::Receiver(test_util::t_s(0));
    original.nav_points_mut()[3].fix.time = NavFixTime::Receiver(test_util::t_s(10));
    original.nav_points_mut()[2].set_satellite_evidence(independent_evidence());
    let bytes = test_util::to_bytes(&original)?;
    let regular = NavFile::read(bytes.as_slice())?;
    assert_eq!(
        regular.nav_points()[2].satellite_evidence(),
        &independent_evidence()
    );
    let repaired = NavFile::read_with_mode(
        bytes.as_slice(),
        NavFileOpenMode::DebugTimeRepair(DebugTimeRepair::new(Duration::seconds(5))?),
    )?;
    assert_eq!(
        repaired.nav_points()[2].fix.time,
        NavFixTime::Receiver(test_util::t_s(15))
    );
    let evidence = repaired.nav_points()[2].satellite_evidence();
    assert_eq!(
        evidence.usage[0].time,
        NavFixTime::Both {
            gps: test_util::t_ms(15400),
            sys: test_util::t_ms(25400)
        }
    );
    assert_eq!(
        evidence.usage[1].time,
        NavFixTime::Host(test_util::t_ms(25500))
    );
    assert_eq!(evidence.observations[0].time, evidence.usage[0].time);
    assert_eq!(
        evidence.observations[1].time,
        NavFixTime::Receiver(test_util::t_ms(15600))
    );
    assert_eq!(repaired.nav_points()[2].satellites(), &None);
    Ok(())
}

#[test]
fn inspection_reports_independent_metadata_and_measurements()
-> Result<(), Box<dyn std::error::Error>> {
    let file = file_with_evidence(independent_evidence())?;
    let mut temporary = tempfile::NamedTempFile::new()?;
    file.write(temporary.as_file_mut())?;
    let output = NavFile::inspect(temporary.path())?;
    for expected in [
        "Satellite usage reports: 2",
        "Satellite observation reports: 2",
        "Partial, count Some(8)",
        "Complete, count Some(0)",
        "membership: Unknown",
        "snr: Some(0.0)",
        "point 0, report 1",
        "receiver time: 2025-05-23 11:33:20.400 UTC",
        "host time: 2025-05-23 11:33:30.400 UTC",
        "host time: unreported",
    ] {
        assert!(output.contains(expected), "missing {expected}: {output}");
    }
    Ok(())
}

#[test]
fn mixed_satellite_layouts_produce_a_structural_error() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group: "",
        dataset: "",
        change: ColumnChange::AddLegacyGroup,
    }
    .bytes()?;
    assert!(matches!(
        NavFile::read(bytes.as_slice()),
        Err(Error::MixedSatelliteLayouts)
    ));
    Ok(())
}

#[rstest]
#[case::v1("1", false)]
#[case::v2("2", false)]
#[case::early_v3("3", false)]
#[case::repeated_point_reference("3", true)]
fn combined_report_files_decode_to_canonical_evidence(
    #[case] version: &str,
    #[case] multiple_reports: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = File::from_bytes(test_util::to_bytes(
        &test_util::recorder_with_one_fix().finish()?,
    )?)?;
    let mut output = FileBuilder::new();
    output.set_attr("geotrace_version", AttrValue::String(version.into()));
    let source_nav = source.group("nav_points")?;
    let mut nav = output.create_group("nav_points");
    for name in source_nav.datasets()? {
        let column = source_nav.dataset(&name)?;
        nav.create_dataset(&name)
            .with_raw_data(column.datatype()?, column.read_raw()?, 1)
            .with_shape(&[1]);
    }
    output.add_group(nav.finish());
    let mut reports = output.create_group("sat_reports");
    let report_count = if multiple_reports { 2 } else { 1 };
    reports
        .create_dataset("nav_point_idx")
        .with_u64_data(&vec![0; report_count]);
    if version == "1" {
        reports
            .create_dataset("time")
            .with_i64_data(&vec![test_util::base().timestamp_micros(); report_count]);
    } else {
        reports.create_dataset("gps_time_us").with_u64_data(&vec![
            test_util::base()
                .timestamp_micros()
                .cast_unsigned();
            report_count
        ]);
        reports
            .create_dataset("sys_time_us")
            .with_u64_data(&vec![u64::MAX; report_count]);
    }
    output.add_group(reports.finish());
    let mut rows = output.create_group("tracked_sats");
    rows.create_dataset("sat_report_idx")
        .with_u64_data(&[0, u64::from(multiple_reports)]);
    rows.create_dataset("constellation").with_u8_data(&[0, 0]);
    rows.create_dataset("prn").with_u32_data(&[7, 7]);
    rows.create_dataset("in_fix").with_u8_data(&[0, 1]);
    rows.create_dataset("elevation")
        .with_f32_data(&[f32::NAN, 50.0]);
    rows.create_dataset("azimuth")
        .with_f32_data(&[10.0, f32::NAN]);
    rows.create_dataset("snr").with_f32_data(&[0.0, 42.0]);
    output.add_group(rows.finish());
    let decoded = NavFile::read(output.finish()?.as_slice())?;
    let point = &decoded.nav_points()[0];
    let expected = SatelliteReport::builder()
        .time(NavFixTime::Receiver(test_util::base()))
        .tracked(vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(7)
                .in_fix(false)
                .azimuth(10.0)
                .snr(0.0)
                .build(),
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(7)
                .in_fix(true)
                .elevation(50.0)
                .snr(42.0)
                .build(),
        ])
        .build();
    if multiple_reports {
        let mut evidence = SatelliteEvidence::default();
        for satellite in &expected.tracked {
            let report = SatelliteReport::builder()
                .time(expected.time)
                .tracked(vec![*satellite])
                .build();
            let sample = SatelliteEvidence::from(&report);
            evidence.usage.extend(sample.usage);
            evidence.observations.extend(sample.observations);
        }
        assert_eq!(point.satellites(), &None);
        assert_eq!(point.satellite_evidence(), &evidence);
    } else {
        assert_eq!(point.satellites(), &Some(expected.clone()));
        assert_eq!(
            point.satellite_evidence(),
            &SatelliteEvidence::from(&expected)
        );
    }
    assert_eq!(test_util::round_trip(&decoded)?.nav_points()[0], *point);
    Ok(())
}

#[test]
fn canonical_v3_reports_without_placement_columns_preserve_their_source_data()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = ModifiedColumn {
        group: "",
        dataset: "",
        change: ColumnChange::WithoutPlacements,
    }
    .bytes()?;
    let file = NavFile::read(bytes.as_slice())?;
    assert_eq!(
        file.nav_points()[0].satellite_evidence(),
        &independent_evidence()
    );
    Ok(())
}
