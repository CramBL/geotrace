use chrono::{DateTime, Duration};
use gt_analysis::{SlipAnalysisConfig, SlipThresholds, loss_of_lock, satellite_utilization};
use gt_types::fixtures;
use gt_types::satellites::{Constellation, ConstellationSet, Satellite, Satellites, Snr};
use gt_types::{
    GpsTime, Latitude, Longitude, NavPoint, SatelliteCoverage, SatelliteMembership,
    SatelliteQueryScope, SatelliteScope,
};
use rstest::rstest;

fn slip_config(window_min: f32) -> SlipAnalysisConfig {
    SlipAnalysisConfig {
        thresholds: SlipThresholds {
            elevation_mask_deg: 15.0,
            snr_drop_db: 10.0,
        },
        rate_window_min: Some(window_min),
    }
}

fn point(millis: i64, satellites: Vec<Satellite>) -> NavPoint {
    fixtures::nav_point_with_report(
        DateTime::UNIX_EPOCH + Duration::milliseconds(millis),
        Latitude::new(55.0),
        Longitude::new(12.0),
        Satellites::new(None, None, satellites),
    )
}

fn gps(prn: u32) -> Satellite {
    fixtures::satellite(
        Constellation::Gps,
        prn,
        Some(40.0),
        Some(Snr::new(42.0)),
        true,
    )
}

#[rstest]
#[case::known_used(
    Some(40.0),
    Some(SatelliteMembership::Used),
    SatelliteCoverage::Complete,
    Some(100.0)
)]
#[case::known_unused(
    Some(40.0),
    Some(SatelliteMembership::Unused),
    SatelliteCoverage::Complete,
    Some(0.0)
)]
#[case::unknown_membership(
    Some(40.0),
    Some(SatelliteMembership::Unknown),
    SatelliteCoverage::Complete,
    None
)]
#[case::observation_only(Some(40.0), None, SatelliteCoverage::Complete, None)]
#[case::unknown_elevation(
    None,
    Some(SatelliteMembership::Used),
    SatelliteCoverage::Complete,
    None
)]
#[case::below_mask(
    Some(5.0),
    Some(SatelliteMembership::Used),
    SatelliteCoverage::Complete,
    None
)]
#[case::partial_observations(
    Some(40.0),
    Some(SatelliteMembership::Used),
    SatelliteCoverage::Partial,
    None
)]
fn utilization_requires_geometry_coverage_and_membership(
    #[case] elevation: Option<f32>,
    #[case] membership: Option<SatelliteMembership>,
    #[case] coverage: SatelliteCoverage,
    #[case] expected: Option<f64>,
) {
    let mut input = point(0, vec![gps(1)]);
    input.navigation.used_count = Some(8);
    let observation = input
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value");
    observation.metadata.coverage = coverage;
    observation
        .satellites
        .first_mut()
        .expect("fixture value")
        .measurements
        .elevation = elevation;
    if let Some(membership) = membership {
        input
            .satellite_evidence
            .usage
            .first_mut()
            .expect("fixture value")
            .satellites
            .first_mut()
            .expect("fixture value")
            .membership = membership;
    } else {
        input.satellite_evidence.usage.clear();
    }
    let points = [input];
    assert_eq!(
        satellite_utilization::util_per_point(&points, 15.0).all,
        [expected]
    );
    assert_eq!(
        satellite_utilization::compute_util(&points, 15.0).all,
        expected
            .map(|value| [0.0, value])
            .into_iter()
            .collect::<Vec<_>>()
    );
}

#[test]
fn below_mask_unknown_membership_preserves_utilization_and_known_anomalies() {
    let mut input = point(0, vec![gps(1), gps(2), gps(3)]);
    let observation = input
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value");
    observation
        .satellites
        .get_mut(1)
        .expect("fixture value")
        .measurements
        .elevation = Some(5.0);
    observation
        .satellites
        .get_mut(2)
        .expect("fixture value")
        .measurements
        .elevation = Some(8.0);
    input
        .satellite_evidence
        .usage
        .first_mut()
        .expect("fixture value")
        .satellites
        .get_mut(1)
        .expect("fixture value")
        .membership = SatelliteMembership::Unknown;
    let util = satellite_utilization::compute_util(&[input], 15.0);
    assert_eq!(util.all, [[0.0, 100.0]]);
    assert_eq!(
        util.anomalies
            .first()
            .expect("fixture value")
            .masked
            .iter()
            .map(|satellite| satellite.prn.value())
            .collect::<Vec<_>>(),
        [3]
    );
}

