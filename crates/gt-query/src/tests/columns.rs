use super::*;

#[test]
fn columns_default_to_time_plus_referenced_metrics() {
    let query = test_util::checked(
        "points | window 5 | where spread(heading) <= 10 deg and avg(velocity) > 30 km/h",
    );
    assert_eq!(column_labels(&query), ["time", "heading", "velocity"]);
}

#[test]
fn explicit_table_controls_columns_time_stays_first() {
    let query = test_util::checked(UC1);
    assert_eq!(
        column_labels(&query),
        ["time", "velocity", "heading", "accel"]
    );
    assert_eq!(query.mode(), DisplayMode::Draw);
}

/// Every column of a checked query, in table order.
fn column_labels(query: &CheckedQuery) -> Vec<String> {
    query.columns().iter().map(TableColumn::label).collect()
}

/// The checked aggregate column written as `label`.
fn aggregate_column<'a>(query: &'a CheckedQuery, label: &str) -> &'a AggregateColumn {
    query
        .columns()
        .iter()
        .find_map(|column| match column {
            TableColumn::Aggregate(aggregate) if aggregate.label() == label => Some(aggregate),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the query has no {label} column"))
}

#[test]
fn a_table_column_takes_a_channel_aggregate() {
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let src = "points | window 3 | where max(@accel.x) > 1 g | table time, max(@accel.x)";
    let query = check(&parse(src).unwrap(), &schema).expect(src);
    assert_eq!(column_labels(&query), ["time", "max(@accel.x)"]);
    // The column's values are accelerations in base units, and a table
    // converts them out through the quantity.
    assert_eq!(
        aggregate_column(&query, "max(@accel.x)").quantity(),
        Some(Quantity::Acceleration)
    );
}

#[test]
fn a_table_column_takes_a_metric_aggregate() {
    let query = test_util::checked("points | window 3 | table avg(velocity)");
    assert_eq!(column_labels(&query), ["time", "avg(velocity)"]);
    assert_eq!(
        aggregate_column(&query, "avg(velocity)").quantity(),
        Some(Quantity::Speed)
    );
    // The runner derives the aggregate's metric: the aggregate counts as a
    // reference to it.
    assert_eq!(query.referenced_metrics(), [QueryMetric::Velocity]);
}

/// An aggregate reduces one timeline: a channel's samples, or the match's nav
/// points. `norm` reduces the whole vector on the channel's own clock.
#[rstest]
#[case::component("max(@accel.x)", Some("accel"))]
#[case::whole_vector("max(norm(@accel))", Some("accel"))]
#[case::metric("avg(velocity)", None)]
fn an_aggregate_column_names_the_channel_it_reduces(
    #[case] call: &str,
    #[case] channel: Option<&str>,
) {
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let src = format!("points | window 3 | table {call}");
    let query = check(&parse(&src).unwrap(), &schema).expect(&src);
    assert_eq!(aggregate_column(&query, call).reduced_channel(), channel);
}

/// `var` squares its argument, and a squared speed has no quantity.
#[test]
fn an_aggregate_column_of_an_unnamed_dimension_has_no_quantity() {
    let query = test_util::checked("points | window 3 | table var(velocity)");
    assert_eq!(aggregate_column(&query, "var(velocity)").quantity(), None);
}

#[test]
fn a_repeated_table_column_is_listed_once() {
    let query = test_util::checked("points | window 3 | table avg(velocity), avg(velocity), time");
    assert_eq!(column_labels(&query), ["time", "avg(velocity)"]);
}

#[rstest]
// Windowed on the points source, an aggregate is the whole fix.
#[case(
    "points | window 3 | table @accel.x",
    "wrap it in an aggregate like max(@accel.x)"
)]
// The hint names both working forms: unwindowed, an aggregate alone still
// needs a window.
#[case(
    "points | table @accel.x",
    "aggregate it over a window like max(@accel.x), or query @accel as the source"
)]
fn a_bare_channel_table_column_is_per_sample(#[case] src: &str, #[case] help: &str) {
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let err = check(&parse(src).unwrap(), &schema).unwrap_err();
    assert_eq!(err.message, "@accel.x is per sample");
    assert_eq!(err.help.as_deref(), Some(help));
    // The underline sits on the column, not on the stage or the query.
    let at = src.find("@accel.x").expect("has the column");
    assert_eq!(err.span, Span::new(at, at + "@accel.x".len()));
}

