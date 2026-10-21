#![expect(
    clippy::expect_used,
    reason = "fixture construction requires valid recordings"
)]

use chrono::{DateTime, Duration, Utc};
use geotrace_sdk::{
    Angle, Constellation as SdkConstellation, NavFile, NavFileBuilder, NavFix, NavFixTime,
    NavRecorder, PositionProvenance as SdkProvenance, SatelliteCoverage as SdkCoverage,
    SatelliteEvidence as SdkEvidence, SatelliteIdentity as SdkIdentity,
    SatelliteMembership as SdkMembership, SatelliteObservation as SdkObservation,
    SatelliteObservationReport as SdkObservationReport, SatelliteScope as SdkScope,
    SatelliteUsage as SdkUsage, SatelliteUsageReport as SdkUsageReport,
    SolutionDimension as SdkDimension, SolutionMethod as SdkMethod, SolutionState as SdkState,
};
use gt_types::satellites::{Constellation, ConstellationSet, NO_DATA_SENTINEL_DB_HZ};
use gt_types::{
    GpsTime, NavPoint, NavigationEvidenceSource, PositionProvenance, SatelliteMembership,
    SatelliteQueryScope, SatelliteScope, SolutionDimension, SolutionMethod, SolutionState, SysTime,
};
use rstest::rstest;

fn fix() -> NavFix {
    NavFix::builder()
        .time(NavFixTime::Receiver(DateTime::UNIX_EPOCH))
        .lat(Angle::degrees(55.0))
        .lon(Angle::degrees(12.0))
        .build()
}

fn loaded_points(recorder: NavRecorder) -> Vec<NavPoint> {
    let file = recorder.finish().expect("the recording has fixes");
    let mut bytes = Vec::new();
    file.write(&mut bytes).expect("writing bytes succeeds");
    gt_loader::load_bytes(&bytes, "navigation.gtd".to_owned())
        .expect("the recording loads")
        .tracks
        .into_iter()
        .flat_map(|track| track.points)
        .collect()
}

fn observation_report() -> SdkObservationReport {
    SdkObservationReport::builder()
        .time(fix().time)
        .scope(SdkScope::Global)
        .coverage(SdkCoverage::Complete)
        .satellites(
            (1..=12)
                .map(|prn| {
                    SdkObservation::builder()
                        .identity(SdkIdentity::new(SdkConstellation::Gps, prn))
                        .build()
                })
                .collect(),
        )
        .build()
}

#[rstest]
#[case::summaries_only(false, false)]
#[case::observations_only(true, false)]
#[case::observations_with_membership(true, true)]
fn progressive_satellite_detail_preserves_navigation_facts(
    #[case] observations: bool,
    #[case] usage: bool,
) {
    let mut recorder = NavFileBuilder::new().open();
    let mut source = fix();
    source.state = Some(SdkState::Fix);
    source.used_count = Some(8);
    source.hdop = Some(0.9);
    recorder.add_nav_fix(source);
    if observations {
        recorder.add(observation_report());
    }
    if usage {
        recorder.add(
            SdkUsageReport::builder()
                .time(source.time)
                .scope(SdkScope::Global)
                .coverage(SdkCoverage::Complete)
                .satellites(
                    (1..=12)
                        .map(|prn| {
                            SdkUsage::new(
                                SdkIdentity::new(SdkConstellation::Gps, prn),
                                if prn <= 8 {
                                    SdkMembership::Used
                                } else {
                                    SdkMembership::Unused
                                },
                            )
                        })
                        .collect(),
                )
                .build(),
        );
    }
    let points = loaded_points(recorder);
    let point = points.first().expect("the source fix exists");
    assert_eq!(
        (point.tpv.lat().as_written(), point.tpv.lon().as_written()),
        (55.0, 12.0)
    );
    assert_eq!(point.tpv.heading(), None);
    assert_eq!(point.tpv.eph_m(), None);
    assert_eq!(point.navigation.hdop, Some(0.9));
    assert_eq!(point.navigation.vdop, None);
    assert_eq!(
        point.navigation.position_provenance,
        Some(PositionProvenance::SourceProvided)
    );
    assert_eq!(point.resolved_navigation_state().state, SolutionState::Fix);
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(8)
    );
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        observations.then_some(12)
    );
    assert_eq!(point.satellite_evidence.usage.is_empty(), !usage);
    assert_eq!(point.satellites.is_some(), observations && usage);
}

