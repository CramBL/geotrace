use super::*;

#[test]
fn a_channel_aggregate_reduces_native_samples_in_the_window_span() {
    // Points at 0,1,2 s, with `@accel` sampled finer than the points. A count
    // `window 3` spans the closed point extent [t(0), t(2)] = [0, 2], holding
    // all five accel samples. Sample values are base units (m/s2) per the
    // `channel_span` contract, near 1g here. The peak 10.8 clears 1.0 g
    // (9.81 m/s2), so the whole track matches.
    let schema = test_util::schema_with("accel", Some("g"), None);
    let provider = TestProvider::new(3).indexed_time().with_channel(
        "accel",
        vec![
            (0.0, 9.6),
            (0.5, 10.3),
            (1.0, 10.8),
            (1.5, 10.0),
            (2.0, 9.7),
        ],
    );
    let output = test_util::run_channel(
        "points | window 3 | where max(@accel) > 1.0 g",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

#[test]
fn a_channel_reduces_more_samples_than_points() {
    // std over a `window 2` (span [0, 1]) reduces all five native accel
    // samples, not the 2 points. Values are base units (m/s2). A flat channel
    // has std 0 (< 0.02 g = 0.196 m/s2). A jumpy one does not.
    let schema = test_util::schema_with("accel", Some("g"), None);
    let flat = TestProvider::new(2).indexed_time().with_channel(
        "accel",
        vec![(0.0, 9.8), (0.25, 9.8), (0.5, 9.8), (0.75, 9.8), (1.0, 9.8)],
    );
    let calm = test_util::run_channel(
        "points | window 2 | where std(@accel) < 0.02 g",
        &schema,
        &flat,
    );
    assert_eq!(calm.matches[0].ranges, vec![0..2]);

    let jumpy = TestProvider::new(2).indexed_time().with_channel(
        "accel",
        vec![
            (0.0, 9.0),
            (0.25, 10.5),
            (0.5, 9.0),
            (0.75, 10.5),
            (1.0, 9.0),
        ],
    );
    let shaky = test_util::run_channel(
        "points | window 2 | where std(@accel) < 0.02 g",
        &schema,
        &jumpy,
    );
    assert!(shaky.matches.is_empty());
}

#[test]
fn a_window_with_no_channel_samples_is_reported_as_skipped() {
    // The channel has no samples in the window's span, so the aggregate is
    // missing: the window is skipped, attributed to the channel by name.
    let schema = test_util::schema_with("accel", Some("g"), None);
    let provider = TestProvider::new(2)
        .indexed_time()
        .with_channel("accel", vec![(100.0, 9.8)]); // far outside [0, 1]
    let output = test_util::run_channel(
        "points | window 2 | where max(@accel) > 0.5 g",
        &schema,
        &provider,
    );
    assert!(output.matches.is_empty());
    assert_eq!(output.summary.skipped_channels.get("accel"), Some(&1));
}

#[test]
fn channel_per_sample_arithmetic_reduces_within_the_timeline() {
    // `@accel * 2` is per-sample math within the channel's own clock: each
    // base-unit sample doubled, then reduced. max(2*5.2) = 10.4 clears
    // 1.0 g (9.81 m/s2).
    let schema = test_util::schema_with("accel", Some("g"), None);
    let provider = TestProvider::new(2)
        .indexed_time()
        .with_channel("accel", vec![(0.0, 4.0), (0.5, 5.2), (1.0, 4.5)]);
    let output = test_util::run_channel(
        "points | window 2 | where max(@accel * 2) > 1.0 g",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..2]);
}

#[test]
fn a_vector_component_reduces_its_own_column() {
    // @accel.y reduces the middle column, not x or z. A `window 3` spans the
    // three points' closed time extent [0, 2], holding all three samples.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let provider = TestProvider::new(3).indexed_time().with_vector_channel(
        "accel",
        vec![
            (0.0, vec![9.0, 10.5, 9.5]),
            (1.0, vec![9.1, 11.0, 9.4]),
            (2.0, vec![9.2, 10.8, 9.6]),
        ],
    );
    // The y column peaks at 11.0, clearing 1.0 g (9.81 m/s2), so all match.
    let y = test_util::run_channel(
        "points | window 3 | where max(@accel.y) > 1.0 g",
        &schema,
        &provider,
    );
    assert_eq!(y.matches[0].ranges, vec![0..3]);
    // The x column peaks at 9.2, below 1.0 g, so nothing matches - proving
    // the reduction reads x's column, not whichever column happens to clear.
    let x = test_util::run_channel(
        "points | window 3 | where max(@accel.x) > 1.0 g",
        &schema,
        &provider,
    );
    assert!(x.matches.is_empty());
}

#[test]
fn norm_reduces_the_per_sample_vector_magnitude() {
    // norm(@accel) is sqrt(x²+y²+z²) per sample, on base-unit rows. Row 0 is
    // (3,4,0) -> 5 m/s2. The rest are near zero. 0.1 g is 0.981 m/s2, so
    // max(norm) = 5 clears it and min does not. Unit "g", which the schema
    // types as an acceleration.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let provider = TestProvider::new(2).indexed_time().with_vector_channel(
        "accel",
        vec![(0.0, vec![3.0, 4.0, 0.0]), (1.0, vec![0.1, 0.0, 0.0])],
    );
    let hit = test_util::run_channel(
        "points | window 2 | where max(norm(@accel)) > 0.1 g",
        &schema,
        &provider,
    );
    assert_eq!(hit.matches[0].ranges, vec![0..2]);
    let miss = test_util::run_channel(
        "points | window 2 | where min(norm(@accel)) > 0.1 g",
        &schema,
        &provider,
    );
    assert!(miss.matches.is_empty());
}

