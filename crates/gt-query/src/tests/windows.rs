use super::*;

#[test]
fn a_count_window_checks_to_window_count() {
    assert_eq!(
        test_util::checked("points | window 5 | where avg(velocity) > 30 km/h").window(),
        Some(Window::Count(NonZeroUsize::new(5).unwrap()))
    );
    assert_eq!(
        test_util::checked("points | where velocity > 30 km/h").window(),
        None
    );
}

#[test]
fn a_duration_window_checks_to_seconds() {
    assert_eq!(
        test_util::checked("points | window 15 s | where avg(velocity) > 30 km/h").window(),
        Some(Window::Duration(15.0))
    );
    // Units convert to seconds: 2 min = 120 s.
    assert_eq!(
        test_util::checked("points | window 2 min | where avg(velocity) > 30 km/h").window(),
        Some(Window::Duration(120.0))
    );
}

#[test]
fn matched_points_count_points_not_windows() {
    // Windows [1,3) and [2,4) both pass, so points 1..=4 (four points)
    // match even though only two windows did.
    let provider = TestProvider::new(6).with(
        QueryMetric::Velocity,
        vec![
            Some(0.0),
            Some(11.0),
            Some(12.0),
            Some(13.0),
            Some(11.0),
            Some(0.0),
        ],
    );
    let output = test_util::run_one(
        "points | window 3 | where avg(velocity) > 36 km/h",
        &provider,
    );
    assert_eq!(output.summary.matched_points, 4);
    assert_eq!(output.summary.total_points, 6);
}

#[test]
fn overlapping_windows_merge_into_one_match() {
    // Windows starting at 1 and 2 pass (avg > 10), so points 1..=4 merge.
    let provider = TestProvider::new(6).with(
        QueryMetric::Velocity,
        vec![
            Some(0.0),
            Some(11.0),
            Some(12.0),
            Some(13.0),
            Some(11.0),
            Some(0.0),
        ],
    );
    let output = test_util::run_one(
        "points | window 3 | where avg(velocity) > 36 km/h",
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![1..5]);
    assert_eq!(output.summary.match_count, 1);
}

#[test]
fn a_duration_window_reduces_the_points_in_its_time_span() {
    // Points at 0..5 s. A `window 2 s` at anchor i spans [t[i], t[i]+2), so
    // two points, and needs the full 2 s to fit (last anchor is point 2).
    let provider = TestProvider::new(5).indexed_time().with(
        QueryMetric::Velocity,
        vec![Some(10.0), Some(10.0), Some(0.0), Some(0.0), Some(0.0)],
    );
    let output = test_util::run_one(
        "points | window 2 s | where avg(velocity) > 5 km/h",
        &provider,
    );
    // Anchor 0 (pts 0,1 avg 10) and anchor 1 (pts 1,2 avg 5 m/s) clear the
    // 5 km/h bar. Anchor 2 (pts 2,3 avg 0) does not, and anchor 3 doesn't fit.
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

#[test]
fn a_duration_window_longer_than_the_track_matches_nothing() {
    // Track spans only 2 s (points at 0, 1, 2), so a 10 s window never fits.
    let provider = TestProvider::new(3).indexed_time().with(
        QueryMetric::Velocity,
        vec![Some(10.0), Some(10.0), Some(10.0)],
    );
    let output = test_util::run_one(
        "points | window 10 s | where avg(velocity) > 0 km/h",
        &provider,
    );
    assert!(output.matches.is_empty());
    assert_eq!(output.summary.tracks_with_no_room_for_the_window, 1);
}

#[test]
fn a_fractional_duration_window_spans_sub_second() {
    // Points at 0, 0.4, 0.8, 1.2 s. A 0.5 s window at anchor 0 spans [0, 0.5),
    // holding points 0 and 1. The anchor at 0.8 s reaches to 1.3 s, past the
    // last point at 1.2 s, so only anchors 0 and 1 fit.
    let provider = TestProvider::new(4)
        .with(
            QueryMetric::Time,
            vec![Some(0.0), Some(0.4), Some(0.8), Some(1.2)],
        )
        .with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(10.0), Some(0.0), Some(0.0)],
        );
    let output = test_util::run_one(
        "points | window 0.5 s | where avg(velocity) > 5 km/h",
        &provider,
    );
    // Anchor 0 (pts 0,1 avg 10, match). Anchor 1 (pts 1,2 avg 5 m/s, match).
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