#[rstest]
#[case::no_fix(SdkState::NoFix, 12, SolutionState::NoFix)]
#[case::fix_zero(SdkState::Fix, 0, SolutionState::Fix)]
#[case::unknown_zero(SdkState::Unknown, 0, SolutionState::Unknown)]
#[case::estimated(SdkState::EstimatedOnly, 8, SolutionState::EstimatedOnly)]
fn contradictory_state_and_count_survive_loading(
    #[case] state: SdkState,
    #[case] count: u32,
    #[case] expected_state: SolutionState,
) {
    let mut recorder = NavFileBuilder::new().open();
    let mut source = fix();
    source.state = Some(state);
    source.used_count = Some(count);
    recorder.add_nav_fix(source);
    let points = loaded_points(recorder);
    let point = points.first().expect("the source fix exists");
    assert_eq!(point.resolved_navigation_state().state, expected_state);
    assert_eq!(
        point.resolved_navigation_state().evidence,
        Some(NavigationEvidenceSource::ExplicitState)
    );
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(count)
    );
}

#[rstest]
#[case::complete(SdkCoverage::Complete, Some(0))]
#[case::partial(SdkCoverage::Partial, None)]
fn empty_observations_establish_only_the_justified_seen_count(
    #[case] coverage: SdkCoverage,
    #[case] seen: Option<u32>,
) {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix());
    let mut observation = observation_report();
    observation.satellites.clear();
    observation.coverage = coverage;
    recorder.add(observation);
    let points = loaded_points(recorder);
    let point = points.first().expect("the source fix exists");
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        seen
    );
    assert_eq!(point.resolved_used_count(SatelliteQueryScope::Global), None);
    assert_eq!(point.resolved_navigation_state().evidence, None);
    assert!(point.satellites.is_none());
}

#[rstest]
#[case::complete(SdkCoverage::Complete, Some(1))]
#[case::partial(SdkCoverage::Partial, None)]
fn scoped_identity_coverage_remains_scoped(
    #[case] coverage: SdkCoverage,
    #[case] gps_count: Option<u32>,
) {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix());
    recorder.add(
        SdkUsageReport::builder()
            .time(fix().time)
            .scope(SdkScope::Constellations(
                geotrace_sdk::ConstellationSet::new(SdkConstellation::Gps, []),
            ))
            .coverage(coverage)
            .satellites(vec![SdkUsage::new(
                SdkIdentity::new(SdkConstellation::Gps, 7),
                SdkMembership::Used,
            )])
            .build(),
    );
    let points = loaded_points(recorder);
    let point = points.first().expect("the source fix exists");
    assert_eq!(point.resolved_used_count(SatelliteQueryScope::Global), None);
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Constellation(Constellation::Gps))
            .map(|value| value.count),
        gps_count
    );
    assert_eq!(
        point
            .satellite_evidence
            .usage
            .first()
            .map(|report| report.metadata.scope),
        Some(SatelliteScope::Constellations(ConstellationSet::single(
            Constellation::Gps
        )))
    );
}

#[test]
fn original_host_time_normalized_report_time_and_input_order_survive_loading() {
    let mut recorder = NavFileBuilder::new().open();
    let receiver_time = DateTime::UNIX_EPOCH + Duration::seconds(10);
    let host_time = receiver_time + Duration::seconds(100);
    let poll_time = host_time + Duration::milliseconds(400);
    let mut source = fix();
    source.time = NavFixTime::Both {
        gps: receiver_time,
        sys: host_time,
    };
    recorder.add_nav_fix(source);
    let mut observation = observation_report();
    observation.time = NavFixTime::Host(poll_time);
    recorder.add(observation);
    recorder.add(
        SdkUsageReport::builder()
            .time(NavFixTime::Host(poll_time))
            .scope(SdkScope::Global)
            .coverage(SdkCoverage::Partial)
            .reported_count(8)
            .satellites(Vec::new())
            .build(),
    );
    let points = loaded_points(recorder);
    assert_eq!(points.len(), 1);
    let point = points.first().expect("the source fix exists");
    let observation = &point
        .satellite_evidence
        .observations
        .first()
        .expect("the observation exists")
        .metadata;
    let usage = &point
        .satellite_evidence
        .usage
        .first()
        .expect("the usage exists")
        .metadata;
    for metadata in [observation, usage] {
        assert_eq!(metadata.gps_time, None);
        assert_eq!(metadata.sys_time, Some(SysTime::from_utc(poll_time)));
        assert_eq!(
            metadata.normalized_time,
            GpsTime::from_utc(receiver_time + Duration::milliseconds(400))
        );
    }
    assert_eq!((observation.input_order, usage.input_order), (0, 1));
    assert_eq!(point.tpv.time(), GpsTime::from_utc(receiver_time));
    assert_eq!(
        point.resolved_navigation_state().evidence,
        Some(NavigationEvidenceSource::UsageAggregate(0))
    );
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(8)
    );
}

