use geotrace_sdk::{
    Angle, Constellation, ConstellationSet, Duration, NavFix, NavFixTime, NavPoint, Satellite,
    SatelliteCoverage, SatelliteEvidence, SatelliteIdentity, SatelliteMembership,
    SatelliteObservation, SatelliteObservationReport, SatelliteReport, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport,
};
use geotrace_sdk_test_util as test_util;
use rstest::rstest;
use strum::IntoEnumIterator as _;

fn legacy_report() -> SatelliteReport {
    SatelliteReport {
        time: NavFixTime::Both {
            gps: test_util::base(),
            sys: test_util::base() + Duration::seconds(2),
        },
        tracked: vec![
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(7)
                .in_fix(false)
                .snr(0.0)
                .azimuth(90.0)
                .build(),
            Satellite::builder()
                .constellation(Constellation::Gps)
                .prn(7)
                .in_fix(true)
                .elevation(45.0)
                .snr(42.0)
                .build(),
            Satellite::builder()
                .constellation(Constellation::Glonass)
                .prn(8)
                .build(),
        ],
    }
}

#[test]
fn legacy_split_preserves_individual_rows_and_both_clocks() {
    let legacy = legacy_report();
    let evidence = SatelliteEvidence::from(&legacy);
    let ([usage], [observations]) = (evidence.usage.as_slice(), evidence.observations.as_slice())
    else {
        panic!("one report of each kind required");
    };
    assert_eq!(usage.time, legacy.time);
    assert_eq!(observations.time, legacy.time);
    assert_eq!(usage.scope, SatelliteScope::Global);
    assert_eq!(observations.scope, SatelliteScope::Global);
    assert_eq!(usage.coverage, SatelliteCoverage::Complete);
    assert_eq!(observations.coverage, SatelliteCoverage::Complete);
    assert_eq!(usage.reported_count, None);
    assert_eq!(observations.reported_count, None);
    assert_eq!(
        usage.satellites,
        vec![
            SatelliteUsage::new(
                SatelliteIdentity::new(Constellation::Gps, 7),
                SatelliteMembership::Unused
            ),
            SatelliteUsage::new(
                SatelliteIdentity::new(Constellation::Gps, 7),
                SatelliteMembership::Used
            ),
            SatelliteUsage::new(
                SatelliteIdentity::new(Constellation::Glonass, 8),
                SatelliteMembership::Unused
            ),
        ]
    );
    assert_eq!(
        observations.satellites,
        vec![
            SatelliteObservation::builder()
                .identity(SatelliteIdentity::new(Constellation::Gps, 7))
                .snr(0.0)
                .azimuth(90.0)
                .build(),
            SatelliteObservation::builder()
                .identity(SatelliteIdentity::new(Constellation::Gps, 7))
                .snr(42.0)
                .elevation(45.0)
                .build(),
            SatelliteObservation::builder()
                .identity(SatelliteIdentity::new(Constellation::Glonass, 8))
                .build(),
        ]
    );
    assert_eq!(evidence.legacy_report(), Some(legacy));
}

#[test]
fn source_scope_is_canonical_and_distinct_from_global() {
    let gps = ConstellationSet::new(Constellation::Gps, [Constellation::Gps]);
    assert_eq!(gps.as_slice(), &[Constellation::Gps]);
    assert!(gps.contains(Constellation::Gps));
    assert!(!gps.contains(Constellation::Glonass));

    let all = ConstellationSet::new(Constellation::Qzss, Constellation::iter().rev());
    assert_eq!(all.as_slice(), Constellation::iter().collect::<Vec<_>>());
    assert_ne!(
        SatelliteScope::Constellations(all.clone()),
        SatelliteScope::Global
    );
    for constellation in Constellation::iter() {
        assert!(SatelliteScope::Global.contains(constellation));
        assert!(SatelliteScope::Constellations(all.clone()).contains(constellation));
    }
    assert!(!SatelliteScope::Constellations(gps).contains(Constellation::Galileo));
}

#[test]
fn independent_reports_preserve_unknown_membership_and_present_zero() {
    let identity = SatelliteIdentity::new(Constellation::Gps, 7);
    let usage = SatelliteUsageReport::builder()
        .time(NavFixTime::Host(test_util::base()))
        .scope(SatelliteScope::Constellations(ConstellationSet::new(
            Constellation::Gps,
            [],
        )))
        .coverage(SatelliteCoverage::Partial)
        .reported_count(0)
        .satellites(vec![SatelliteUsage::new(
            identity,
            SatelliteMembership::Unknown,
        )])
        .build();
    let observation = SatelliteObservationReport::builder()
        .time(NavFixTime::Receiver(
            test_util::base() + Duration::milliseconds(400),
        ))
        .scope(SatelliteScope::Global)
        .coverage(SatelliteCoverage::Complete)
        .satellites(vec![
            SatelliteObservation::builder()
                .identity(identity)
                .snr(0.0)
                .build(),
        ])
        .build();
    let evidence = SatelliteEvidence::new(vec![usage.clone()], vec![observation.clone()]);
    assert_eq!(evidence.usage, vec![usage]);
    assert_eq!(evidence.observations, vec![observation]);
    assert_eq!(evidence.legacy_report(), None);
}

