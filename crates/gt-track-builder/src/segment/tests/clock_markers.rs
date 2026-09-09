use chrono::{Duration, TimeZone as _, Utc};
use gt_types::markers::{GeneratedMarker, GeneratedMarkerKind};
use gt_types::nav_point::NavPoint;
use rstest::rstest;

use crate::segment::{
    self, DEFAULT_CLOCK_OUTLIER_SIGMAS, GeneratedMarkerConfig, MIN_CLOCK_SAMPLES,
};
use crate::test_util;

fn generated_markers_of(
    points: &[NavPoint],
    config: &GeneratedMarkerConfig,
) -> Vec<GeneratedMarker> {
    test_util::with_placed_points_of(points, |placed| {
        placed.map_or_else(Vec::new, |placed| {
            segment::detect_generated_markers(placed, config)
        })
    })
}

fn clock_discontinuities_of(points: &[NavPoint], sigmas: f64) -> Vec<GeneratedMarker> {
    test_util::with_placed_points_of(points, |placed| {
        placed.map_or_else(Vec::new, |placed| {
            segment::detect_clock_discontinuities(placed, sigmas, &[])
        })
    })
}

#[test]
fn clock_discontinuity_flags_suspend_boundary_once() {
    // Steady ~300 ms offset, then one sample whose system clock has jumped
    // ~2 h ahead (the device resumed from suspend) - the mortmobil.gtd case.
    let two_hours_ms = 2 * 3600 * 1000;
    let points = vec![
        test_util::fix_with_host_clock_ahead(1000, Duration::milliseconds(300)),
        test_util::fix_with_host_clock_ahead(1001, Duration::milliseconds(300)),
        test_util::fix_with_host_clock_ahead(1002, Duration::milliseconds(300)),
        test_util::fix_with_host_clock_ahead(1003, Duration::milliseconds(300)),
        test_util::fix_with_host_clock_ahead(1004, Duration::milliseconds(300 + two_hours_ms)),
    ];
    let markers = clock_discontinuities_of(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS);
    assert_eq!(
        markers.len(),
        1,
        "exactly one discontinuity at the boundary"
    );
    let marker = markers.first().expect("one marker");
    assert!(matches!(
        marker.kind,
        GeneratedMarkerKind::ClockDiscontinuity { .. }
    ));
    if let GeneratedMarkerKind::ClockDiscontinuity { step } = marker.kind {
        // System clock jumped 2 h ahead, so GPS−system dropped by 2 h.
        assert_eq!(step.num_milliseconds(), -two_hours_ms);
    }
    assert_eq!(
        marker.time,
        Utc.timestamp_opt(1004, 0).single().expect("valid")
    );
}

/// Steady 234 ms offset with one sample carrying a 1 h 09 m recording gap -
/// the `gnss.h5.gtd` case, where the receiver reported its pre-gap GPS epoch
/// for the first fix after resuming.
fn resume_from_gap_points() -> Vec<NavPoint> {
    vec![
        test_util::fix_with_host_clock_ahead(1000, Duration::milliseconds(210)),
        test_util::fix_with_host_clock_ahead(1001, Duration::milliseconds(227)),
        test_util::fix_with_host_clock_ahead(1002, Duration::milliseconds(240)),
        test_util::fix_with_host_clock_ahead(1003, Duration::milliseconds(234)),
        test_util::fix_with_host_clock_ahead(1004, Duration::milliseconds(4_127_054)),
        test_util::fix_with_host_clock_ahead(1005, Duration::milliseconds(240)),
        test_util::fix_with_host_clock_ahead(1006, Duration::milliseconds(215)),
        test_util::fix_with_host_clock_ahead(1007, Duration::milliseconds(235)),
    ]
}