#[test]
fn repeated_rows_merge_within_each_sample_and_original_sdk_rows_remain_available() {
    let identity = SdkIdentity::new(SdkConstellation::Gps, 7);
    let usage = SdkUsageReport::builder()
        .time(fix().time)
        .scope(SdkScope::Global)
        .coverage(SdkCoverage::Complete)
        .satellites(
            [
                SdkMembership::Unknown,
                SdkMembership::Unused,
                SdkMembership::Used,
            ]
            .map(|membership| SdkUsage::new(identity, membership))
            .to_vec(),
        )
        .build();
    let observation = SdkObservationReport::builder()
        .time(fix().time)
        .scope(SdkScope::Global)
        .coverage(SdkCoverage::Complete)
        .satellites(vec![
            SdkObservation::builder()
                .identity(identity)
                .azimuth(90.0)
                .snr(NO_DATA_SENTINEL_DB_HZ)
                .build(),
            SdkObservation::builder()
                .identity(identity)
                .elevation(30.0)
                .azimuth(180.0)
                .snr(0.0)
                .build(),
            SdkObservation::builder()
                .identity(identity)
                .elevation(40.0)
                .snr(42.0)
                .build(),
        ])
        .build();
    let mut recorder = NavFileBuilder::new().open();
    recorder
        .add_nav_fix(fix())
        .add(usage)
        .add(observation.clone());
    let mut later = observation;
    later.time = NavFixTime::Receiver(DateTime::UNIX_EPOCH + Duration::milliseconds(200));
    later.satellites.truncate(1);
    recorder.add(later);
    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("the recording is valid")
        .write(&mut bytes)
        .expect("writing bytes succeeds");
    let decoded = NavFile::read(bytes.as_slice()).expect("the SDK decodes the recording");
    let sdk_point = decoded.nav_points().first().expect("the source fix exists");
    assert_eq!(
        sdk_point
            .satellite_evidence()
            .usage
            .first()
            .map(|report| report.satellites.len()),
        Some(3)
    );
    assert_eq!(
        sdk_point
            .satellite_evidence()
            .observations
            .first()
            .map(|report| report.satellites.len()),
        Some(3)
    );
    let loaded = gt_loader::load_bytes(&bytes, "repeated.gtd".to_owned())
        .expect("the loader converts the recording");
    let point = loaded
        .tracks
        .first()
        .and_then(|track| track.points.first())
        .expect("the source fix exists");
    assert_eq!(
        point
            .satellite_evidence
            .usage
            .first()
            .map(|report| report.satellites.len()),
        Some(1)
    );
    assert_eq!(
        point
            .satellite_evidence
            .usage
            .first()
            .and_then(|report| report.satellites.first())
            .map(|row| row.membership),
        Some(SatelliteMembership::Used)
    );
    assert_eq!(point.satellite_evidence.observations.len(), 2);
    let observation = point
        .satellite_evidence
        .observations
        .first()
        .expect("the first sample exists");
    assert_eq!(observation.satellites.len(), 1);
    let row = observation
        .satellites
        .first()
        .expect("the merged observation exists");
    assert_eq!(
        (
            row.measurements.elevation,
            row.measurements.azimuth,
            row.measurements.snr.map(|snr| snr.value())
        ),
        (Some(30.0), Some(90.0), Some(42.0))
    );
    let later_row = point
        .satellite_evidence
        .observations
        .get(1)
        .and_then(|report| report.satellites.first())
        .expect("the second sample exists");
    assert_eq!(
        (
            later_row.measurements.elevation,
            later_row.measurements.snr.map(|snr| snr.value())
        ),
        (None, Some(NO_DATA_SENTINEL_DB_HZ))
    );
    assert_eq!(
        loaded
            .load_warnings
            .iter()
            .find(|warning| warning.issue == "satellite(s) merged from repeated evidence rows")
            .map(|warning| warning.count),
        Some(2)
    );
}

