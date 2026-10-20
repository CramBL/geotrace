use chrono::TimeDelta;
use gt_egui_mipmap::SelectionRange;
use gt_types::satellites::{Constellation, ConstellationSet, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::{
    MetricKind, NavPoint, NavigationEvidenceSource, PositionProvenance, SatelliteCoverage,
    SatelliteObservationReport, SatelliteReportMetadata, SatelliteScope, SatelliteUsageReport,
};
use rstest::rstest;

use super::TrackSeries;
use crate::AnalysisConfig;

fn points_at(offsets: &[i64]) -> Vec<NavPoint> {
    offsets
        .iter()
        .map(|offset| {
            gt_test_utils::fixtures::nav_points_from(crate::test_util::at_second(*offset), 1, 1)
                .remove(0)
        })
        .collect()
}

fn metadata(millis: i64) -> SatelliteReportMetadata {
    let time = GpsTime::from_utc(crate::test_util::at_second(0) + TimeDelta::milliseconds(millis));
    SatelliteReportMetadata {
        gps_time: Some(time),
        sys_time: None,
        normalized_time: time,
        input_order: 0,
        scope: SatelliteScope::Global,
        coverage: SatelliteCoverage::Complete,
        reported_count: None,
    }
}

fn series(points: Vec<NavPoint>) -> TrackSeries {
    super::build_track_series(
        0,
        &gt_test_utils::loaded_track_with_points(points),
        AnalysisConfig::default(),
    )
}

fn drawn_runs(series: &TrackSeries, kind: MetricKind, target: usize) -> Vec<Vec<[f64; 2]>> {
    let base = crate::test_util::at_second(0).timestamp() as f64;
    series
        .metrics
        .get(&kind)
        .expect("count metric")
        .runs
        .iter()
        .map(|run| {
            let points = run.select_slice(
                SelectionRange::within_viewport(f64::NEG_INFINITY..=f64::INFINITY),
                target,
            );
            points
                .iter()
                .map(|point| [point.x - base, point.y])
                .collect()
        })
        .collect()
}

#[rstest]
#[case::summary(Some(8), None, SatelliteCoverage::Complete, false, vec![vec![[0.0, 8.0]]], vec![])]
#[case::complete_empty(None, None, SatelliteCoverage::Complete, true, vec![vec![[0.0, 0.0]]], vec![vec![[0.0, 0.0]]])]
#[case::partial_empty(None, None, SatelliteCoverage::Partial, true, vec![], vec![])]
#[case::summary_with_observations(Some(8), Some(12), SatelliteCoverage::Partial, true, vec![vec![[0.0, 8.0]]], vec![vec![[0.0, 12.0]]])]
#[case::partial_with_zero(None, Some(0), SatelliteCoverage::Partial, true, vec![vec![[0.0, 0.0]]], vec![vec![[0.0, 0.0]]])]
fn count_samples_preserve_presence_and_coverage(
    #[case] summary: Option<u32>,
    #[case] aggregate: Option<u32>,
    #[case] coverage: SatelliteCoverage,
    #[case] report_present: bool,
    #[case] used: Vec<Vec<[f64; 2]>>,
    #[case] seen: Vec<Vec<[f64; 2]>>,
) {
    let mut points = points_at(&[0]);
    let point = points.first_mut().expect("one point");
    point.navigation.used_count = summary;
    if report_present {
        let metadata = SatelliteReportMetadata {
            coverage,
            reported_count: aggregate,
            ..metadata(0)
        };
        point.satellite_evidence.usage.push(SatelliteUsageReport {
            metadata,
            satellites: Vec::new(),
        });
        point
            .satellite_evidence
            .observations
            .push(SatelliteObservationReport {
                metadata,
                satellites: Vec::new(),
            });
    }
    let series = series(points);
    assert_eq!(drawn_runs(&series, MetricKind::SatsFix, 2), used);
    assert_eq!(drawn_runs(&series, MetricKind::SatsSeen, 2), seen);
}

#[test]
fn scoped_counts_require_exact_aggregate_scope_or_complete_lists() {
    let mut points = points_at(&[0]);
    let point = points.first_mut().expect("one point");
    point.satellite_evidence.usage.push(SatelliteUsageReport {
        metadata: SatelliteReportMetadata {
            scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
            reported_count: Some(8),
            ..metadata(0)
        },
        satellites: Vec::new(),
    });
    let series = series(points);
    assert_eq!(
        drawn_runs(&series, MetricKind::GpsFix, 2),
        [vec![[0.0, 8.0]]]
    );
    assert!(drawn_runs(&series, MetricKind::SatsFix, 2).is_empty());
    assert!(drawn_runs(&series, MetricKind::GlonassFix, 2).is_empty());
    assert!(series.present.contains(Constellation::Gps));
}

#[test]
fn source_positions_without_counts_split_sparse_polling_at_every_level() {
    let mut points = points_at(&[0, 1, 2, 3, 4, 5, 6]);
    for index in [0, 5] {
        points
            .get_mut(index)
            .expect("source point")
            .satellite_evidence
            .usage
            .push(SatelliteUsageReport {
                metadata: SatelliteReportMetadata {
                    reported_count: Some(8),
                    ..metadata(index as i64 * 1_000)
                },
                satellites: Vec::new(),
            });
    }
    let series = series(points);
    for target in [2, 400, usize::MAX] {
        assert_eq!(
            drawn_runs(&series, MetricKind::SatsFix, target),
            [vec![[0.0, 8.0]], vec![[5.0, 8.0]]]
        );
    }
}

#[rstest]
#[case::other_kind(None, vec![vec![[0.0, 8.0], [2.0, 8.0]]])]
#[case::other_scope(Some(Constellation::Glonass), vec![vec![[0.0, 8.0], [2.0, 8.0]]])]
#[case::partial_same_scope(Some(Constellation::Gps), vec![vec![[0.0, 8.0]], vec![[2.0, 8.0]]])]
fn orphan_reports_split_only_relevant_count_runs(
    #[case] usage_scope: Option<Constellation>,
    #[case] expected: Vec<Vec<[f64; 2]>>,
) {
    let mut points = points_at(&[0, 1, 2]);
    for index in [0, 2] {
        points
            .get_mut(index)
            .expect("source point")
            .satellite_evidence
            .usage
            .push(SatelliteUsageReport {
                metadata: SatelliteReportMetadata {
                    scope: SatelliteScope::Constellations(ConstellationSet::single(
                        Constellation::Gps,
                    )),
                    reported_count: Some(8),
                    ..metadata(index as i64 * 1_000)
                },
                satellites: Vec::new(),
            });
    }
    let middle = points.get_mut(1).expect("carrier");
    middle.navigation.position_provenance = Some(PositionProvenance::SdkSynthesized);
    if let Some(constellation) = usage_scope {
        middle.satellite_evidence.usage.push(SatelliteUsageReport {
            metadata: SatelliteReportMetadata {
                scope: SatelliteScope::Constellations(ConstellationSet::single(constellation)),
                coverage: SatelliteCoverage::Partial,
                ..metadata(1_000)
            },
            satellites: Vec::new(),
        });
    } else {
        middle
            .satellite_evidence
            .observations
            .push(SatelliteObservationReport {
                metadata: metadata(1_000),
                satellites: Vec::new(),
            });
    }
    let series = series(points);
    assert_eq!(drawn_runs(&series, MetricKind::GpsFix, 2), expected);
}

#[test]
#[expect(clippy::float_cmp, reason = "Reported integer counts are unchanged")]
fn reports_preserve_fractional_times_and_independent_samples() {
    let mut points = points_at(&[10]);
    let point = points.first_mut().expect("source point");
    for (index, millis) in [10_100, 10_400].into_iter().enumerate() {
        point.satellite_evidence.usage.push(SatelliteUsageReport {
            metadata: SatelliteReportMetadata {
                reported_count: Some(8),
                input_order: index as u64,
                ..metadata(millis)
            },
            satellites: Vec::new(),
        });
    }
    let series = series(points);
    let samples = drawn_runs(&series, MetricKind::SatsFix, usize::MAX);
    let expected = [[10.1, 8.0], [10.4, 8.0]];
    for (sample, expected) in samples.first().expect("one run").iter().zip(expected) {
        assert!((sample[0] - expected[0]).abs() < 0.000_001);
        assert_eq!(sample[1], expected[1]);
    }
    assert_eq!(samples.iter().map(Vec::len).sum::<usize>(), 2);
    let base = crate::test_util::at_second(0).timestamp() as f64;
    assert_eq!(series.x_range, Some((base + 10.0, base + 10.4)));
}

#[test]
fn coincident_summary_precedes_report_without_changing_recorded_aggregate() {
    let mut points = points_at(&[0]);
    let point = points.first_mut().expect("source point");
    point.navigation.used_count = Some(8);
    point.satellite_evidence.usage.push(SatelliteUsageReport {
        metadata: SatelliteReportMetadata {
            reported_count: Some(9),
            ..metadata(0)
        },
        satellites: Vec::new(),
    });
    let output = series(points.clone());
    assert_eq!(
        drawn_runs(&output, MetricKind::SatsFix, 2),
        [vec![[0.0, 8.0]]]
    );
    assert_eq!(
        points
            .first()
            .expect("source point")
            .satellite_evidence
            .usage
            .first()
            .expect("report")
            .metadata
            .reported_count,
        Some(9)
    );
}

#[rstest]
#[case::navigation(vec![0, 1, 0, 1], vec![0, 1_000, 0, 1_000])]
#[case::reports(vec![0, 1, 2, 3], vec![0, 1_000, 0, 1_000])]
#[case::one_fix(vec![0], vec![0, 1_000, 0, 1_000])]
fn backward_steps_preserve_distinct_runs(
    #[case] offsets: Vec<i64>,
    #[case] report_millis: Vec<i64>,
) {
    let mut points = points_at(&offsets);
    for (index, millis) in report_millis.into_iter().enumerate() {
        let position = if points.len() == 1 { 0 } else { index };
        points
            .get_mut(position)
            .expect("point")
            .satellite_evidence
            .usage
            .push(SatelliteUsageReport {
                metadata: SatelliteReportMetadata {
                    input_order: index as u64,
                    reported_count: Some(index as u32),
                    ..metadata(millis)
                },
                satellites: Vec::new(),
            });
    }
    let series = series(points);
    assert_eq!(
        drawn_runs(&series, MetricKind::SatsFix, 2),
        [vec![[0.0, 0.0], [1.0, 1.0]], vec![[0.0, 2.0], [1.0, 3.0]]]
    );
}

#[test]
#[expect(
    clippy::float_cmp,
    reason = "Selected samples retain their original values and timestamps"
)]
fn decimated_samples_retain_original_report_references() {
    let mut points = points_at(&[0]);
    for index in 0_u32..2_000 {
        points
            .first_mut()
            .expect("point")
            .satellite_evidence
            .usage
            .push(SatelliteUsageReport {
                metadata: SatelliteReportMetadata {
                    input_order: u64::from(index),
                    reported_count: Some(index % 13),
                    ..metadata(i64::from(index) * 100)
                },
                satellites: Vec::new(),
            });
    }
    let series = series(points);
    let metric = series.metrics.get(&MetricKind::SatsFix).expect("metric");
    let run = metric.runs.first().expect("one run");
    let selected = run.select_slice(
        SelectionRange::within_viewport(f64::NEG_INFINITY..=f64::INFINITY),
        2,
    );
    assert!(selected.len() < 2_000);
    for sample in selected {
        let source = metric.sample_source(0, sample.x).expect("original source");
        let NavigationEvidenceSource::UsageAggregate(index) = source.evidence else {
            panic!("report aggregate");
        };
        let report = metadata(index as i64 * 100);
        assert_eq!(source.point_index, 0);
        assert_eq!(
            sample.x,
            report.normalized_time.as_secs_f64_with_subseconds()
        );
        assert_eq!(sample.y, (index % 13) as f64);
    }
}

