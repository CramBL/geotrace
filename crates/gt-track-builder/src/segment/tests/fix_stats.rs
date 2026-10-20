use std::path::PathBuf;

use chrono::Duration;
use gt_types::markers::GeneratedMarkerKind;
use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, ConstellationSet};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::track::{FileSource, FixStats, LoadedFile};
use gt_types::{
    PositionProvenance, SatelliteCoverage, SatelliteObservationReport, SatelliteReportMetadata,
    SatelliteScope, SatelliteUsageReport, SolutionState,
};
use rstest::rstest;

use crate::segment::{self, FileMeta, SegmentationConfig};
use crate::test_util;

#[rstest]
#[case::no_fixes(vec![])]
#[case::equal_time(vec![test_util::fix_with_a_satellite_in_fix(0), test_util::fix_with_a_satellite_in_fix(0)])]
#[case::one_fix_with_a_report(vec![test_util::fix_with_a_satellite_in_fix(0)])]
fn compute_fix_stats_requires_positive_run_duration(#[case] pts: Vec<NavPoint>) {
    assert!(segment::compute_fix_stats(&pts).is_none());
}

#[test]
fn compute_fix_stats_all_with_fix() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_fix(60),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(60));
    assert_eq!(stats.time_without_fix, Duration::zero());
    assert_eq!(stats.fix_loss_count, 0);
    assert_eq!(stats.max_continuous_no_fix, Duration::zero());
}

#[test]
fn compute_fix_stats_all_without_fix() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_view_only(0),
        test_util::fix_with_a_satellite_in_view_only(120),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::zero());
    assert_eq!(stats.time_without_fix, Duration::seconds(120));
    assert_eq!(stats.fix_loss_count, 0);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
}

#[test]
fn compute_fix_stats_fix_then_lost() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_view_only(60),
        test_util::fix_with_a_satellite_in_view_only(180),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(60));
    assert_eq!(stats.time_without_fix, Duration::seconds(120));
    assert_eq!(stats.fix_loss_count, 1);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
}

#[test]
fn compute_fix_stats_multiple_losses() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_view_only(100),
        test_util::fix_with_a_satellite_in_view_only(200),
        test_util::fix_with_a_satellite_in_fix(300),
        test_util::fix_with_a_satellite_in_view_only(400),
        test_util::fix_with_a_satellite_in_view_only(450),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(200));
    assert_eq!(stats.time_without_fix, Duration::seconds(250));
    assert_eq!(stats.fix_loss_count, 2);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(200));
}

#[test]
fn compute_fix_stats_ignores_points_without_sat_data() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_without_a_satellite_report(30),
        test_util::fix_without_a_satellite_report(60),
        test_util::fix_with_a_satellite_in_fix(90),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(90));
    assert_eq!(stats.time_without_fix, Duration::zero());
    assert_eq!(stats.fix_loss_count, 0);
}

#[test]
fn build_loaded_file_file_fix_stats_aggregates_tracks() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_view_only(60),
        test_util::fix_with_a_satellite_in_view_only(180),
        test_util::fix_with_a_satellite_in_fix(10_000),
        test_util::fix_with_a_satellite_in_view_only(10_060),
        test_util::fix_with_a_satellite_in_fix(10_120),
    ];
    let f = segment::build_loaded_file(
        "test.gtd".to_owned(),
        &pts,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("test.gtd")),
        FileMeta::default(),
        vec![],
    );
    assert_eq!(f.tracks.len(), 2, "expected two tracks");
    let stats = f.metadata.fix_stats.expect("fix stats should be present");
    assert_eq!(stats.time_with_fix, Duration::seconds(60 + 60));
    assert_eq!(stats.time_without_fix, Duration::seconds(120 + 60));
    assert_eq!(stats.fix_loss_count, 2);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
}