#[test]
fn a_table_column_is_a_metric_or_an_aggregate() {
    let err = check(
        &parse("points | table abs(velocity)").unwrap(),
        &ChannelSchema::new(),
    )
    .unwrap_err();
    assert_eq!(err.message, "a table column is a metric or an aggregate");
    assert_eq!(err.help.as_deref(), Some("try velocity, or max(@accel.x)"));
}

#[test]
fn a_channel_column_reduces_the_samples_of_its_match() {
    // The column reduces the accel samples in [0, 2], the closed time extent
    // of the match's points 0..3. Their peak is 10.8 m/s2.
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
    let src = "points | window 3 | where max(@accel) > 1.0 g | table max(@accel)";
    let query = check(&parse(src).unwrap(), &schema).expect(src);
    let column = aggregate_column(&query, "max(@accel)");
    assert_eq!(column.value_over_match(&provider, 0..3), Some(10.8));
}

/// The column reduces the sample at 5 s, between the match's points. The third
/// point steps the clock back to 1 s, and the points run from 0 s to 10 s.
#[test]
fn a_channel_column_spans_the_time_extent_of_a_match_across_a_backward_time_step() {
    let schema = test_util::schema_with("sensor", None, None);
    let provider = TestProvider::new(3)
        .with(QueryMetric::Time, vec![Some(0.0), Some(10.0), Some(1.0)])
        .with_channel("sensor", vec![(5.0, 10.0)]);
    let src = "points | window 3 | where max(@sensor) > 5 | table max(@sensor)";
    let query = check(&parse(src).unwrap(), &schema).expect(src);
    let column = aggregate_column(&query, "max(@sensor)");
    assert_eq!(column.value_over_match(&provider, 0..3), Some(10.0));
}

#[test]
fn a_metric_column_reduces_the_points_of_its_match() {
    let provider = TestProvider::new(4).indexed_time().with(
        QueryMetric::Velocity,
        vec![Some(10.0), Some(20.0), Some(30.0), Some(400.0)],
    );
    let query = test_util::checked("points | window 2 | table avg(velocity)");
    let column = aggregate_column(&query, "avg(velocity)");
    // The match's own three points average to 20 m/s: the fourth point is
    // outside it.
    assert_eq!(column.value_over_match(&provider, 0..3), Some(20.0));
}

#[test]
fn a_channel_source_column_reduces_the_samples_its_match_indexes() {
    // On a channel source a match is a range of samples. The column reduces
    // the rows of that range: x peaks at 3 over the first two samples.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y"]);
    let provider = TestProvider::new(0).with_vector_channel(
        "accel",
        vec![
            (0.0, vec![1.0, 2.0]),
            (1.0, vec![3.0, 4.0]),
            (2.0, vec![9.0, 0.0]),
        ],
    );
    let src = "@accel | window 2 | where max(@accel.x) > 0 g | table max(@accel.x)";
    let query = check(&parse(src).unwrap(), &schema).expect(src);
    let column = aggregate_column(&query, "max(@accel.x)");
    assert_eq!(column.value_over_match(&provider, 0..2), Some(3.0));
}

#[test]
fn a_column_of_an_empty_match_has_no_value() {
    let provider = TestProvider::new(2)
        .indexed_time()
        .with(QueryMetric::Velocity, vec![Some(10.0), Some(20.0)]);
    let query = test_util::checked("points | window 2 | table avg(velocity)");
    let column = aggregate_column(&query, "avg(velocity)");
    assert_eq!(column.value_over_match(&provider, 1..1), None);
}

#[test]
fn a_missing_value_leaves_the_column_without_one() {
    // One point of the match has no velocity. A missing value poisons the
    // column as it poisons a window in a run.
    let provider = TestProvider::new(3)
        .indexed_time()
        .with(QueryMetric::Velocity, vec![Some(10.0), None, Some(30.0)]);
    let query = test_util::checked("points | window 2 | table avg(velocity)");
    let column = aggregate_column(&query, "avg(velocity)");
    assert_eq!(column.value_over_match(&provider, 0..3), None);
    assert_eq!(column.value_over_match(&provider, 2..3), Some(30.0));
}
