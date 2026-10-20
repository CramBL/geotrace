use chrono::{DateTime, Duration};
use rstest::rstest;

use super::{NavigationEvidenceSource, ResolvedNavigationState};
use crate::navigation::SolutionState;
use crate::satellite_evidence::{
    SatelliteCoverage, SatelliteEvidence, SatelliteIdentity, SatelliteMembership,
    SatelliteObservation, SatelliteObservationReport, SatelliteQueryScope, SatelliteReportMetadata,
    SatelliteScope, SatelliteUsage, SatelliteUsageReport,
};
use crate::satellites::{Constellation, ConstellationSet, Prn, SatSample, Satellites};
use crate::{GpsTime, Latitude, Longitude, NavPoint, SysTime, TimePositionVelocity};

#[derive(Clone)]
struct UsageInput {
    count: Option<u32>,
    scope: SatelliteScope,
    coverage: SatelliteCoverage,
    offset_ms: i64,
    input_order: u64,
    memberships: Vec<SatelliteMembership>,
}

impl Default for UsageInput {
    fn default() -> Self {
        Self {
            count: None,
            scope: SatelliteScope::Global,
            coverage: SatelliteCoverage::Partial,
            offset_ms: 0,
            input_order: 0,
            memberships: Vec::new(),
        }
    }
}

impl UsageInput {
    fn report(self) -> SatelliteUsageReport {
        SatelliteUsageReport {
            metadata: SatelliteReportMetadata {
                gps_time: Some(GpsTime::from_utc(
                    DateTime::UNIX_EPOCH + Duration::milliseconds(self.offset_ms),
                )),
                sys_time: None,
                normalized_time: GpsTime::from_utc(
                    DateTime::UNIX_EPOCH + Duration::milliseconds(self.offset_ms),
                ),
                input_order: self.input_order,
                scope: self.scope,
                coverage: self.coverage,
                reported_count: self.count,
            },
            satellites: self
                .memberships
                .into_iter()
                .zip(1u32..)
                .map(|(membership, prn)| SatelliteUsage {
                    identity: SatelliteIdentity {
                        constellation: Constellation::Gps,
                        prn: Prn::new(prn),
                    },
                    membership,
                })
                .collect(),
        }
    }
}

fn point() -> NavPoint {
    NavPoint::new(
        TimePositionVelocity::builder()
            .time(GpsTime::from_utc(DateTime::UNIX_EPOCH))
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .build(),
        None,
    )
}

#[rstest]
#[case::absent(None, None, SolutionState::Unknown, None)]
#[case::explicit_unknown(
    Some(SolutionState::Unknown),
    Some(12),
    SolutionState::Unknown,
    Some(NavigationEvidenceSource::ExplicitState)
)]
#[case::explicit_fix_zero(
    Some(SolutionState::Fix),
    Some(0),
    SolutionState::Fix,
    Some(NavigationEvidenceSource::ExplicitState)
)]
#[case::explicit_no_fix_positive(
    Some(SolutionState::NoFix),
    Some(12),
    SolutionState::NoFix,
    Some(NavigationEvidenceSource::ExplicitState)
)]
#[case::explicit_estimate(
    Some(SolutionState::EstimatedOnly),
    Some(12),
    SolutionState::EstimatedOnly,
    Some(NavigationEvidenceSource::ExplicitState)
)]
#[case::summary_zero(
    None,
    Some(0),
    SolutionState::NoFix,
    Some(NavigationEvidenceSource::NavigationCount)
)]
#[case::summary_positive(
    None,
    Some(8),
    SolutionState::Fix,
    Some(NavigationEvidenceSource::NavigationCount)
)]
fn navigation_facts_precede_reports(
    #[case] state: Option<SolutionState>,
    #[case] count: Option<u32>,
    #[case] expected_state: SolutionState,
    #[case] expected_evidence: Option<NavigationEvidenceSource>,
) {
    let mut point = point();
    point.navigation.state = state;
    point.navigation.used_count = count;
    if count.is_some() {
        point.satellite_evidence.usage.push(
            UsageInput {
                count: Some(8),
                scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
                ..UsageInput::default()
            }
            .report(),
        );
    }
    assert_eq!(
        point.resolved_navigation_state(),
        ResolvedNavigationState {
            state: expected_state,
            evidence: expected_evidence
        }
    );
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        count
    );
}