#[rstest]
#[case::autonomous(
    SdkMethod::Autonomous,
    SolutionMethod::Autonomous,
    SdkDimension::TwoD,
    SolutionDimension::TwoD,
    SdkProvenance::SourceProvided,
    PositionProvenance::SourceProvided
)]
#[case::differential(
    SdkMethod::Differential,
    SolutionMethod::Differential,
    SdkDimension::ThreeD,
    SolutionDimension::ThreeD,
    SdkProvenance::SdkSynthesized,
    PositionProvenance::SdkSynthesized
)]
#[case::float(
    SdkMethod::RtkFloat,
    SolutionMethod::RtkFloat,
    SdkDimension::TwoD,
    SolutionDimension::TwoD,
    SdkProvenance::SourceProvided,
    PositionProvenance::SourceProvided
)]
#[case::fixed(
    SdkMethod::RtkFixed,
    SolutionMethod::RtkFixed,
    SdkDimension::TwoD,
    SolutionDimension::TwoD,
    SdkProvenance::SourceProvided,
    PositionProvenance::SourceProvided
)]
#[case::dead_reckoning(
    SdkMethod::DeadReckoning,
    SolutionMethod::DeadReckoning,
    SdkDimension::TwoD,
    SolutionDimension::TwoD,
    SdkProvenance::SourceProvided,
    PositionProvenance::SourceProvided
)]
#[case::hybrid(
    SdkMethod::Hybrid,
    SolutionMethod::Hybrid,
    SdkDimension::TwoD,
    SolutionDimension::TwoD,
    SdkProvenance::SourceProvided,
    PositionProvenance::SourceProvided
)]
#[case::other(
    SdkMethod::Other,
    SolutionMethod::Other,
    SdkDimension::TwoD,
    SolutionDimension::TwoD,
    SdkProvenance::SourceProvided,
    PositionProvenance::SourceProvided
)]
fn descriptive_metadata_preserves_values_without_creating_state(
    #[case] sdk_method: SdkMethod,
    #[case] method: SolutionMethod,
    #[case] sdk_dimension: SdkDimension,
    #[case] dimension: SolutionDimension,
    #[case] sdk_provenance: SdkProvenance,
    #[case] provenance: PositionProvenance,
) {
    let mut recorder = NavFileBuilder::new().open();
    let mut source = fix();
    source.method = Some(sdk_method);
    source.dimension = Some(sdk_dimension);
    source.position_provenance = Some(sdk_provenance);
    source.hdop = Some(0.9);
    source.vdop = Some(1.1);
    source.pdop = Some(1.3);
    recorder.add_nav_fix(source);
    let points = loaded_points(recorder);
    let point = points.first().expect("the source fix exists");
    assert_eq!(point.navigation.method, Some(method));
    assert_eq!(point.navigation.dimension, Some(dimension));
    assert_eq!(point.navigation.position_provenance, Some(provenance));
    assert_eq!(
        (
            point.navigation.hdop,
            point.navigation.vdop,
            point.navigation.pdop
        ),
        (Some(0.9), Some(1.1), Some(1.3))
    );
    assert_eq!(point.resolved_navigation_state().evidence, None);
}

#[rstest]
#[case::in_range(55.0)]
#[case::out_of_range(91.0)]
fn headingless_legacy_coordinates_remain_recorded(#[case] latitude: f64) {
    let mut recorder = NavFileBuilder::new().with_lenient_errors().open();
    let mut source = fix();
    source.position_provenance = None;
    source.lat = Angle::degrees(latitude);
    recorder.add_nav_fix(source);
    let points = loaded_points(recorder);
    let point = points.first().expect("the source fix exists");
    assert_eq!(point.tpv.lat().as_written().to_bits(), latitude.to_bits());
    assert_eq!(point.tpv.heading(), None);
    assert_eq!(point.navigation.position_provenance, None);
    assert_eq!(
        point.resolved_navigation_state().state,
        SolutionState::Unknown
    );
    assert_eq!(point.resolved_navigation_state().evidence, None);
}

