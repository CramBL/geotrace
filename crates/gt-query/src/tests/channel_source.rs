use super::*;

#[test]
fn a_channel_can_be_the_source() {
    // `@accel | ...` iterates the channel's own samples. A bare per-sample
    // predicate is fine here: the sample is the match granularity.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    check(
        &parse("@accel | where norm(@accel) > 1 g").unwrap(),
        &schema,
    )
    .expect("a channel source with a per-sample predicate checks");
    // Aggregated over a window of samples, too.
    check(
        &parse("@accel | window 5 | where std(norm(@accel)) < 0.02 g").unwrap(),
        &schema,
    )
    .expect("a windowed channel-source aggregate checks");
}

#[test]
fn a_channel_source_round_trips_through_the_formatter() {
    let src = "@accel\n| window 5\n| where (max(norm(@accel)) > 1 g)";
    let query = parse(src).unwrap();
    assert_eq!(query.to_string(), src);
}

#[rstest]
// A nav-point metric on a channel source has no interpolation yet.
#[case(
    "@accel | where velocity > 1 km/h",
    "velocity is not available on a channel source"
)]
// A table column is a nav metric too, rejected like a predicate one.
#[case(
    "@accel | where norm(@accel) > 1 g | table velocity",
    "velocity is not available on a channel source"
)]
// Only the source channel is on the timeline.
#[case(
    "@accel | window 3 | where max(@gyro.x) > 1 deg",
    "@gyro is not the source channel"
)]
// Windowed, the source channel must be aggregated like any other.
#[case("@accel | window 3 | where @accel.x > 1 g", "@accel.x is per sample")]
#[case(
    "@accel | window 3 | where norm(@accel) > 1 g",
    "norm(@accel) is per sample"
)]
// A component is not a whole-channel source.
#[case(
    "@accel.x | where @accel.x > 1 g",
    "a channel source is a whole channel"
)]
// An unknown source channel.
#[case("@nope | where @nope > 1", "no such channel @nope")]
fn a_channel_source_rejects(#[case] src: &str, #[case] message: &str) {
    let mut schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    schema.insert(
        "gyro",
        ChannelInfo {
            unit: Some(Unit::DEG.into()),
            period_deg: None,
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            conflicts: Vec::new(),
        },
    );
    let err = check(&parse(src).unwrap(), &schema).unwrap_err();
    assert_eq!(err.message, message);
}

#[test]
fn a_channel_source_matches_per_sample() {
    // `@accel | where ...` judges each sample on its own. The match ranges
    // are sample indices, and the total is the sample count, not nav points.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let provider = TestProvider::new(2).indexed_time().with_vector_channel(
        "accel",
        vec![
            (0.0, vec![9.8, 0.0, 0.0]),  // norm 9.8, over 0.1 g
            (1.0, vec![0.1, 0.0, 0.0]),  // norm 0.1, under
            (2.0, vec![10.0, 0.0, 0.0]), // over
        ],
    );
    let output = test_util::run_channel("@accel | where norm(@accel) > 0.1 g", &schema, &provider);
    assert_eq!(output.matches[0].ranges, vec![0..1, 2..3]);
    // Three channel samples, not the provider's two nav points.
    assert_eq!(output.summary.total_points, 3);
}

#[test]
fn a_channel_source_window_reduces_its_samples() {
    // `@accel | window 3 | where max(norm(@accel)) > 0.1 g`: the window over
    // all three samples has peak norm 10, so every sample matches.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let provider = TestProvider::new(2).indexed_time().with_vector_channel(
        "accel",
        vec![
            (0.0, vec![0.1, 0.0, 0.0]),
            (1.0, vec![0.1, 0.0, 0.0]),
            (2.0, vec![10.0, 0.0, 0.0]),
        ],
    );
    let output = test_util::run_channel(
        "@accel | window 3 | where max(norm(@accel)) > 0.1 g",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..3]);
}

#[test]
fn a_channel_source_duration_window_groups_by_sample_time() {
    // `@accel | window 2 s`: at anchor 0 the samples in [0, 2) s are indices
    // 0 and 1 (the sample at 2.0 s starts a window that overruns the data).
    // Their std over the x column is 0, so a calm channel matches [0, 2).
    let schema = test_util::vector_schema("accel", Some("g"), &["x"]);
    let provider = TestProvider::new(2).indexed_time().with_vector_channel(
        "accel",
        vec![(0.0, vec![9.8]), (1.0, vec![9.8]), (2.0, vec![9.8])],
    );
    let output = test_util::run_channel(
        "@accel | window 2 s | where std(@accel.x) < 0.02 g",
        &schema,
        &provider,
    );
    assert_eq!(output.matches[0].ranges, vec![0..2]);
}