#[test]
fn complete_usage_omissions_are_unused_and_partial_omissions_are_unknown() {
    let mut input = point(0, vec![gps(1), gps(2), gps(3), gps(4)]);
    input
        .satellite_evidence
        .usage
        .first_mut()
        .expect("fixture value")
        .satellites
        .clear();
    assert_eq!(
        satellite_utilization::util_per_point(&[input.clone()], 15.0).all,
        [Some(0.0)]
    );
    input
        .satellite_evidence
        .usage
        .first_mut()
        .expect("fixture value")
        .metadata
        .coverage = SatelliteCoverage::Partial;
    assert_eq!(
        satellite_utilization::util_per_point(&[input], 15.0).all,
        [None]
    );
}

#[test]
fn scoped_complete_evidence_supplies_only_the_requested_constellation() {
    let mut input = point(0, vec![gps(1)]);
    let scope = SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    input
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value")
        .metadata
        .scope = scope;
    input
        .satellite_evidence
        .usage
        .first_mut()
        .expect("fixture value")
        .metadata
        .scope = scope;
    let util = satellite_utilization::util_per_point(&[input], 15.0);
    assert_eq!(util.all, [None]);
    assert_eq!(util.gps, [Some(100.0)]);
    assert_eq!(util.glonass, [None]);
}

#[test]
fn diagnostic_epochs_preserve_multiple_report_times_and_point_reference() {
    let mut input = point(10_000, vec![gps(1)]);
    let mut later = input
        .satellite_evidence
        .observations
        .first()
        .expect("fixture value")
        .clone();
    later.metadata.normalized_time =
        GpsTime::from_utc(DateTime::UNIX_EPOCH + Duration::milliseconds(10_400));
    later.metadata.input_order = 1;
    later
        .satellites
        .first_mut()
        .expect("fixture value")
        .measurements
        .snr = Some(Snr::new(20.0));
    input.satellite_evidence.observations.push(later);
    let points = [input];
    let utilization = satellite_utilization::utilization_epochs(&points, 15.0);
    assert_eq!(
        utilization
            .iter()
            .map(|epoch| (
                epoch.source.point_index,
                epoch.source.observation_index,
                epoch
                    .source
                    .metadata
                    .normalized_time
                    .as_secs_f64_with_subseconds()
            ))
            .collect::<Vec<_>>(),
        [(0, 0, 10.0), (0, 1, 10.4)]
    );
    let slips = loss_of_lock::slip_epochs(&points, slip_config(1.0));
    assert_eq!(
        slips.last().expect("fixture value").source,
        utilization.last().expect("fixture value").source
    );
    assert_eq!(slips.last().expect("fixture value").slips.len(), 1);
    assert_eq!(
        loss_of_lock::slip_rate_series(&points, 15.0, 10.0, 1.0).all,
        [[10.4, 1.0]]
    );
    assert_eq!(
        loss_of_lock::slip_rate_per_point(&points, 15.0, 10.0, 1.0).all,
        [Some(1.0)]
    );
}

#[rstest]
#[case::partial(SatelliteCoverage::Partial, 0, None)]
#[case::complete(SatelliteCoverage::Complete, 1, Some(1.0))]
fn disappearance_requires_complete_coverage(
    #[case] coverage: SatelliteCoverage,
    #[case] count: usize,
    #[case] rate: Option<f64>,
) {
    let first = point(0, vec![gps(1)]);
    let mut second = point(1_000, vec![]);
    second
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value")
        .metadata
        .coverage = coverage;
    let points = [first, second];
    assert_eq!(
        loss_of_lock::detect_slip_events(&points, 15.0, 10.0).len(),
        count
    );
    assert_eq!(
        loss_of_lock::slip_rate_per_point(&points, 15.0, 10.0, 1.0).gps,
        [None, rate]
    );
}

#[test]
fn excluded_scope_preserves_identity_history_and_complete_omission_closes_presence() {
    let mut points = [
        point(0, vec![gps(1)]),
        point(1_000, vec![]),
        point(2_000, vec![]),
        point(3_000, vec![]),
    ];
    for (index, input) in points.iter_mut().enumerate() {
        let constellation = if index == 1 {
            Constellation::Glonass
        } else {
            Constellation::Gps
        };
        input
            .satellite_evidence
            .observations
            .first_mut()
            .expect("fixture value")
            .metadata
            .scope = SatelliteScope::Constellations(ConstellationSet::single(constellation));
    }
    let epochs = loss_of_lock::slip_epochs(&points, slip_config(1.0));
    assert_eq!(
        epochs
            .iter()
            .map(|epoch| epoch.slips.len())
            .collect::<Vec<_>>(),
        [0, 0, 1, 0]
    );
    assert_eq!(
        loss_of_lock::slip_rate_per_point(&points, 15.0, 10.0, 1.0).gps,
        [None, None, Some(1.0), Some(1.0)]
    );
    assert!(
        epochs
            .iter()
            .all(|epoch| epoch.rates.get(SatelliteQueryScope::Global).is_none())
    );
}