#[test]
fn host_timestamp_and_missing_heading_do_not_change_explicit_fix_state() {
    let mut recorder = NavFileBuilder::new().open();
    let mut source = fix();
    source.time = NavFixTime::Host(DateTime::UNIX_EPOCH);
    source.state = Some(SdkState::Fix);
    recorder.add(source);
    let points = loaded_points(recorder);
    let point = points.first().expect("the source fix exists");
    assert_eq!(point.tpv.gps_time(), None);
    assert_eq!(
        point.tpv.sys_time(),
        Some(SysTime::from_utc(DateTime::UNIX_EPOCH))
    );
    assert_eq!(point.tpv.heading(), None);
    assert_eq!(point.resolved_navigation_state().state, SolutionState::Fix);
    assert_eq!(
        point.navigation.position_provenance,
        Some(PositionProvenance::SourceProvided)
    );
}

#[test]
fn observation_only_orphan_has_synthesized_provenance_and_no_state_observation() {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(fix());
    let mut observation = observation_report();
    observation.time = NavFixTime::Receiver(DateTime::UNIX_EPOCH + Duration::seconds(1));
    recorder.add(observation);
    let points = loaded_points(recorder);
    assert_eq!(points.len(), 2);
    let carrier = points.get(1).expect("the orphan has a carrier");
    assert_eq!(
        carrier.navigation.position_provenance,
        Some(PositionProvenance::SdkSynthesized)
    );
    assert_eq!(carrier.navigation.state, None);
    assert_eq!(carrier.resolved_navigation_state().evidence, None);
    assert_eq!(
        carrier.resolved_used_count(SatelliteQueryScope::Global),
        None
    );
    assert_eq!(
        carrier
            .resolved_seen_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(12)
    );
}

#[test]
fn report_clock_normalization_out_of_range_returns_a_structured_error() {
    let mut source = fix();
    source.time = NavFixTime::Both {
        gps: DateTime::<Utc>::MAX_UTC,
        sys: DateTime::UNIX_EPOCH,
    };
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(source);
    let mut file = recorder.finish().expect("the source fix is valid");
    let report = SdkUsageReport::builder()
        .time(NavFixTime::Host(DateTime::<Utc>::MAX_UTC))
        .scope(SdkScope::Global)
        .coverage(SdkCoverage::Complete)
        .satellites(Vec::new())
        .build();
    file.nav_points_mut()
        .first_mut()
        .expect("the source fix exists")
        .set_satellite_evidence(SdkEvidence::new(vec![report], vec![]));
    let mut bytes = Vec::new();
    file.write(&mut bytes)
        .expect("the recorded clocks are representable");
    let error = gt_loader::load_bytes(&bytes, "clock_limit.gtd".to_owned())
        .expect_err("the normalized time is out of range");
    assert!(matches!(
        error,
        gt_loader::LoadError::ReportTimeOutOfRange {
            record: 0,
            kind: "usage",
            sample: 0
        }
    ));
}

#[rstest]
#[case::one_projected_row(2, 1, None)]
#[case::different_duplicate_counts(3, 2, Some(1))]
fn usage_merges_are_reported_when_projected_rows_have_different_counts(
    #[case] usage_rows: usize,
    #[case] observation_rows: usize,
    #[case] expected_legacy_warnings: Option<u32>,
) {
    let identity = SdkIdentity::new(SdkConstellation::Gps, 7);
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(fix());
    recorder.add(
        SdkUsageReport::builder()
            .time(fix().time)
            .scope(SdkScope::Global)
            .coverage(SdkCoverage::Complete)
            .satellites(vec![
                SdkUsage::new(identity, SdkMembership::Used);
                usage_rows
            ])
            .build(),
    );
    recorder.add(
        SdkObservationReport::builder()
            .time(fix().time)
            .scope(SdkScope::Global)
            .coverage(SdkCoverage::Complete)
            .satellites(vec![
                SdkObservation::builder().identity(identity).build();
                observation_rows
            ])
            .build(),
    );
    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("the recording is valid")
        .write(&mut bytes)
        .expect("writing bytes succeeds");
    let file =
        gt_loader::load_bytes(&bytes, "unequal_rows.gtd".to_owned()).expect("the recording loads");
    let point = file
        .tracks
        .first()
        .and_then(|track| track.points.first())
        .expect("the source fix exists");
    assert!(point.satellites.is_some());
    assert_eq!(
        point
            .satellite_evidence
            .usage
            .first()
            .map(|report| report.satellites.len()),
        Some(1)
    );
    assert_eq!(
        file.load_warnings
            .iter()
            .find(|warning| warning.issue == "satellite(s) merged from repeated evidence rows")
            .map(|warning| warning.count),
        Some(1)
    );
    assert_eq!(
        file.load_warnings
            .iter()
            .find(|warning| warning.issue == "satellite(s) merged from several rows of one report")
            .map(|warning| warning.count),
        expected_legacy_warnings
    );
}

