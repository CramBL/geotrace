use geotrace_sdk::{
    Angle, Constellation, ConstellationSet, NavFile, NavFileBuilder, NavFix, NavFixTime,
    PositionProvenance, SatelliteCoverage, SatelliteEvidence, SatelliteIdentity,
    SatelliteMembership, SatelliteObservation, SatelliteObservationReport, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport, SolutionDimension, SolutionMethod, SolutionState,
    Velocity,
};
use geotrace_sdk_test_util as test_util;
use rstest::rstest;

#[expect(clippy::expect_used, reason = "fixture construction must succeed")]
fn recording() -> NavFile {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add(NavFix::status_only(
        NavFixTime::Receiver(test_util::base()),
        SolutionState::NoFix,
    ));
    for offset in [0, 100] {
        recorder.add(
            SatelliteUsageReport::builder()
                .time(NavFixTime::Receiver(test_util::t_ms(offset)))
                .scope(SatelliteScope::Global)
                .coverage(SatelliteCoverage::Partial)
                .reported_count(8)
                .satellites(vec![
                    SatelliteUsage::new(
                        SatelliteIdentity::new(Constellation::Gps, 1),
                        SatelliteMembership::Unknown,
                    ),
                    SatelliteUsage::new(
                        SatelliteIdentity::new(Constellation::Gps, 1),
                        SatelliteMembership::Used,
                    ),
                ])
                .build(),
        );
        recorder.add(
            SatelliteObservationReport::builder()
                .time(NavFixTime::Receiver(test_util::t_ms(offset)))
                .scope(SatelliteScope::Global)
                .coverage(SatelliteCoverage::Partial)
                .satellites(vec![
                    SatelliteObservation::builder()
                        .identity(SatelliteIdentity::new(Constellation::Gps, 1))
                        .snr(0.0)
                        .build(),
                    SatelliteObservation::builder()
                        .identity(SatelliteIdentity::new(Constellation::Gps, 1))
                        .azimuth(90.0)
                        .build(),
                ])
                .build(),
        );
    }
    recorder
        .finish()
        .expect("valid navigation and satellite samples")
}

#[test]
fn status_only_recordings_compare_after_readback_without_changing_ieee_equality() {
    let source = recording();
    let decoded = test_util::round_trip(&source).unwrap();
    assert!(source.equals_ignoring_build_provenance(&decoded));
    assert!(decoded.equals_ignoring_build_provenance(&source));
    assert_ne!(source, decoded);
    assert!(source.nav_points()[0].fix.lat.as_degrees().is_nan());
    assert!(decoded.nav_points()[0].fix.lon.as_degrees().is_nan());
}

#[test]
fn absent_coordinate_payloads_compare_without_changing_raw_values() {
    const OTHER_NAN_BITS: u64 = 0x7ff8_0000_0000_0042;
    let source = recording();
    let mut other = source.clone();
    other.nav_points_mut()[0].fix.lat = Angle::degrees(f64::from_bits(OTHER_NAN_BITS));
    other.nav_points_mut()[0].fix.lon = Angle::degrees(-f64::NAN);
    assert!(source.equals_ignoring_build_provenance(&other));
    assert_eq!(
        other.nav_points()[0].fix.lat.as_degrees().to_bits(),
        OTHER_NAN_BITS
    );
}

#[rstest]
#[case::one_nan(Some(SolutionState::NoFix), f64::NAN, 12.0)]
#[case::unreported_state(None, f64::NAN, f64::NAN)]
fn unclassified_nan_coordinates_retain_ieee_comparison(
    #[case] state: Option<SolutionState>,
    #[case] lat: f64,
    #[case] lon: f64,
) {
    let mut file = recording();
    let fix = &mut file.nav_points_mut()[0].fix;
    fix.state = state;
    fix.lat = Angle::degrees(lat);
    fix.lon = Angle::degrees(lon);
    assert!(!file.equals_ignoring_build_provenance(&file.clone()));
}