#[rstest]
#[case::drop(Some(42.0), Some(20.0), Some(1.0))]
#[case::measured_zero(Some(42.0), Some(0.0), Some(1.0))]
#[case::no_drop(Some(42.0), Some(40.0), Some(0.0))]
#[case::no_previous_signal(None, Some(20.0), None)]
#[case::no_current_signal(Some(42.0), None, None)]
fn signal_comparisons_preserve_unavailable_values(
    #[case] previous: Option<f32>,
    #[case] current: Option<f32>,
    #[case] expected: Option<f64>,
) {
    let mut points = [point(0, vec![gps(1)]), point(1_000, vec![gps(1)])];
    for (input, signal) in points.iter_mut().zip([previous, current]) {
        input
            .satellite_evidence
            .observations
            .first_mut()
            .expect("fixture value")
            .satellites
            .first_mut()
            .expect("fixture value")
            .measurements
            .snr = signal.map(Snr::new);
    }
    let epochs = loss_of_lock::slip_epochs(&points, slip_config(1.0));
    assert_eq!(epochs.last().expect("fixture value").rates.all, expected);
    assert_eq!(
        loss_of_lock::slip_rate_per_point(&points, 15.0, 10.0, 1.0).all,
        [None, expected]
    );
    assert_eq!(
        loss_of_lock::slip_rate_series(&points, 15.0, 10.0, 1.0).all,
        expected
            .map(|value| [1.0, value])
            .into_iter()
            .collect::<Vec<_>>()
    );
}

#[test]
fn utilization_selects_eligible_evidence_before_distance_and_scope_ordering() {
    let mut input = point(0, vec![gps(1)]);
    let mut farther = input
        .satellite_evidence
        .observations
        .first()
        .expect("fixture value")
        .clone();
    farther.metadata.normalized_time =
        GpsTime::from_utc(DateTime::UNIX_EPOCH + Duration::milliseconds(400));
    input
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value")
        .satellites
        .first_mut()
        .expect("fixture value")
        .measurements
        .elevation = None;
    input.satellite_evidence.observations.push(farther);
    let evidence = input
        .utilization_evidence(SatelliteQueryScope::Global, 15.0)
        .expect("fixture value");
    assert_eq!(evidence.observation_index, 1);
    let mut partial = input
        .satellite_evidence
        .usage
        .first()
        .expect("fixture value")
        .clone();
    partial.metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    partial.metadata.coverage = SatelliteCoverage::Partial;
    partial
        .satellites
        .first_mut()
        .expect("fixture value")
        .membership = SatelliteMembership::Unused;
    input.satellite_evidence.usage.push(partial);
    let evidence = input
        .utilization_evidence(SatelliteQueryScope::Global, 15.0)
        .expect("fixture value");
    assert_eq!(evidence.usage_index, 1);
    assert_eq!(
        satellite_utilization::util_per_point(&[input], 15.0)
            .all
            .first()
            .copied()
            .flatten(),
        Some(0.0)
    );
}

#[test]
fn navigation_summary_and_observations_preserve_counts_without_utilization_membership() {
    let mut input = point(0, (1..=12).map(gps).collect());
    input.navigation.used_count = Some(8);
    input.satellite_evidence.usage.clear();
    assert_eq!(
        input
            .resolved_used_count(SatelliteQueryScope::Global)
            .expect("fixture value")
            .count,
        8
    );
    assert_eq!(
        input
            .resolved_seen_count(SatelliteQueryScope::Global)
            .expect("fixture value")
            .count,
        12
    );
    assert_eq!(
        satellite_utilization::util_per_point(&[input], 15.0).all,
        [None]
    );
}

#[test]
fn an_independent_below_mask_anomaly_survives_an_unevaluable_ratio() {
    let mut input = point(0, vec![gps(1), gps(2)]);
    let observation = input
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value");
    observation
        .satellites
        .first_mut()
        .expect("fixture value")
        .measurements
        .elevation = None;
    observation
        .satellites
        .get_mut(1)
        .expect("fixture value")
        .measurements
        .elevation = Some(5.0);
    let util = satellite_utilization::compute_util(&[input], 15.0);
    assert!(util.all.is_empty());
    assert_eq!(
        util.anomalies
            .first()
            .expect("fixture value")
            .masked
            .first()
            .expect("fixture value")
            .prn
            .value(),
        2
    );
}

#[test]
fn missing_geometry_breaks_signal_comparisons() {
    let mut points = [
        point(0, vec![gps(1)]),
        point(1_000, vec![gps(1)]),
        point(2_000, vec![gps(1)]),
    ];
    points
        .get_mut(1)
        .expect("fixture value")
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value")
        .satellites
        .first_mut()
        .expect("fixture value")
        .measurements
        .elevation = None;
    assert_eq!(
        loss_of_lock::slip_rate_per_point(&points, 15.0, 10.0, 1.0).all,
        [None, None, None]
    );
    assert!(loss_of_lock::detect_slip_events(&points, 15.0, 10.0).is_empty());
}

