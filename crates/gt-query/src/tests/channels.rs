use super::*;

#[rstest]
#[case("points | where @accel > 0", "accel", None)]
#[case("points | where @accel.x > 0", "accel", Some("x"))]
fn a_channel_reference_parses_and_formats(
    #[case] src: &str,
    #[case] name: &str,
    #[case] component: Option<&str>,
) {
    use crate::ast::Expr;
    let query = parse(src).expect(src);
    // The predicate is `<channel> > 0`, so the channel is the comparison's LHS.
    let Some(Expr::Binary { lhs, .. }) = query.predicates.first() else {
        panic!("expected a comparison in {src}");
    };
    let Expr::Channel(c) = lhs.as_ref() else {
        panic!("expected a channel reference as the lhs in {src}");
    };
    assert_eq!(c.name, name);
    assert_eq!(c.component.as_deref(), component);
    // The canonical form round-trips through parse → format unchanged.
    let expected = match component {
        Some(comp) => format!("@{name}.{comp}"),
        None => format!("@{name}"),
    };
    assert!(query.to_string().contains(&expected));
}

#[test]
fn a_channel_absent_from_the_schema_is_no_such_channel() {
    // An empty schema has no channels.
    let err = check(
        &parse("points | window 10 | where max(@accel) > 0.1 g").unwrap(),
        &ChannelSchema::new(),
    )
    .unwrap_err();
    assert_eq!(err.message, "no such channel @accel");
}

#[test]
fn a_scalar_channel_resolves_to_its_unit_dimension() {
    // @accel (unit g) is an acceleration, so it compares to an acceleration
    // literal and rejects a speed.
    let schema = test_util::schema_with("accel", Some("g"), None);
    let ok = "points | window 10 | where max(@accel) > 0.1 g";
    check(&parse(ok).unwrap(), &schema).expect("checks with the schema");

    let bad = "points | window 10 | where max(@accel) > 30 km/h";
    let err = check(&parse(bad).unwrap(), &schema).unwrap_err();
    assert_eq!(
        err.message,
        "expected a acceleration unit (m/s2, g, km/h/s), found km/h"
    );
}

#[test]
fn a_channel_with_a_period_is_circular_and_accepts_spread() {
    // @heading (deg, period 360) is a wrapping angle, so spread accepts it.
    let schema = test_util::schema_with("heading", Some("deg"), Some(360.0));
    let ok = "points | window 10 | where spread(@heading) < 10 deg";
    check(&parse(ok).unwrap(), &schema).expect("checks with the schema");
}

#[rstest]
#[case::a_period_on_a_length("m", Some(360.0), "points | window 10 | where avg(@sensor) > 1 m")]
#[case::a_period_of_zero("deg", Some(0.0), "points | window 10 | where avg(@sensor) > 1 deg")]
fn a_channel_that_does_not_wrap_accepts_avg(
    #[case] unit: &str,
    #[case] period_deg: Option<f64>,
    #[case] src: &str,
) {
    let schema = test_util::schema_with("sensor", Some(unit), period_deg);
    check(&parse(src).unwrap(), &schema).expect(src);
}

#[rstest]
#[case("avg")]
#[case("min")]
#[case("max")]
fn a_wrapping_channel_rejects_ambiguous_aggregates(#[case] func: &str) {
    // avg/min/max collapse a wrapping angle ambiguously, the same rule
    // `heading` and `lon` follow.
    let schema = test_util::schema_with("heading", Some("deg"), Some(360.0));
    let src = format!("points | window 10 | where {func}(@heading) < 10 deg");
    let err = check(&parse(&src).unwrap(), &schema).unwrap_err();
    assert_eq!(
        err.message,
        format!("{func} on a wrapping angle is ambiguous")
    );
}

#[rstest]
// No unit and an unrecognised unit both resolve to a bare number: it
// compares to a plain number but not to a dimensioned literal.
#[case(None)]
#[case(Some("furlong"))]
fn a_channel_without_a_known_unit_is_a_bare_number(#[case] unit: Option<&str>) {
    let schema = test_util::schema_with("x", unit, None);
    let ok = "points | window 10 | where max(@x) > 5";
    check(&parse(ok).unwrap(), &schema).expect("a bare number compares to a number");

    let bad = "points | window 10 | where max(@x) > 5 g";
    check(&parse(bad).unwrap(), &schema).unwrap_err();
}