#[test]
fn an_excursion_is_one_marker_not_a_pair_of_discontinuities() {
    let markers =
        generated_markers_of(&resume_from_gap_points(), &GeneratedMarkerConfig::default());
    let [marker] = markers.as_slice() else {
        panic!("expected exactly one marker, got {}", markers.len());
    };
    let GeneratedMarkerKind::ClockOffsetExcursion {
        deviation,
        offset,
        samples,
    } = marker.kind
    else {
        panic!("expected a clock offset excursion, got {:?}", marker.kind);
    };
    assert_eq!(offset.num_milliseconds(), -4_127_054);
    assert_eq!(deviation.num_milliseconds(), -4_126_820);
    assert_eq!(samples, 1);
    assert_eq!(
        marker.time,
        Utc.timestamp_opt(1004, 0).single().expect("valid"),
        "placed at the sample that departed furthest"
    );
}

#[test]
fn excursion_detection_off_leaves_the_discontinuity_markers() {
    let config = GeneratedMarkerConfig {
        detect_clock_offset_excursions: false,
        ..GeneratedMarkerConfig::default()
    };
    let markers = generated_markers_of(&resume_from_gap_points(), &config);
    assert!(
        markers.is_empty(),
        "the excursion sample stays out of the step series either way, so the \
         departure is never re-reported as a pair of jumps: {markers:?}"
    );
}

#[test]
fn a_permanent_offset_step_stays_a_discontinuity() {
    let mut points: Vec<NavPoint> = (0..6)
        .map(|i| test_util::fix_with_host_clock_ahead(1000 + i, Duration::milliseconds(200)))
        .collect();
    points.extend((6..12).map(|i| {
        test_util::fix_with_host_clock_ahead(1000 + i, Duration::milliseconds(3_600_000))
    }));
    let markers = generated_markers_of(&points, &GeneratedMarkerConfig::default());
    let [marker] = markers.as_slice() else {
        panic!("expected exactly one marker, got {}", markers.len());
    };
    assert!(matches!(
        marker.kind,
        GeneratedMarkerKind::ClockDiscontinuity { .. }
    ));
}

/// Five minutes of host-clock offset, far past the jitter of a healthy
/// clock and steady across the track.
const FIVE_MINUTES_MS: i64 = 5 * 60 * 1000;

/// No sample of a host clock offset that stands far from GPS across
/// the whole track is an outlier: the median is taken over the track's
/// own offsets.
#[rstest]
#[case::jitter_around_a_300_ms_offset(vec![300, 305, 298, 302])]
#[case::jitter_around_a_five_minute_offset(vec![
    FIVE_MINUTES_MS,
    FIVE_MINUTES_MS + 4,
    FIVE_MINUTES_MS - 3,
    FIVE_MINUTES_MS + 2,
    FIVE_MINUTES_MS - 5,
])]
fn clock_discontinuity_ignores_an_offset_series_without_an_outlier(#[case] sys_ahead_ms: Vec<i64>) {
    let points: Vec<NavPoint> = sys_ahead_ms
        .iter()
        .enumerate()
        .map(|(index, &ahead)| {
            test_util::fix_with_host_clock_ahead(1000 + index as i64, Duration::milliseconds(ahead))
        })
        .collect();

    assert!(clock_discontinuities_of(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS).is_empty());
}

#[test]
fn clock_discontinuity_needs_enough_samples() {
    // Below MIN_CLOCK_SAMPLES, detection is skipped even with an obvious 2 h
    // jump on the last sample - too few samples for a robust estimate.
    let two_hours_ms = 2 * 3600 * 1000;
    for count in 0..MIN_CLOCK_SAMPLES {
        let points: Vec<NavPoint> = (0..count)
            .map(|i| {
                let ahead = if i + 1 == count {
                    300 + two_hours_ms
                } else {
                    300
                };
                test_util::fix_with_host_clock_ahead(1000 + i as i64, Duration::milliseconds(ahead))
            })
            .collect();
        assert!(
            clock_discontinuities_of(&points, DEFAULT_CLOCK_OUTLIER_SIGMAS).is_empty(),
            "detection must be skipped with {count} samples (< {MIN_CLOCK_SAMPLES})"
        );
    }
}