#[test]
fn norm_of_a_non_finite_sample_poisons_the_window() {
    // A component of f64::MAX squares to inf, so norm is non-finite: the
    // window poisons, matches nothing, and the skip is counted.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y"]);
    let provider = TestProvider::new(1)
        .indexed_time()
        .with_vector_channel("accel", vec![(0.0, vec![f64::MAX, 0.0])]);
    let output = test_util::run_channel(
        "points | window 1 | where max(norm(@accel)) > 0.1 g",
        &schema,
        &provider,
    );
    assert!(output.matches.is_empty());
    assert_eq!(output.summary.skipped_non_finite, 1);
}

#[test]
fn components_combine_per_sample_within_the_row() {
    // sqrt(x² + y²) is per-sample math across two columns of the same row.
    // Row 0 is (3, 4) -> 5 m/s2, clearing 0.1 g (0.981 m/s2). The explicit
    // form matches what norm computes over those columns.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y"]);
    let provider = TestProvider::new(2)
        .indexed_time()
        .with_vector_channel("accel", vec![(0.0, vec![3.0, 4.0]), (1.0, vec![0.1, 0.0])]);
    let output = test_util::run_channel(
        "points | window 2 | where max(sqrt(@accel.x² + @accel.y²)) > 0.1 g",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..2]);
}

#[test]
fn a_duration_window_gathers_channel_samples_over_its_span() {
    // A `window 2 s` at anchor 0 spans [0, 2] s (the declared extent, closed
    // at the top), the only anchor whose full duration fits. The sample at
    // exactly 2.0 s is gathered though point 2 (at t=2) is not in the
    // half-open [0, 2) point window - the boundary difference the code flags.
    // The span's peak 10.8 clears 1.0 g, so the window's points 0 and 1 match.
    let schema = test_util::schema_with("accel", Some("g"), None);
    let provider = TestProvider::new(3).indexed_time().with_channel(
        "accel",
        vec![(0.0, 9.6), (0.7, 10.8), (1.4, 10.0), (2.0, 9.7)],
    );
    let output = test_util::run_channel(
        "points | window 2 s | where max(@accel) > 1.0 g",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..2]);
}

/// A slice keeps whole rows: every column of a sample comes with its time.
#[rstest]
#[case::inside(1..3, vec![1.0, 2.0], vec![1.0, 1.1, 2.0, 2.1])]
#[case::empty(1..1, vec![], vec![])]
#[case::past_the_end(2..4, vec![], vec![])]
fn a_timeline_slice_holds_the_rows_of_its_range(
    #[case] rows: std::ops::Range<usize>,
    #[case] times: Vec<f64>,
    #[case] values: Vec<f64>,
) {
    let timeline = ChannelTimeline {
        times: vec![0.0, 1.0, 2.0],
        values: vec![0.0, 0.1, 1.0, 1.1, 2.0, 2.1],
        columns: 2,
    };

    let slice = timeline.slice_rows(rows);

    assert_eq!(slice.times, times);
    assert_eq!(slice.values, values);
    assert_eq!(slice.columns, 2);
}

#[test]
fn a_channel_span_includes_samples_on_both_endpoints() {
    // The span is closed at both ends, so samples at exactly `t_lo` and `t_hi`
    // both count. A count `window 3` spans [0, 2]. The endpoints 9 and 11 are
    // the min and max, so spread = 2 clears the threshold. Dropping either
    // endpoint drops the spread to 1 and the window no longer matches, so the
    // assertion pins both bounds. A unitless channel keeps the math plain.
    let schema = test_util::schema_with("sensor", None, None);
    let provider = TestProvider::new(3)
        .indexed_time()
        .with_channel("sensor", vec![(0.0, 9.0), (1.0, 10.0), (2.0, 11.0)]);
    let output = test_util::run_channel(
        "points | window 3 | where spread(@sensor) > 1.5",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

/// The window's aggregate reads the sample at 5 s, between its points. The
/// third point steps the clock back to 1 s, and the points run from 0 s to
/// 10 s.
#[test]
fn a_count_window_spans_the_time_extent_of_its_points_across_a_backward_time_step() {
    let schema = test_util::schema_with("sensor", None, None);
    let provider = TestProvider::new(3)
        .with(QueryMetric::Time, vec![Some(0.0), Some(10.0), Some(1.0)])
        .with_channel("sensor", vec![(5.0, 10.0)]);
    let output = test_util::run_channel(
        "points | window 3 | where max(@sensor) > 5",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

/// Nothing but the provider keeps a filtered point out of the match: a window
/// whose condition reads a channel aggregate alone reads none of its points.
/// The window matches on the sample at 0.5 s and marks its other two points.
#[test]
fn a_window_marks_only_the_points_its_provider_offers() {
    let schema = test_util::schema_with("sensor", None, None);
    let provider = TestProvider::new(3)
        .indexed_time()
        .with_channel("sensor", vec![(0.5, 10.0)])
        .filtering_out(1);
    let output = test_util::run_channel(
        "points | window 3 | where max(@sensor) > 5",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..1, 2..3]);
}

/// The window has no span to gather samples over: no point of it has a
/// timestamp. Its channel aggregate reports a missing time.
#[test]
fn a_window_whose_points_have_no_time_skips_on_the_time_metric() {
    let schema = test_util::schema_with("sensor", None, None);
    let provider = TestProvider::new(2).with_channel("sensor", vec![(0.5, 10.0)]);
    let output = test_util::run_channel(
        "points | window 2 | where max(@sensor) > 5",
        &schema,
        &provider,
    );
    assert!(output.matches.is_empty());
    assert_eq!(output.summary.skipped.get(&QueryMetric::Time), Some(&1));
}