#[test]
fn a_bare_channel_must_be_aggregated() {
    // Like a nav-point metric, a channel has no per-point value. Used raw it
    // errors with a hint to wrap it in an aggregate.
    let schema = test_util::schema_with("accel", Some("g"), None);
    let err = check(
        &parse("points | window 10 | where @accel > 0.1 g").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "@accel is per sample");
    assert_eq!(
        err.help.as_deref(),
        Some("wrap it in an aggregate like max(@accel)")
    );
}

#[test]
fn an_aggregate_over_two_channels_is_rejected() {
    // An aggregate reduces one timeline. Two channels are on separate clocks
    // and cannot be combined per sample, so mixing them is a category error.
    let mut schema = test_util::schema_with("ax", Some("g"), None);
    schema.insert(
        "ay",
        ChannelInfo {
            unit: Some(Unit::G.into()),
            period_deg: None,
            components: vec![],
            conflicts: Vec::new(),
        },
    );
    let err = check(
        &parse("points | window 10 | where max(@ax + @ay) > 1.0 g").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "an aggregate reduces one channel at a time");
}

#[test]
fn an_aggregate_mixing_a_channel_and_a_metric_is_rejected() {
    // @accel (a channel) and accel (the derived nav metric) share a dimension
    // but not a clock, so an aggregate cannot combine them per element.
    let schema = test_util::schema_with("accel", Some("g"), None);
    let err = check(
        &parse("points | window 10 | where max(@accel + accel) > 1.0 g").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "cannot mix @accel with a per-point metric");
}

#[test]
fn a_vector_component_resolves_to_the_channel_dimension() {
    // @accel.x is one column of the g-unit vector, so it is an acceleration:
    // it compares to an acceleration literal and rejects a speed.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let ok = "points | window 10 | where max(@accel.x) > 0.1 g";
    check(&parse(ok).unwrap(), &schema).expect("a component checks like a scalar");

    let bad = "points | window 10 | where max(@accel.x) > 30 km/h";
    let err = check(&parse(bad).unwrap(), &schema).unwrap_err();
    assert!(err.message.contains("acceleration"), "{}", err.message);
}

#[test]
fn a_bare_vector_channel_needs_a_component() {
    // A whole vector has no scalar value. The error points at a component.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let err = check(
        &parse("points | window 10 | where max(@accel) > 0.1 g").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "@accel is a vector channel");
    assert_eq!(
        err.help.as_deref(),
        Some("reference a component like @accel.x")
    );
}

#[test]
fn an_unknown_component_is_rejected() {
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let err = check(
        &parse("points | window 10 | where max(@accel.w) > 0.1 g").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "@accel has no component w");
    assert_eq!(err.help.as_deref(), Some("its components are x, y, z"));
}

#[test]
fn a_component_on_a_scalar_channel_is_rejected() {
    let schema = test_util::schema_with("incline", Some("deg"), None);
    let err = check(
        &parse("points | window 10 | where max(@incline.x) > 1 deg").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "@incline is not a vector channel");
}

#[test]
fn a_bare_component_must_be_aggregated() {
    // Like a nav-point metric, a component has no per-point value. Used raw
    // it hints at wrapping it in an aggregate, keeping the `.x`.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let err = check(
        &parse("points | window 10 | where @accel.x > 0.1 g").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "@accel.x is per sample");
    assert_eq!(
        err.help.as_deref(),
        Some("wrap it in an aggregate like max(@accel.x)")
    );
}

#[test]
fn a_windowless_points_channel_points_at_the_working_forms() {
    // On the points source with no window, wrapping the channel in an
    // aggregate alone dead-ends on "needs a window", so the hint points at
    // both forms that work: an aggregate over a window, or the channel as
    // its own source. (Regression: the old hint sent the user to
    // max(@accel.x), which then failed with "max needs a window".)
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let err = check(&parse("points | where @accel.x > 0.2 mg").unwrap(), &schema).unwrap_err();
    assert_eq!(err.message, "@accel.x is per sample");
    assert_eq!(
        err.help.as_deref(),
        Some("aggregate it over a window like max(@accel.x), or query @accel as the source")
    );
}

