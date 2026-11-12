use chrono::TimeDelta;
use gt_plot::PlotState;
use gt_plot::test_util::{self, PlotPosition, PlotSources};
use gt_types::{
    BootClockBackwardStep, BootTime, FixRef, GpsTime, Latitude, Longitude, MetricKind, NavPoint,
    PointIdx, SysTime, SystemBootComparison, SystemBootTimePair, TimePositionVelocity,
};
use strum::IntoEnumIterator as _;

#[test]
fn system_boot_hover_preserves_microseconds_source_and_repair_details() {
    let reference = SystemBootTimePair {
        system_time: SysTime::from_utc(test_util::at_second(0)),
        boot_time: BootTime::from_micros(1).unwrap(),
    };
    let points = (0..2)
        .map(|index| {
            let boot_time = BootTime::from_micros(1 - u64::try_from(index).unwrap()).unwrap();
            let pair = SystemBootTimePair {
                boot_time,
                ..reference
            };
            let step = (index == 1).then_some(BootClockBackwardStep {
                previous_record: 0,
                record: 1,
                previous_boot_time: reference.boot_time,
                boot_time,
            });
            NavPoint::new(
                TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(test_util::at_second(index)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .sys_time(pair.system_time)
                    .boot_time(boot_time)
                    .system_boot_comparison(SystemBootComparison {
                        reference,
                        delta: pair.system_boot_delta_since(reference),
                    })
                    .maybe_boot_clock_backward_step(step)
                    .debug_time_repair_threshold(TimeDelta::seconds(5))
                    .build(),
                None,
            )
        })
        .collect();
    let mut state = PlotState::default();
    for kind in MetricKind::iter() {
        state
            .metric_vis
            .set(kind, kind == MetricKind::SystemBootDeltaMs);
    }
    let mut plot = test_util::drawn_plot(
        vec![test_util::recording(points, Vec::new())],
        PlotSources::default(),
        state,
    );
    assert!(!plot.metric_chip_disabled("System/boot delta (ms)"));
    plot.hover(plot.screen_position(PlotPosition {
        offset_secs: 1.0,
        y: 0.001,
    }));
    let label = plot.hover_label();
    assert!(label.contains("+0.001ms (+1us)"), "{label}");
    assert!(
        label.contains("Reference system 2024-01-15 12:00:00.000000 UTC"),
        "{label}"
    );
    assert!(label.contains("Reference boot 1us"), "{label}");
    assert!(
        label.contains("Boot-clock backward step fix 1 (1us) -> fix 2 (0us)"),
        "{label}"
    );
    assert!(
        label.contains("debug time repair (threshold 5s)"),
        "{label}"
    );
    assert!(label.contains("synthetic shifts"), "{label}");
    assert_eq!(
        plot.state().hovered_metric_fix,
        Some(FixRef::new(test_util::track0(), PointIdx::new(1)))
    );
}

#[test]
fn system_boot_chip_is_discoverable_without_complete_pairs() {
    let plot = test_util::drawn_plot(
        vec![test_util::recording(test_util::fixes(2, 1), Vec::new())],
        PlotSources::default(),
        PlotState::default(),
    );
    assert!(plot.metric_chip_disabled("System/boot delta (ms)"));
}
