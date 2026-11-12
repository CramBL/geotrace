use chrono::{DateTime, TimeDelta, Utc};
use gt_types::{
    BootClockBackwardStep, BootTime, GpsTime, Latitude, Longitude, MetricKind, NavPoint, SysTime,
    SystemBootComparison, SystemBootTimePair, TimePositionVelocity,
};
use rstest::rstest;

use super::AnalysisConfig;

struct ClockSeriesFixture {
    times_nanos: &'static [i64],
    boot_micros: &'static [u64],
    missing_pairs: &'static [usize],
    backward_steps: &'static [usize],
    expected_runs: &'static [&'static [usize]],
}

#[rstest]
#[case::empty(ClockSeriesFixture {
    times_nanos: &[], boot_micros: &[], missing_pairs: &[], backward_steps: &[], expected_runs: &[],
})]
#[case::single_reference(ClockSeriesFixture {
    times_nanos: &[0], boot_micros: &[0], missing_pairs: &[], backward_steps: &[], expected_runs: &[&[0]],
})]
#[case::missing_pairs(ClockSeriesFixture {
    times_nanos: &[0, 1_000_000, 2_000_000, 3_000_000, 4_000_000],
    boot_micros: &[0, 0, 0, 0, 0],
    missing_pairs: &[0, 2], backward_steps: &[], expected_runs: &[&[1], &[3, 4]],
})]
#[case::boot_backward_step(ClockSeriesFixture {
    times_nanos: &[0, 1_000_000, 2_000_000, 3_000_000],
    boot_micros: &[0, 1, 0, 0],
    missing_pairs: &[], backward_steps: &[2], expected_runs: &[&[0, 1], &[2, 3]],
})]
#[case::non_increasing_plot_times(ClockSeriesFixture {
    times_nanos: &[0, 1_000_000, 1_000_000, 0, 2_000_000],
    boot_micros: &[0, 0, 0, 0, 0],
    missing_pairs: &[], backward_steps: &[], expected_runs: &[&[0, 1], &[2], &[3, 4]],
})]
#[case::rounded_plot_times(ClockSeriesFixture {
    times_nanos: &[0, 1, 1_000_000],
    boot_micros: &[0, 0, 0],
    missing_pairs: &[], backward_steps: &[], expected_runs: &[&[0], &[1, 2]],
})]
fn system_boot_series_preserves_samples_and_source_identity(#[case] fixture: ClockSeriesFixture) {
    let epoch = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
    let boot_time = BootTime::from_micros(0).unwrap();
    let reference = SystemBootTimePair {
        system_time: SysTime::from_utc(epoch),
        boot_time,
    };
    let points = fixture
        .times_nanos
        .iter()
        .enumerate()
        .map(|(index, &nanos)| {
            let Some(&boot_micros) = fixture.boot_micros.get(index) else {
                panic!("each fix has a boot reading");
            };
            let Some(&previous_boot_micros) = fixture.boot_micros.get(index.saturating_sub(1))
            else {
                panic!("the previous fix has a boot reading");
            };
            let boot_time = BootTime::from_micros(boot_micros).unwrap();
            let system_time = SysTime::from_utc(
                epoch + TimeDelta::microseconds(index as i64 + i64::try_from(boot_micros).unwrap()),
            );
            let pair = SystemBootTimePair {
                system_time,
                boot_time,
            };
            let comparison =
                (!fixture.missing_pairs.contains(&index)).then_some(SystemBootComparison {
                    reference,
                    delta: pair.system_boot_delta_since(reference),
                });
            let backward_step =
                fixture
                    .backward_steps
                    .contains(&index)
                    .then_some(BootClockBackwardStep {
                        previous_record: index.saturating_sub(1),
                        record: index,
                        previous_boot_time: BootTime::from_micros(previous_boot_micros).unwrap(),
                        boot_time,
                    });
            NavPoint::new(
                TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(epoch + TimeDelta::nanoseconds(nanos)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .boot_time(boot_time)
                    .maybe_sys_time(comparison.map(|_| system_time))
                    .maybe_system_boot_comparison(comparison)
                    .maybe_boot_clock_backward_step(backward_step)
                    .build(),
                None,
            )
        })
        .collect();
    let track = gt_test_utils::loaded_track_with_points(points);
    let series = super::build_track_series(0, &track, AnalysisConfig::default());
    let Some(metric) = series.metrics.get(&MetricKind::SystemBootDeltaMs) else {
        panic!("system/boot metric series is present");
    };
    assert_eq!(metric.runs.len(), fixture.expected_runs.len());
    for (run_index, (run, expected_sources)) in
        metric.runs.iter().zip(fixture.expected_runs).enumerate()
    {
        let samples = run.finest_level();
        assert_eq!(samples.len(), expected_sources.len());
        for (sample, &point_index) in samples.iter().zip(*expected_sources) {
            assert_eq!(sample.y, point_index as f64 / 1_000.0);
            assert_eq!(
                metric
                    .sample_source(run_index, sample.x)
                    .unwrap()
                    .point_index,
                point_index
            );
        }
    }
}