proptest::proptest! {
    #[test]
    fn all_state_buckets_sum_to_positive_intervals_in_recorded_order(
        deltas_and_states in proptest::collection::vec((-10i64..20, 0u8..5), 0..30usize)
    ) {
        let mut second = 0;
        let points: Vec<_> = deltas_and_states.into_iter().map(|(delta, state)| {
            second += delta;
            let state = match state {
                0 => Some(SolutionState::Fix),
                1 => Some(SolutionState::NoFix),
                2 => Some(SolutionState::EstimatedOnly),
                3 => Some(SolutionState::Unknown),
                _ => None,
            };
            StateSample { second, state }.point()
        }).collect();
        let expected_total = points.windows(2).map(|pair| {
            let [left, right] = pair else { return Duration::zero() };
            (right.tpv.time() - left.tpv.time()).max(Duration::zero())
        }).sum::<Duration>();
        let stats = segment::compute_fix_stats(&points);
        proptest::prop_assert_eq!(stats.map_or(Duration::zero(), FixStats::total_duration), expected_total);
        proptest::prop_assert_eq!(stats.is_some(), expected_total > Duration::zero());
        if let Some(stats) = stats {
            proptest::prop_assert!(stats.time_with_fix >= Duration::zero());
            proptest::prop_assert!(stats.time_without_fix >= Duration::zero());
            proptest::prop_assert!(stats.time_estimated_only >= Duration::zero());
            proptest::prop_assert!(stats.time_unknown >= Duration::zero());
            proptest::prop_assert!(stats.max_continuous_no_fix <= stats.time_without_fix + stats.time_estimated_only);
            proptest::prop_assert!((0.0..=1.0).contains(&stats.known_state_coverage()));
            proptest::prop_assert!(stats.availability().is_none_or(|fraction| (0.0..=1.0).contains(&fraction)));
        }
    }
}

struct StateSample {
    second: i64,
    state: Option<SolutionState>,
}

impl StateSample {
    fn point(self) -> NavPoint {
        let mut point = test_util::fix_without_a_satellite_report(self.second);
        point.navigation.state = self.state;
        point
    }
}

fn build_file(points: &[NavPoint]) -> LoadedFile {
    segment::build_loaded_file(
        "navigation.gtd".to_owned(),
        points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("navigation.gtd")),
        FileMeta::default(),
        vec![],
    )
}

#[test]
fn navigation_intervals_and_markers_use_the_same_state_observations() {
    let points: Vec<_> = [
        (0, SolutionState::Fix),
        (10, SolutionState::NoFix),
        (30, SolutionState::Fix),
        (40, SolutionState::Unknown),
        (50, SolutionState::EstimatedOnly),
        (60, SolutionState::Fix),
    ]
    .into_iter()
    .map(|(second, state)| {
        StateSample {
            second,
            state: Some(state),
        }
        .point()
    })
    .collect();
    let file = build_file(&points);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(
        stats,
        FixStats {
            time_with_fix: Duration::seconds(20),
            time_without_fix: Duration::seconds(20),
            time_estimated_only: Duration::seconds(10),
            time_unknown: Duration::seconds(10),
            fix_loss_count: 1,
            max_continuous_no_fix: Duration::seconds(20),
        }
    );
    assert_eq!(stats.availability(), Some(0.4));
    assert!((stats.known_state_coverage() - 50.0 / 60.0).abs() < f64::EPSILON);
    let track = file.tracks.first().expect("one track");
    let markers: Vec<_> = track
        .generated_markers
        .iter()
        .map(|marker| (marker.time, marker.kind.clone(), marker.lat, marker.lon))
        .collect();
    assert_eq!(
        markers,
        vec![
            (
                test_util::time_at_second(0),
                GeneratedMarkerKind::GnssFixLost,
                points
                    .first()
                    .expect("first point")
                    .tpv
                    .position()
                    .expect("position")
                    .0,
                points
                    .first()
                    .expect("first point")
                    .tpv
                    .position()
                    .expect("position")
                    .1
            ),
            (
                test_util::time_at_second(30),
                GeneratedMarkerKind::GnssFixRegained {
                    fix_lost_duration: Duration::seconds(30)
                },
                points
                    .get(2)
                    .expect("recovering point")
                    .tpv
                    .position()
                    .expect("position")
                    .0,
                points
                    .get(2)
                    .expect("recovering point")
                    .tpv
                    .position()
                    .expect("position")
                    .1
            ),
        ]
    );
}

#[rstest]
#[case::absent(None)]
#[case::explicit_unknown(Some(SolutionState::Unknown))]
fn positive_duration_without_known_state_is_unknown(#[case] state: Option<SolutionState>) {
    let points = [
        StateSample { second: 0, state }.point(),
        StateSample { second: 10, state }.point(),
    ];
    let file = build_file(&points);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.time_unknown, Duration::seconds(10));
    assert_eq!(stats.known_duration(), Duration::zero());
    assert_eq!(stats.availability(), None);
    assert!(stats.known_state_coverage().abs() < f64::EPSILON);
    assert!(
        file.tracks
            .first()
            .expect("track")
            .generated_markers
            .is_empty()
    );
}