#[test]
fn an_aggregate_without_a_window_hints_at_adding_one() {
    // Following the hint above to an aggregate still needs a window: the error
    // says how.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let err = check(
        &parse("points | where max(@accel.x) > 0.2 mg").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "max needs a window");
    assert_eq!(
        err.help.as_deref(),
        Some("add a window before the where, e.g. window 10")
    );
}

#[test]
fn a_windowless_points_norm_points_at_the_working_forms() {
    // norm takes the same unwindowed-points branch, labelled as it reads.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let err = check(
        &parse("points | where norm(@accel) > 0.2 mg").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "norm(@accel) is per sample");
    assert_eq!(
        err.help.as_deref(),
        Some("aggregate it over a window like max(norm(@accel)), or query @accel as the source")
    );
}

#[test]
fn components_of_one_channel_combine_per_sample() {
    // Components of one vector share a clock, so per-sample math across them
    // is a single timeline: sqrt(x² + y²) type-checks as an acceleration.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let src = "points | window 10 | where max(sqrt(@accel.x² + @accel.y²)) > 0.1 g";
    check(&parse(src).unwrap(), &schema).expect("shared-clock components combine");
}

#[test]
fn two_different_channels_cannot_combine() {
    // Distinct channels are on independent clocks, even at the same dimension
    // (both acceleration here, so the timeline rule is the one that rejects it,
    // not a unit mismatch).
    let mut schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    schema.insert(
        "accel2",
        ChannelInfo {
            unit: Some(Unit::G.into()),
            period_deg: None,
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            conflicts: Vec::new(),
        },
    );
    let err = check(
        &parse("points | window 10 | where max(@accel.x + @accel2.x) > 0.1 g").unwrap(),
        &schema,
    )
    .unwrap_err();
    assert_eq!(err.message, "an aggregate reduces one channel at a time");
}

#[test]
fn norm_is_the_magnitude_of_a_vector_channel() {
    // norm(@accel) is an acceleration, so it compares to a g literal.
    let schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    let ok = "points | window 10 | where max(norm(@accel)) > 1 g";
    check(&parse(ok).unwrap(), &schema).expect("norm of a vector is its dimension");
}

/// accel and accel2 (g-unit vectors) plus the scalar incline, for exercising
/// norm's rejections and cross-timeline mixing.
fn norm_schema() -> ChannelSchema {
    let mut schema = test_util::vector_schema("accel", Some("g"), &["x", "y", "z"]);
    schema.insert(
        "accel2",
        ChannelInfo {
            unit: Some(Unit::G.into()),
            period_deg: None,
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            conflicts: Vec::new(),
        },
    );
    schema.insert(
        "incline",
        ChannelInfo {
            unit: Some(Unit::DEG.into()),
            period_deg: None,
            components: vec![],
            conflicts: Vec::new(),
        },
    );
    schema
}

#[rstest]
// A scalar channel has no vector to take the magnitude of.
#[case(
    "max(norm(@incline)) > 1 deg",
    "@incline is not a vector channel",
    Some("norm needs a vector like @accel")
)]
// A single component is not a whole vector.
#[case(
    "max(norm(@accel.x)) > 1 g",
    "norm takes a whole vector, not a component",
    Some("use norm(@accel)")
)]
// norm is per sample, so a bare use needs an aggregate.
#[case(
    "norm(@accel) > 1 g",
    "norm(@accel) is per sample",
    Some("wrap it in an aggregate like max(norm(@accel))")
)]
// norm's channel counts as a timeline: two are on separate clocks.
#[case(
    "max(norm(@accel) + norm(@accel2)) > 1 g",
    "an aggregate reduces one channel at a time",
    Some("split it into separate aggregates, one per channel")
)]
// And a channel cannot mix with a per-point metric (accel is acceleration,
// matching norm's dimension, so the timeline rule is the one that rejects it).
#[case(
    "max(norm(@accel) + accel) > 1 g",
    "cannot mix @accel with a per-point metric",
    Some("a channel and a nav-point metric are on separate clocks")
)]
fn norm_and_channel_mixing_rejections(
    #[case] predicate: &str,
    #[case] message: &str,
    #[case] help: Option<&str>,
) {
    let src = format!("points | window 10 | where {predicate}");
    let err = check(&parse(&src).unwrap(), &norm_schema()).unwrap_err();
    assert_eq!(err.message, message);
    assert_eq!(err.help.as_deref(), help);
}