#[test]
fn diagnostic_runs_preserve_unavailable_epochs_and_report_times() {
    let points: Vec<_> = points_at(&[0, 1, 2, 3, 4])
        .into_iter()
        .enumerate()
        .map(|(index, point)| {
            let time = GpsTime::from_utc(point.tpv.time().utc() + TimeDelta::milliseconds(200));
            let elevation = (index != 2).then_some(45.0);
            let satellites = Satellites::new(
                Some(time),
                None,
                vec![Satellite::new(
                    Constellation::Gps,
                    1,
                    elevation,
                    Some(90.0),
                    Some(40.0),
                    true,
                )],
            );
            NavPoint::new(point.tpv, Some(satellites))
        })
        .collect();
    let series = series(points);
    let utilization = drawn_runs(&series, MetricKind::UtilGps, 2);
    let slips = drawn_runs(&series, MetricKind::SlipGps, 2);
    assert_eq!(utilization.iter().map(Vec::len).collect::<Vec<_>>(), [2, 2]);
    assert_eq!(slips.iter().map(Vec::len).collect::<Vec<_>>(), [1, 1]);
    for run in utilization.iter().chain(&slips) {
        assert!(run.iter().all(|sample| (sample[0] - 2.2).abs() > 0.001));
        assert!(
            run.iter()
                .all(|sample| (sample[0].fract() - 0.2).abs() < 0.000_001)
        );
    }
    assert!(
        utilization
            .iter()
            .flatten()
            .all(|sample| (sample[1] - 100.0).abs() < 1e-9)
    );
    assert!(slips.iter().flatten().all(|sample| sample[1] == 0.0));
}