#[test]
fn sparse_state_observations_cover_intermediate_positions_and_preserve_unknown_ends() {
    let mut points: Vec<_> = (-10..=60)
        .map(|tenth| {
            let time = test_util::time_at_second(0) + Duration::milliseconds(tenth * 100);
            let mut point = test_util::fix_without_a_satellite_report(0);
            point.tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(time))
                .lat(point.tpv.lat())
                .lon(point.tpv.lon())
                .maybe_heading(point.tpv.heading())
                .build();
            point
        })
        .collect();
    for index in [10, 60] {
        points
            .get_mut(index)
            .expect("evidence point")
            .navigation
            .state = Some(SolutionState::Fix);
    }
    let stats = segment::compute_fix_stats(&points).expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(5));
    assert_eq!(stats.time_unknown, Duration::seconds(2));
    assert!((stats.known_state_coverage() - 5.0 / 7.0).abs() < f64::EPSILON);
    assert_eq!(FixStats::default().availability(), None);
    assert!(FixStats::default().known_state_coverage().abs() < f64::EPSILON);
}

#[test]
fn no_fix_and_estimated_only_form_one_continuous_absence() {
    let points: Vec<_> = [
        (0, SolutionState::Fix),
        (10, SolutionState::NoFix),
        (20, SolutionState::EstimatedOnly),
        (40, SolutionState::NoFix),
        (50, SolutionState::Fix),
    ]
    .into_iter()
    .map(|(second, state)| {
        StateSample {
            second,
            state: Some(state),
        }
        .point()
    })
    .collect();
    let file = build_file(&points);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(40));
    assert_eq!(stats.time_without_fix, Duration::seconds(20));
    assert_eq!(stats.time_estimated_only, Duration::seconds(20));
    assert_eq!(stats.fix_loss_count, 1);
    assert_eq!(
        file.tracks
            .first()
            .expect("track")
            .generated_markers
            .last()
            .expect("recovery")
            .kind,
        GeneratedMarkerKind::GnssFixRegained {
            fix_lost_duration: Duration::seconds(50)
        }
    );
}

#[rstest]
#[case::fix_first(SolutionState::Fix, SolutionState::Unknown, 10)]
#[case::unknown_first(SolutionState::Unknown, SolutionState::Fix, 0)]
fn equal_time_explicit_state_uses_first_recorded_point(
    #[case] first: SolutionState,
    #[case] second: SolutionState,
    #[case] fix_seconds: i64,
) {
    let mut fallback = test_util::fix_with_a_satellite_in_view_only(0);
    fallback.navigation.used_count = Some(0);
    let points = [
        fallback,
        StateSample {
            second: 0,
            state: Some(first),
        }
        .point(),
        StateSample {
            second: 0,
            state: Some(second),
        }
        .point(),
        StateSample {
            second: 10,
            state: Some(SolutionState::Fix),
        }
        .point(),
    ];
    let file = build_file(&points);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(fix_seconds));
    assert_eq!(stats.time_unknown, Duration::seconds(10 - fix_seconds));
    assert_eq!(stats.fix_loss_count, 0);
    assert!(
        file.tracks
            .first()
            .expect("track")
            .generated_markers
            .is_empty()
    );
}

#[test]
fn backward_steps_reset_evidence_intervals_and_loss_recovery() {
    let points: Vec<_> = [
        (0, SolutionState::Fix),
        (10, SolutionState::NoFix),
        (20, SolutionState::NoFix),
        (5, SolutionState::Fix),
        (15, SolutionState::EstimatedOnly),
        (25, SolutionState::EstimatedOnly),
    ]
    .into_iter()
    .map(|(second, state)| {
        StateSample {
            second,
            state: Some(state),
        }
        .point()
    })
    .collect();
    let file = build_file(&points);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(20));
    assert_eq!(stats.time_without_fix, Duration::seconds(10));
    assert_eq!(stats.time_estimated_only, Duration::seconds(10));
    assert_eq!(stats.time_unknown, Duration::zero());
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(10));
    assert_eq!(stats.fix_loss_count, 2);
    let markers = &file.tracks.first().expect("track").generated_markers;
    assert_eq!(
        markers
            .iter()
            .map(|marker| marker.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            GeneratedMarkerKind::GnssFixLost,
            GeneratedMarkerKind::GnssFixLost
        ]
    );
}