#[test]
fn host_only_orphan_time_and_coordinates_survive_loading_and_placement() {
    let mut recorder = NavFileBuilder::new().open();
    for second in [0, 10] {
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Host(
                    DateTime::UNIX_EPOCH + Duration::seconds(second),
                ))
                .lat(Angle::degrees(0.0))
                .lon(Angle::degrees(second as f64))
                .build(),
        );
    }
    let poll_time = DateTime::UNIX_EPOCH + Duration::seconds(2);
    recorder.add(
        SdkUsageReport::builder()
            .time(NavFixTime::Host(poll_time))
            .scope(SdkScope::Global)
            .coverage(SdkCoverage::Partial)
            .reported_count(8)
            .satellites(Vec::new())
            .build(),
    );
    recorder.add(
        SdkObservationReport::builder()
            .time(NavFixTime::Host(poll_time))
            .scope(SdkScope::Global)
            .coverage(SdkCoverage::Partial)
            .reported_count(12)
            .satellites(Vec::new())
            .build(),
    );
    let recording = recorder.finish().expect("source fixes define the timeline");
    let mut bytes = Vec::new();
    recording.write(&mut bytes).expect("recording encodes");
    let loaded =
        gt_loader::load_bytes(&bytes, "host_poll.gtd".to_owned()).expect("recording loads");
    let track = loaded.tracks.first().expect("one track");
    assert_eq!((loaded.tracks.len(), track.points.len()), (1, 3));
    let placed = track
        .placed_points()
        .expect("source positions define geometry");
    for (point, second) in placed.iter().zip([0, 2, 10]) {
        assert_eq!(
            point.fix.tpv.time().utc(),
            DateTime::UNIX_EPOCH + Duration::seconds(second)
        );
        assert!((point.fix.tpv.lat().as_written()).abs() < 1e-12);
        assert!((point.fix.tpv.lon().as_written() - second as f64).abs() < 1e-12);
        let (latitude, longitude) = point.resolved_position();
        assert!(latitude.as_degrees().abs() < 1e-12);
        assert!((longitude.as_degrees() - second as f64).abs() < 1e-12);
        assert_eq!(
            point.fix.navigation.position_provenance,
            Some(if second == 2 {
                PositionProvenance::SdkSynthesized
            } else {
                PositionProvenance::SourceProvided
            })
        );
    }
    let carrier = &track.points[1];
    let usage = &carrier.satellite_evidence.usage;
    let observations = &carrier.satellite_evidence.observations;
    assert_eq!((usage.len(), observations.len()), (1, 1));
    for (metadata, input_order) in [(&usage[0].metadata, 0), (&observations[0].metadata, 1)] {
        assert_eq!(
            (
                metadata.gps_time,
                metadata.sys_time,
                metadata.normalized_time,
                metadata.input_order
            ),
            (
                None,
                Some(SysTime::from_utc(poll_time)),
                GpsTime::from_utc(poll_time),
                input_order
            ),
        );
    }
    assert_eq!(
        (
            carrier
                .resolved_used_count(SatelliteQueryScope::Global)
                .map(|count| count.count),
            carrier
                .resolved_seen_count(SatelliteQueryScope::Global)
                .map(|count| count.count),
        ),
        (Some(8), Some(12)),
    );
}