#[rstest]
#[case::scoped_zero(UsageInput { count: Some(0), scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)), ..UsageInput::default() }, SolutionState::Unknown, None)]
#[case::partial_empty(UsageInput::default(), SolutionState::Unknown, None)]
#[case::complete_empty(UsageInput { coverage: SatelliteCoverage::Complete, ..UsageInput::default() }, SolutionState::NoFix, Some(NavigationEvidenceSource::UsageList(0)))]
#[case::unknown_row(UsageInput { coverage: SatelliteCoverage::Complete, memberships: vec![SatelliteMembership::Unknown], ..UsageInput::default() }, SolutionState::Unknown, None)]
#[case::positive_and_unknown(UsageInput { coverage: SatelliteCoverage::Complete, memberships: vec![SatelliteMembership::Used, SatelliteMembership::Unknown], ..UsageInput::default() }, SolutionState::Fix, Some(NavigationEvidenceSource::UsageList(0)))]
#[case::partial_positive(UsageInput { memberships: vec![SatelliteMembership::Used], ..UsageInput::default() }, SolutionState::Fix, Some(NavigationEvidenceSource::UsageList(0)))]
#[case::complete_unused(UsageInput { coverage: SatelliteCoverage::Complete, memberships: vec![SatelliteMembership::Unused], ..UsageInput::default() }, SolutionState::NoFix, Some(NavigationEvidenceSource::UsageList(0)))]
fn report_state_requires_eligible_evidence(
    #[case] input: UsageInput,
    #[case] expected_state: SolutionState,
    #[case] expected_evidence: Option<NavigationEvidenceSource>,
) {
    let mut point = point();
    point.satellite_evidence.usage.push(input.report());
    assert_eq!(
        point.resolved_navigation_state(),
        ResolvedNavigationState {
            state: expected_state,
            evidence: expected_evidence
        }
    );
}

#[rstest]
#[case::scope_breaks_equal_distance(0, 0, SolutionState::Fix, 1)]
#[case::distance_precedes_scope(100, 200, SolutionState::NoFix, 0)]
fn aggregate_selection_orders_distance_before_scope(
    #[case] global_offset_ms: i64,
    #[case] gps_offset_ms: i64,
    #[case] expected_state: SolutionState,
    #[case] expected_index: usize,
) {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            count: Some(0),
            offset_ms: global_offset_ms,
            ..UsageInput::default()
        }
        .report(),
        UsageInput {
            count: Some(8),
            offset_ms: gps_offset_ms,
            scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(
        point.resolved_navigation_state(),
        ResolvedNavigationState {
            state: expected_state,
            evidence: Some(NavigationEvidenceSource::UsageAggregate(expected_index))
        }
    );
    assert_eq!(point.satellite_evidence.usage.len(), 2);
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(0)
    );
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Constellation(Constellation::Gps))
            .map(|value| value.count),
        Some(8)
    );
}

#[test]
fn aggregate_state_precedes_a_nearer_list_and_inconclusive_reports_are_skipped() {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            count: Some(0),
            scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
            ..UsageInput::default()
        }
        .report(),
        UsageInput {
            coverage: SatelliteCoverage::Complete,
            offset_ms: 20,
            ..UsageInput::default()
        }
        .report(),
        UsageInput {
            count: Some(8),
            offset_ms: 400,
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(
        point.resolved_navigation_state().evidence,
        Some(NavigationEvidenceSource::UsageAggregate(2))
    );
    point.satellite_evidence.usage.pop();
    assert_eq!(
        point.resolved_navigation_state(),
        ResolvedNavigationState {
            state: SolutionState::NoFix,
            evidence: Some(NavigationEvidenceSource::UsageList(1))
        }
    );
}

#[test]
fn usage_lists_use_the_same_selection_order_as_aggregates() {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            coverage: SatelliteCoverage::Complete,
            offset_ms: 50,
            ..UsageInput::default()
        }
        .report(),
        UsageInput {
            memberships: vec![SatelliteMembership::Used],
            offset_ms: 100,
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(
        point.resolved_navigation_state(),
        ResolvedNavigationState {
            state: SolutionState::NoFix,
            evidence: Some(NavigationEvidenceSource::UsageList(0))
        }
    );
}

#[rstest]
#[case::earlier_time(-100, 100, 9, 0, 0)]
#[case::original_input_order(100, 100, 9, 3, 1)]
fn selection_uses_report_time_then_original_input_order(
    #[case] first_offset_ms: i64,
    #[case] second_offset_ms: i64,
    #[case] first_order: u64,
    #[case] second_order: u64,
    #[case] expected_index: usize,
) {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            count: Some(0),
            offset_ms: first_offset_ms,
            input_order: first_order,
            ..UsageInput::default()
        }
        .report(),
        UsageInput {
            count: Some(8),
            offset_ms: second_offset_ms,
            input_order: second_order,
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(
        point.resolved_navigation_state().evidence,
        Some(NavigationEvidenceSource::UsageAggregate(expected_index))
    );
}

#[test]
fn narrower_matching_constellation_sets_precede_broader_sets() {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            count: Some(8),
            scope: SatelliteScope::Constellations(
                ConstellationSet::single(Constellation::Gps).with(Constellation::Glonass),
            ),
            ..UsageInput::default()
        }
        .report(),
        UsageInput {
            count: Some(9),
            scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(
        point.resolved_navigation_state().evidence,
        Some(NavigationEvidenceSource::UsageAggregate(1))
    );
}