#[test]
fn file_aggregation_weights_durations_and_resets_at_track_boundaries() {
    let points: Vec<_> = [
        (0, Some(SolutionState::Fix)),
        (10, Some(SolutionState::NoFix)),
        (30, Some(SolutionState::EstimatedOnly)),
        (50, Some(SolutionState::NoFix)),
        (1000, Some(SolutionState::Fix)),
        (1010, Some(SolutionState::Unknown)),
        (1020, None),
    ]
    .into_iter()
    .map(|(second, state)| StateSample { second, state }.point())
    .collect();
    let file = build_file(&points);
    assert_eq!(file.tracks.len(), 2);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(20));
    assert_eq!(stats.time_without_fix, Duration::seconds(20));
    assert_eq!(stats.time_estimated_only, Duration::seconds(20));
    assert_eq!(stats.time_unknown, Duration::seconds(10));
    assert_eq!(stats.availability(), Some(1.0 / 3.0));
    assert!((stats.known_state_coverage() - 6.0 / 7.0).abs() < f64::EPSILON);
    assert_eq!(stats.fix_loss_count, 1);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(40));
    assert!(
        file.tracks
            .get(1)
            .expect("second track")
            .generated_markers
            .is_empty()
    );
}

struct UsageSample {
    second: i64,
    scope: SatelliteScope,
    count: Option<u32>,
    input_order: u64,
}

impl UsageSample {
    fn report(self) -> SatelliteUsageReport {
        let time = GpsTime::from_utc(test_util::time_at_second(self.second));
        SatelliteUsageReport {
            metadata: SatelliteReportMetadata {
                gps_time: Some(time),
                sys_time: None,
                normalized_time: time,
                input_order: self.input_order,
                scope: self.scope,
                coverage: SatelliteCoverage::Complete,
                reported_count: self.count,
            },
            satellites: vec![],
        }
    }
}

#[test]
fn associated_usage_observes_state_at_fix_time_and_observation_only_carriers_are_skipped() {
    let mut start = test_util::fix_without_a_satellite_report(0);
    let mut synthesized = test_util::fix_without_a_satellite_report(10);
    synthesized.navigation.position_provenance = Some(PositionProvenance::SdkSynthesized);
    let mut end = test_util::fix_without_a_satellite_report(20);
    for (point, count) in [(&mut start, 8), (&mut synthesized, 0), (&mut end, 8)] {
        let mut report = UsageSample {
            second: point.tpv.time().utc().timestamp(),
            scope: SatelliteScope::Global,
            count: Some(count),
            input_order: 0,
        }
        .report();
        report.metadata.normalized_time =
            GpsTime::from_utc(report.metadata.normalized_time.utc() + Duration::milliseconds(400));
        report.metadata.gps_time = Some(report.metadata.normalized_time);
        point.satellite_evidence.usage.push(report);
    }
    let baseline = build_file(&[start.clone(), synthesized.clone(), end.clone()]);
    let mut carrier = test_util::fix_without_a_satellite_report(5);
    carrier.navigation.position_provenance = Some(PositionProvenance::SdkSynthesized);
    let metadata = UsageSample {
        second: 5,
        scope: SatelliteScope::Global,
        count: None,
        input_order: 0,
    }
    .report()
    .metadata;
    carrier
        .satellite_evidence
        .observations
        .push(SatelliteObservationReport {
            metadata,
            satellites: vec![],
        });
    let file = build_file(&[start, carrier, synthesized, end]);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(file.metadata.fix_stats, baseline.metadata.fix_stats);
    assert_eq!(stats.time_with_fix, Duration::seconds(10));
    assert_eq!(stats.time_without_fix, Duration::seconds(10));
    assert_eq!(stats.fix_loss_count, 1);
    let markers: Vec<_> = file
        .tracks
        .first()
        .expect("track")
        .generated_markers
        .iter()
        .map(|marker| (marker.time, marker.kind.clone()))
        .collect();
    let baseline_markers: Vec<_> = baseline
        .tracks
        .first()
        .expect("track")
        .generated_markers
        .iter()
        .map(|marker| (marker.time, marker.kind.clone()))
        .collect();
    assert_eq!(markers, baseline_markers);
    assert_eq!(
        markers,
        vec![
            (
                test_util::time_at_second(0),
                GeneratedMarkerKind::GnssFixLost
            ),
            (
                test_util::time_at_second(20),
                GeneratedMarkerKind::GnssFixRegained {
                    fix_lost_duration: Duration::seconds(20)
                }
            ),
        ]
    );
}