#[rstest]
#[case::time(|fix: &mut NavFix| fix.time = NavFixTime::Host(test_util::base()))]
#[case::state(|fix: &mut NavFix| fix.state = Some(SolutionState::Unknown))]
#[case::absent_state(|fix: &mut NavFix| fix.state = None)]
#[case::method(|fix: &mut NavFix| fix.method = Some(SolutionMethod::RtkFixed))]
#[case::dimension(|fix: &mut NavFix| fix.dimension = Some(SolutionDimension::ThreeD))]
#[case::zero_count(|fix: &mut NavFix| fix.used_count = Some(0))]
#[case::positive_count(|fix: &mut NavFix| fix.used_count = Some(8))]
#[case::heading(|fix: &mut NavFix| fix.heading = Some(Angle::degrees(0.0)))]
#[case::speed(|fix: &mut NavFix| fix.speed = Some(Velocity::meter_per_second(0.0)))]
#[case::accuracy(|fix: &mut NavFix| fix.eph_m = Some(0.9))]
#[case::hdop(|fix: &mut NavFix| fix.hdop = Some(0.9))]
#[case::vdop(|fix: &mut NavFix| fix.vdop = Some(0.0))]
#[case::pdop(|fix: &mut NavFix| fix.pdop = Some(1.5))]
#[case::origin(|fix: &mut NavFix| fix.position_provenance = Some(PositionProvenance::SdkSynthesized))]
#[case::latitude(|fix: &mut NavFix| fix.lat = Angle::degrees(91.0))]
#[case::longitude(|fix: &mut NavFix| fix.lon = Angle::degrees(181.0))]
fn recorded_navigation_differences_remain_unequal(#[case] change: fn(&mut NavFix)) {
    let source = recording();
    let mut other = source.clone();
    change(&mut other.nav_points_mut()[0].fix);
    assert!(!source.equals_ignoring_build_provenance(&other));
}

#[rstest]
#[case::usage_time(|e: &mut SatelliteEvidence| e.usage[0].time = NavFixTime::Host(test_util::base()))]
#[case::usage_scope(|e: &mut SatelliteEvidence| e.usage[0].scope = SatelliteScope::Constellations(ConstellationSet::new(Constellation::Gps, [])))]
#[case::usage_coverage(|e: &mut SatelliteEvidence| e.usage[0].coverage = SatelliteCoverage::Complete)]
#[case::usage_count_absent(|e: &mut SatelliteEvidence| e.usage[0].reported_count = None)]
#[case::usage_count_zero(|e: &mut SatelliteEvidence| e.usage[0].reported_count = Some(0))]
#[case::membership(|e: &mut SatelliteEvidence| e.usage[0].satellites[0].membership = SatelliteMembership::Unused)]
#[case::usage_identity(|e: &mut SatelliteEvidence| e.usage[0].satellites[0].identity.prn = 2)]
#[case::usage_rows(|e: &mut SatelliteEvidence| e.usage[0].satellites.reverse())]
#[case::usage_samples(|e: &mut SatelliteEvidence| e.usage.reverse())]
#[case::usage_time_mapping(|e: &mut SatelliteEvidence| e.usage[0].placement.as_mut().unwrap().normalized_time = test_util::t_ms(1))]
#[case::usage_input_order(|e: &mut SatelliteEvidence| e.usage[0].placement.as_mut().unwrap().input_order = 99)]
#[case::usage_placement_absent(|e: &mut SatelliteEvidence| e.usage[0].placement = None)]
#[case::usage_absent(|e: &mut SatelliteEvidence| e.usage.clear())]
#[case::observation_time(|e: &mut SatelliteEvidence| e.observations[0].time = NavFixTime::Host(test_util::base()))]
#[case::observation_scope(|e: &mut SatelliteEvidence| e.observations[0].scope = SatelliteScope::Constellations(ConstellationSet::new(Constellation::Gps, [])))]
#[case::observation_coverage(|e: &mut SatelliteEvidence| e.observations[0].coverage = SatelliteCoverage::Complete)]
#[case::observation_count(|e: &mut SatelliteEvidence| e.observations[0].reported_count = Some(0))]
#[case::signal_absent(|e: &mut SatelliteEvidence| e.observations[0].satellites[0].snr = None)]
#[case::elevation(|e: &mut SatelliteEvidence| e.observations[0].satellites[0].elevation = Some(0.0))]
#[case::azimuth(|e: &mut SatelliteEvidence| e.observations[0].satellites[0].azimuth = Some(0.0))]
#[case::observation_rows(|e: &mut SatelliteEvidence| e.observations[0].satellites.reverse())]
#[case::observation_samples(|e: &mut SatelliteEvidence| e.observations.reverse())]
#[case::observations_absent(|e: &mut SatelliteEvidence| e.observations.clear())]
fn recorded_report_differences_remain_unequal(#[case] change: fn(&mut SatelliteEvidence)) {
    let source = recording();
    let mut other = source.clone();
    let point = &mut other.nav_points_mut()[0];
    let mut evidence = point.satellite_evidence().clone();
    change(&mut evidence);
    point.set_satellite_evidence(evidence);
    assert!(!source.equals_ignoring_build_provenance(&other));
}