#[rstest]
#[case::partial_without_aggregate(
    SatelliteCoverage::Partial,
    None,
    Some(SatelliteMembership::Used),
    None
)]
#[case::complete_unknown(
    SatelliteCoverage::Complete,
    None,
    Some(SatelliteMembership::Unknown),
    None
)]
#[case::complete_empty(SatelliteCoverage::Complete, None, None, Some(0))]
#[case::complete_used(
    SatelliteCoverage::Complete,
    None,
    Some(SatelliteMembership::Used),
    Some(1)
)]
#[case::aggregate_over_list(
    SatelliteCoverage::Complete,
    Some(8),
    Some(SatelliteMembership::Used),
    Some(8)
)]
#[case::partial_with_zero(
    SatelliteCoverage::Partial,
    Some(0),
    Some(SatelliteMembership::Used),
    Some(0)
)]
fn exact_count_requires_coverage_or_an_aggregate(
    #[case] coverage: SatelliteCoverage,
    #[case] count: Option<u32>,
    #[case] membership: Option<SatelliteMembership>,
    #[case] expected_count: Option<u32>,
) {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            count,
            coverage,
            memberships: membership.into_iter().collect(),
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        expected_count
    );
}

#[rstest]
#[case::global(SatelliteScope::Global, Some(1), Some(0))]
#[case::gps(
    SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
    None,
    Some(0)
)]
#[case::combined(SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps).with(Constellation::Glonass)), None, Some(0))]
#[case::all_named(SatelliteScope::Constellations(ConstellationSet::all()), None, Some(0))]
fn complete_lists_establish_scoped_counts_including_omitted_constellations(
    #[case] scope: SatelliteScope,
    #[case] expected_global: Option<u32>,
    #[case] expected_glonass: Option<u32>,
) {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            coverage: SatelliteCoverage::Complete,
            scope,
            memberships: vec![SatelliteMembership::Used],
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        expected_global
    );
    let glonass = point
        .resolved_used_count(SatelliteQueryScope::Constellation(Constellation::Glonass))
        .map(|value| value.count);
    assert_eq!(
        glonass,
        if scope.covers(SatelliteQueryScope::Constellation(Constellation::Glonass)) {
            expected_glonass
        } else {
            None
        }
    );
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Constellation(Constellation::Gps))
            .map(|value| value.count),
        Some(1)
    );
}

#[test]
fn combined_aggregate_does_not_supply_constellation_counts() {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            count: Some(8),
            scope: SatelliteScope::Constellations(
                ConstellationSet::single(Constellation::Gps).with(Constellation::Glonass),
            ),
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(point.resolved_used_count(SatelliteQueryScope::Global), None);
    assert_eq!(
        point.resolved_used_count(SatelliteQueryScope::Constellation(Constellation::Gps)),
        None
    );
}

#[test]
fn unknown_membership_affects_only_its_constellation() {
    let mut report = UsageInput {
        coverage: SatelliteCoverage::Complete,
        memberships: vec![SatelliteMembership::Unknown],
        ..UsageInput::default()
    }
    .report();
    let identity = SatelliteIdentity {
        constellation: Constellation::Glonass,
        prn: Prn::new(1),
    };
    assert_eq!(report.list_used_count(SatelliteQueryScope::Global), None);
    assert_eq!(
        report.list_used_count(SatelliteQueryScope::Constellation(Constellation::Glonass)),
        Some(0)
    );
    assert_eq!(report.membership(identity), SatelliteMembership::Unused);
    report.metadata.coverage = SatelliteCoverage::Partial;
    assert_eq!(report.membership(identity), SatelliteMembership::Unknown);
    report.metadata.coverage = SatelliteCoverage::Complete;
    report.metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    assert_eq!(report.membership(identity), SatelliteMembership::Unknown);
}

#[test]
fn independent_partial_samples_are_never_combined_into_complete_counts() {
    let mut point = point();
    point.satellite_evidence.usage = vec![
        UsageInput {
            memberships: vec![SatelliteMembership::Used],
            ..UsageInput::default()
        }
        .report(),
        UsageInput {
            memberships: vec![SatelliteMembership::Unused],
            offset_ms: 100,
            ..UsageInput::default()
        }
        .report(),
    ];
    assert_eq!(point.resolved_used_count(SatelliteQueryScope::Global), None);
}