#[test]
fn absent_complete_empty_and_partial_empty_are_distinct() {
    let absent = SatelliteEvidence::default();
    let empty = SatelliteReport {
        time: NavFixTime::Receiver(test_util::base()),
        tracked: vec![],
    };
    let complete = SatelliteEvidence::from(&empty);
    let mut partial = complete.clone();
    for usage in &mut partial.usage {
        usage.coverage = SatelliteCoverage::Partial;
    }
    for observation in &mut partial.observations {
        observation.coverage = SatelliteCoverage::Partial;
    }
    assert_ne!(absent, complete);
    assert_ne!(complete, partial);
    assert_ne!(absent, partial);
    assert_eq!(absent.legacy_report(), None);
    assert_eq!(complete.legacy_report(), Some(empty));
    assert_eq!(partial.legacy_report(), None);
}

#[rstest]
#[case::usage_absent(|evidence: &mut SatelliteEvidence| evidence.usage.clear())]
#[case::observations_absent(|evidence: &mut SatelliteEvidence| evidence.observations.clear())]
#[case::multiple_usage(|evidence: &mut SatelliteEvidence| evidence.usage.extend(evidence.usage.clone()))]
#[case::multiple_observations(|evidence: &mut SatelliteEvidence| evidence.observations.extend(evidence.observations.clone()))]
#[case::usage_partial(|evidence: &mut SatelliteEvidence| evidence
            .usage
            .iter_mut()
            .for_each(|r| r.coverage = SatelliteCoverage::Partial))]
#[case::observations_partial(|evidence: &mut SatelliteEvidence| evidence
            .observations
            .iter_mut()
            .for_each(|r| r.coverage = SatelliteCoverage::Partial))]
#[case::usage_scoped(|evidence: &mut SatelliteEvidence| evidence.usage.iter_mut().for_each(|r| {
            r.scope = SatelliteScope::Constellations(ConstellationSet::new(Constellation::Gps, []))
        }))]
#[case::observations_scoped(|evidence: &mut SatelliteEvidence| evidence.observations.iter_mut().for_each(|r| {
            r.scope = SatelliteScope::Constellations(ConstellationSet::new(Constellation::Gps, []))
        }))]
#[case::different_receiver_times(|evidence: &mut SatelliteEvidence| evidence.usage.iter_mut().for_each(|r| {
            r.time = NavFixTime::Both {
                gps: test_util::base() + Duration::milliseconds(1),
                sys: test_util::base() + Duration::seconds(2),
            }
        }))]
#[case::different_host_times(|evidence: &mut SatelliteEvidence| evidence.usage.iter_mut().for_each(|r| {
            r.time = NavFixTime::Both {
                gps: test_util::base(),
                sys: test_util::base() + Duration::seconds(3),
            }
        }))]
#[case::different_clock_presence(|evidence: &mut SatelliteEvidence| evidence
            .usage
            .iter_mut()
            .for_each(|r| r.time = NavFixTime::Receiver(test_util::base())))]
#[case::unknown_membership(|evidence: &mut SatelliteEvidence| evidence
            .usage
            .iter_mut()
            .flat_map(|r| &mut r.satellites)
            .for_each(|s| s.membership = SatelliteMembership::Unknown))]
#[case::used_count_conflict(|evidence: &mut SatelliteEvidence| evidence
            .usage
            .iter_mut()
            .for_each(|r| r.reported_count = Some(2)))]
#[case::seen_count_conflict(|evidence: &mut SatelliteEvidence| evidence
            .observations
            .iter_mut()
            .for_each(|r| r.reported_count = Some(3)))]
#[case::used_identity_unobserved(|evidence: &mut SatelliteEvidence| evidence.observations.iter_mut().for_each(|r| {
            r.satellites
                .retain(|s| s.identity.constellation != Constellation::Gps)
        }))]
#[case::ambiguous_duplicate_membership(|evidence: &mut SatelliteEvidence| evidence
            .observations
            .iter_mut()
            .for_each(|r| r.satellites.reverse()))]
fn unrepresentable_evidence_has_no_legacy_projection(#[case] modify: fn(&mut SatelliteEvidence)) {
    let mut evidence = SatelliteEvidence::from(&legacy_report());
    modify(&mut evidence);
    assert_eq!(evidence.legacy_report(), None);
}

#[test]
fn projection_matches_reordered_rows_and_complete_omissions() {
    let mut legacy = legacy_report();
    legacy.tracked.remove(0);
    let mut evidence = SatelliteEvidence::from(&legacy);
    for usage in &mut evidence.usage {
        usage.satellites.reverse();
        usage
            .satellites
            .retain(|s| s.membership == SatelliteMembership::Used);
        usage.satellites.push(SatelliteUsage::new(
            SatelliteIdentity::new(Constellation::Galileo, 42),
            SatelliteMembership::Unused,
        ));
        usage.reported_count = Some(1);
    }
    for observation in &mut evidence.observations {
        observation.reported_count = Some(2);
    }
    assert_eq!(evidence.legacy_report(), Some(legacy));
}

#[test]
fn consistent_aggregates_count_distinct_identities() {
    let legacy = legacy_report();
    let mut evidence = SatelliteEvidence::from(&legacy);
    for usage in &mut evidence.usage {
        usage.reported_count = Some(1);
    }
    for observation in &mut evidence.observations {
        observation.reported_count = Some(2);
    }
    assert_eq!(evidence.legacy_report(), Some(legacy));
}

#[test]
fn legacy_nav_point_access_preserves_report_presence() {
    let mut point = NavPoint {
        fix: NavFix::builder()
            .time(NavFixTime::Receiver(test_util::base()))
            .lat(Angle::degrees(51.5))
            .lon(Angle::degrees(-0.1))
            .build(),
        satellites: None,
    };
    assert_eq!(point.satellite_evidence(), SatelliteEvidence::default());
    point.satellites = Some(legacy_report());
    assert_eq!(point.satellite_evidence().legacy_report(), point.satellites);
}