#[rstest]
#[case::nearer_report(UsageSample { second: 1, scope: SatelliteScope::Global, count: Some(0), input_order: 0 }, UsageSample { second: 2, scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)), count: Some(8), input_order: 1 }, 0)]
#[case::narrower_scope(UsageSample { second: 0, scope: SatelliteScope::Global, count: Some(0), input_order: 0 }, UsageSample { second: 0, scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)), count: Some(8), input_order: 1 }, 10)]
#[case::earlier_time(UsageSample { second: 1, scope: SatelliteScope::Global, count: Some(0), input_order: 0 }, UsageSample { second: -1, scope: SatelliteScope::Global, count: Some(8), input_order: 1 }, 10)]
#[case::input_order(UsageSample { second: 0, scope: SatelliteScope::Global, count: Some(0), input_order: 1 }, UsageSample { second: 0, scope: SatelliteScope::Global, count: Some(8), input_order: 0 }, 10)]
#[case::aggregate_before_list(UsageSample { second: 0, scope: SatelliteScope::Global, count: None, input_order: 0 }, UsageSample { second: 1, scope: SatelliteScope::Global, count: Some(8), input_order: 1 }, 10)]
fn equal_time_fallback_observations_use_shared_report_precedence(
    #[case] first: UsageSample,
    #[case] second: UsageSample,
    #[case] fix_seconds: i64,
) {
    let mut first_point = test_util::fix_without_a_satellite_report(0);
    first_point.satellite_evidence.usage.push(first.report());
    let mut second_point = test_util::fix_without_a_satellite_report(0);
    second_point.satellite_evidence.usage.push(second.report());
    let points = [
        first_point,
        second_point,
        StateSample {
            second: 10,
            state: Some(SolutionState::Fix),
        }
        .point(),
    ];
    let file = build_file(&points);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.time_with_fix, Duration::seconds(fix_seconds));
    assert_eq!(stats.time_without_fix, Duration::seconds(10 - fix_seconds));
    assert!(
        file.tracks
            .first()
            .expect("track")
            .generated_markers
            .is_empty()
    );
}

#[test]
fn navigation_count_precedes_associated_counts_and_scoped_zero_supplies_no_state() {
    let gps_scope = SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    let mut own_zero = test_util::fix_without_a_satellite_report(0);
    own_zero.navigation.used_count = Some(0);
    own_zero.satellite_evidence.usage.push(
        UsageSample {
            second: 0,
            scope: gps_scope,
            count: Some(8),
            input_order: 0,
        }
        .report(),
    );
    let mut scoped_zero = test_util::fix_without_a_satellite_report(5);
    scoped_zero.satellite_evidence.usage.push(
        UsageSample {
            second: 5,
            scope: gps_scope,
            count: Some(0),
            input_order: 0,
        }
        .report(),
    );
    let file = build_file(&[
        own_zero,
        scoped_zero,
        StateSample {
            second: 10,
            state: Some(SolutionState::Fix),
        }
        .point(),
    ]);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.time_without_fix, Duration::seconds(10));
    assert_eq!(stats.time_unknown, Duration::zero());
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(10));
    assert!(
        file.tracks
            .first()
            .expect("track")
            .generated_markers
            .is_empty()
    );
}

#[test]
fn explicit_unknown_ends_an_observed_loss_chain() {
    let points: Vec<_> = [
        (0, SolutionState::Fix),
        (10, SolutionState::NoFix),
        (20, SolutionState::Unknown),
        (30, SolutionState::Fix),
    ]
    .into_iter()
    .map(|(second, state)| {
        StateSample {
            second,
            state: Some(state),
        }
        .point()
    })
    .collect();
    let file = build_file(&points);
    let stats = file.metadata.fix_stats.expect("positive duration");
    assert_eq!(stats.time_unknown, Duration::seconds(10));
    assert_eq!(stats.fix_loss_count, 1);
    let markers = &file.tracks.first().expect("track").generated_markers;
    assert_eq!(
        markers
            .iter()
            .map(|marker| marker.kind.clone())
            .collect::<Vec<_>>(),
        vec![GeneratedMarkerKind::GnssFixLost]
    );
}