#[test]
fn a_duration_window_survives_a_backward_time_step() {
    // A backward clock jump at point 2 (times 0, 1, 0, 1 s): the crate does
    // not assume monotonic time, so anchors after the jump must still be
    // evaluated, not silently dropped.
    let provider = TestProvider::new(4)
        .with(
            QueryMetric::Time,
            vec![Some(0.0), Some(1.0), Some(0.0), Some(1.0)],
        )
        .with(
            QueryMetric::Velocity,
            vec![Some(0.0), Some(0.0), Some(10.0), Some(10.0)],
        );
    // window 2 s never fits (max time is 1 s), but the fast pair after the
    // jump proves the loop reaches them: with window 1 s, anchor 2 spans
    // [0,1) = point 2 (avg 10, match), which a `break` would have skipped.
    let output = test_util::run_one(
        "points | window 1 s | where avg(velocity) > 5 km/h",
        &provider,
    );
    assert!(output.matches.iter().any(|m| m.ranges.contains(&(2..3))));
}

#[test]
fn a_duration_window_holds_the_points_of_one_chronological_run() {
    // The clock steps back at point 3, from 2 s to 0.1 s. The window at point 0
    // holds the points at 0 s and 1 s, both above the bar. The windows of the
    // second run hold points below it, and the windows at points 1 and 2 have
    // no room: their 2 s runs past 2 s, the last time of their own run.
    let provider = TestProvider::new(7)
        .with(
            QueryMetric::Time,
            vec![
                Some(0.0),
                Some(1.0),
                Some(2.0),
                Some(0.1),
                Some(1.1),
                Some(2.1),
                Some(3.1),
            ],
        )
        .with(
            QueryMetric::Velocity,
            vec![
                Some(10.0),
                Some(10.0),
                Some(10.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
                Some(0.0),
            ],
        );
    let output = test_util::run_one(
        "points | window 2 s | where avg(velocity) > 5 km/h",
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..2]);
}

#[test]
fn a_duration_window_has_no_room_in_a_run_shorter_than_itself() {
    // The clock steps back at point 2, from 11 s to 0 s. No anchor of either run
    // has room for a 2 s window: each run spans a second, however far the
    // track's largest time reaches.
    let provider = TestProvider::new(4)
        .with(
            QueryMetric::Time,
            vec![Some(10.0), Some(11.0), Some(0.0), Some(1.0)],
        )
        .with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(10.0), Some(10.0), Some(10.0)],
        );
    let output = test_util::run_one(
        "points | window 2 s | where avg(velocity) > 5 km/h",
        &provider,
    );
    assert!(output.matches.is_empty());
    assert_eq!(output.summary.tracks_with_no_room_for_the_window, 1);
}

#[test]
fn a_channel_source_duration_window_holds_the_samples_of_one_chronological_run() {
    // The channel's clock steps back at sample 3, from 2 s to 1 s, and only the
    // samples after the step are above the bar. The windows at samples 3 and 4
    // match. The windows at samples 5 and 6 have no room in their own run.
    let provider = TestProvider::new(0).with_channel(
        "sensor",
        vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (1.0, 10.0),
            (2.0, 10.0),
            (3.0, 10.0),
            (4.0, 10.0),
        ],
    );
    let query = check(
        &parse("@sensor | window 2 s | where max(@sensor) > 5").unwrap(),
        &test_util::schema_with("sensor", None, None),
    )
    .unwrap();

    let output = run(
        &query,
        &[TrackInput {
            track: test_util::track_ref(),
            provider: &provider,
        }],
    );

    assert_eq!(output.matches[0].ranges, vec![3..6]);
}

#[test]
fn a_duration_window_spans_real_time_not_point_count() {
    // Uneven spacing: points at 0, 1, 5, 6 s. A 2 s window at point 0 holds
    // only points 0 and 1 (point at 5 s is outside [0, 2)). The sparse
    // stretch is not force-filled to a fixed count.
    let provider = TestProvider::new(4)
        .with(
            QueryMetric::Time,
            vec![Some(0.0), Some(1.0), Some(5.0), Some(6.0)],
        )
        .with(
            QueryMetric::Velocity,
            vec![Some(10.0), Some(10.0), Some(0.0), Some(0.0)],
        );
    // window 2 s: anchor 0 → pts 0,1 (avg 10, match). Anchor 1 → t=1, 1+2=3
    // <= 6, pts with time in [1,3) = just point 1 (avg 10, match). Anchor 2
    // → t=5, 5+2=7 > 6, doesn't fit → break.
    let output = test_util::run_one(
        "points | window 2 s | where avg(velocity) > 5 km/h",
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..2]);
}

#[test]
fn short_track_is_reported_not_dropped() {
    let provider =
        TestProvider::new(3).with(QueryMetric::Velocity, vec![Some(1.0), Some(2.0), Some(3.0)]);
    let output = test_util::run_one(
        "points | window 10 | where avg(velocity) > 0 km/h",
        &provider,
    );
    assert!(output.matches.is_empty());
    assert_eq!(output.summary.tracks_with_no_room_for_the_window, 1);
}