#[test]
fn observation_counts_and_geometry_are_independent_of_usage() {
    let mut point = point();
    let metadata = UsageInput {
        coverage: SatelliteCoverage::Complete,
        ..UsageInput::default()
    }
    .report()
    .metadata;
    point.satellite_evidence.observations = vec![SatelliteObservationReport {
        metadata,
        satellites: vec![SatelliteObservation {
            identity: SatelliteIdentity {
                constellation: Constellation::Gps,
                prn: Prn::new(1),
            },
            measurements: SatSample {
                elevation: None,
                azimuth: Some(90.0),
                snr: None,
            },
        }],
    }];
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(1)
    );
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Constellation(Constellation::Glonass))
            .map(|value| value.count),
        Some(0)
    );
    assert_eq!(point.resolved_used_count(SatelliteQueryScope::Global), None);
    assert_eq!(point.resolved_navigation_state().evidence, None);
    let report = point.satellite_evidence.observations.first_mut().unwrap();
    assert!(report.has_azimuth());
    assert!(!report.has_sky_position());
    report
        .satellites
        .first_mut()
        .unwrap()
        .measurements
        .elevation = Some(30.0);
    assert!(report.has_sky_position());
    report.metadata.coverage = SatelliteCoverage::Partial;
    report.metadata.reported_count = Some(12);
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(12)
    );
    assert_eq!(
        point.resolved_seen_count(SatelliteQueryScope::Constellation(Constellation::Gps)),
        None
    );
}

#[test]
fn observation_selection_filters_scope_and_coverage_before_distance() {
    let mut point = point();
    let global = UsageInput {
        coverage: SatelliteCoverage::Complete,
        offset_ms: 200,
        ..UsageInput::default()
    }
    .report()
    .metadata;
    let scoped = SatelliteReportMetadata {
        reported_count: Some(8),
        coverage: SatelliteCoverage::Partial,
        scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
        normalized_time: point.tpv.time(),
        ..global
    };
    point.satellite_evidence.observations = vec![
        SatelliteObservationReport {
            metadata: scoped,
            satellites: vec![],
        },
        SatelliteObservationReport {
            metadata: global,
            satellites: vec![],
        },
    ];
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Global)
            .map(|value| (value.count, value.evidence)),
        Some((0, NavigationEvidenceSource::ObservationList(1)))
    );
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Constellation(Constellation::Gps))
            .map(|value| (value.count, value.evidence)),
        Some((8, NavigationEvidenceSource::ObservationAggregate(0)))
    );
}

#[test]
fn duplicate_identities_count_once_and_known_membership_precedes_unknown() {
    let mut usage = UsageInput {
        coverage: SatelliteCoverage::Complete,
        memberships: vec![SatelliteMembership::Unknown],
        ..UsageInput::default()
    }
    .report();
    let identity = usage.satellites.first().unwrap().identity;
    usage.satellites.extend([
        SatelliteUsage {
            identity,
            membership: SatelliteMembership::Unused,
        },
        SatelliteUsage {
            identity,
            membership: SatelliteMembership::Used,
        },
    ]);
    assert_eq!(usage.list_used_count(SatelliteQueryScope::Global), Some(1));
    let observation = SatelliteObservation {
        identity,
        measurements: SatSample {
            elevation: None,
            azimuth: None,
            snr: None,
        },
    };
    let report = SatelliteObservationReport {
        metadata: usage.metadata,
        satellites: vec![observation, observation],
    };
    assert_eq!(report.list_seen_count(SatelliteQueryScope::Global), Some(1));
    let evidence = SatelliteEvidence {
        usage: vec![usage],
        observations: vec![report],
    };
    assert_eq!(evidence.usage.len(), 1);
}

#[test]
fn legacy_empty_report_supplies_known_zero_usage_and_observations() {
    let time = GpsTime::from_utc(DateTime::UNIX_EPOCH);
    let point = NavPoint::new(point().tpv, Some(Satellites::new(Some(time), None, vec![])));
    assert_eq!(
        point.resolved_navigation_state().state,
        SolutionState::NoFix
    );
    assert_eq!(
        point
            .resolved_used_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(0)
    );
    assert_eq!(
        point
            .resolved_seen_count(SatelliteQueryScope::Global)
            .map(|value| value.count),
        Some(0)
    );
}

#[test]
fn legacy_constructor_normalizes_host_report_time_without_replacing_it_with_fix_time() {
    let host_time = SysTime::from_utc(DateTime::UNIX_EPOCH + Duration::seconds(100));
    let report_time = SysTime::from_utc(host_time.utc() + Duration::milliseconds(400));
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(DateTime::UNIX_EPOCH))
        .sys_time(host_time)
        .lat(Latitude::new(55.0))
        .lon(Longitude::new(12.0))
        .build();
    let point = NavPoint::new(tpv, Some(Satellites::new(None, Some(report_time), vec![])));
    assert_eq!(
        point
            .satellite_evidence
            .usage
            .first()
            .map(|report| report.metadata.normalized_time.utc()),
        Some(DateTime::UNIX_EPOCH + Duration::milliseconds(400))
    );
}