proptest::proptest! {
    #[test]
    fn slip_rates_count_events_in_the_trailing_half_open_window(
        steps in proptest::collection::vec(1_i64..30, 1..25),
        window_seconds in 1_u16..120,
    ) {
        let count = u32::try_from(steps.len()).expect("fixture value");
        let mut points = vec![point(0, (1..=count).map(gps).collect())];
        let mut time = 0_i64;
        for (index, step) in steps.into_iter().enumerate() {
            time += step;
            let remaining = count - u32::try_from(index).expect("fixture value") - 1;
            points.push(point(time * 1000, (1..=remaining).map(gps).collect()));
        }
        let window_min = f32::from(window_seconds) / 60.0;
        let epochs = loss_of_lock::slip_epochs(&points, slip_config(window_min));
        proptest::prop_assert_eq!(epochs.first().expect("fixture value").rates.all, None);
        for (index, epoch) in epochs.iter().enumerate().skip(1) {
            let time = epoch.source.metadata.normalized_time.as_secs_f64_with_subseconds();
            let lower = time - f64::from(window_min) * 60.0;
            let count = epochs.iter().take(index + 1).filter(|epoch| {
                let event_time = epoch.source.metadata.normalized_time.as_secs_f64_with_subseconds();
                !epoch.slips.is_empty() && event_time > lower && event_time <= time
            }).count();
            let expected = Some(count as f64 / f64::from(window_min));
            proptest::prop_assert_eq!(epoch.rates.all, expected);
            proptest::prop_assert_eq!(epoch.rates.get(SatelliteQueryScope::Constellation(Constellation::Gps)), expected);
        }
    }
}

#[test]
fn report_time_steps_backwards_reset_history_within_one_point() {
    let mut input = point(10_000, vec![gps(1)]);
    input
        .satellite_evidence
        .observations
        .first_mut()
        .expect("fixture value")
        .satellites
        .first_mut()
        .expect("fixture value")
        .measurements
        .snr = Some(Snr::new(20.0));
    let mut earlier = input
        .satellite_evidence
        .observations
        .first()
        .expect("fixture value")
        .clone();
    earlier.metadata.normalized_time =
        GpsTime::from_utc(DateTime::UNIX_EPOCH + Duration::seconds(9));
    earlier
        .satellites
        .first_mut()
        .expect("fixture value")
        .measurements
        .snr = Some(Snr::new(42.0));
    input.satellite_evidence.observations.push(earlier);
    let epochs = loss_of_lock::slip_epochs(&[input], slip_config(1.0));
    assert_eq!(
        epochs
            .iter()
            .map(|epoch| epoch
                .source
                .metadata
                .normalized_time
                .as_secs_f64_with_subseconds())
            .collect::<Vec<_>>(),
        [10.0, 9.0]
    );
    assert!(
        epochs
            .iter()
            .all(|epoch| epoch.slips.is_empty() && epoch.rates.all.is_none())
    );
}

#[test]
fn partial_omission_preserves_the_previous_observation_until_complete_coverage() {
    let mut points = [
        point(0, vec![gps(1)]),
        point(1_000, vec![]),
        point(2_000, vec![]),
    ];
    points
        .get_mut(1)
        .expect("partial point")
        .satellite_evidence
        .observations
        .first_mut()
        .expect("partial report")
        .metadata
        .coverage = SatelliteCoverage::Partial;
    let epochs = loss_of_lock::slip_epochs(&points, slip_config(1.0));
    assert_eq!(
        epochs
            .iter()
            .map(|epoch| epoch.slips.len())
            .collect::<Vec<_>>(),
        [0, 0, 1]
    );
    assert_eq!(
        loss_of_lock::slip_rate_per_point(&points, 15.0, 10.0, 1.0).all,
        [None, None, Some(1.0)]
    );
}

#[test]
fn a_partial_signal_drop_is_an_event_with_an_unavailable_total_rate() {
    let mut points = [point(0, vec![gps(1)]), point(1_000, vec![gps(1)])];
    let report = points
        .get_mut(1)
        .expect("second point")
        .satellite_evidence
        .observations
        .first_mut()
        .expect("observation");
    report.metadata.coverage = SatelliteCoverage::Partial;
    report
        .satellites
        .first_mut()
        .expect("satellite")
        .measurements
        .snr = Some(Snr::new(20.0));
    let epochs = loss_of_lock::slip_epochs(&points, slip_config(1.0));
    assert_eq!(epochs.last().expect("epoch").slips.len(), 1);
    assert!(epochs.last().expect("epoch").rates.all.is_none());
    assert_eq!(
        loss_of_lock::slip_rate_per_point(&points, 15.0, 10.0, 1.0).all,
        [None, None]
    );
}
