use super::*;

/// `jamming` is a ratio, so it takes what the `util_*` metrics take: a `%`
/// literal or another ratio metric. A bare number or a literal of any
/// other dimension is rejected.
#[rstest]
#[case::percent_literal("points | where jamming > 10 %", true)]
#[case::a_range("points | where 2 % < jamming and jamming < 10 %", true)]
// The unit is required, exactly as it is for the `util_*` metrics.
#[case::bare_number("points | where jamming > 0.1", false)]
#[case::zero("points | where jamming > 0", false)]
#[case::against_another_ratio("points | with mask 15 deg | where jamming > util_all", true)]
#[case::compared_to_a_length("points | where jamming > 10 m", false)]
#[case::compared_to_a_speed("points | where jamming > 10 km/h", false)]
#[case::compared_to_a_duration("points | where jamming > 10 s", false)]
fn jamming_accepts_ratios_and_rejects_other_dimensions(#[case] src: &str, #[case] accepted: bool) {
    let checked = parse(src).and_then(|query| test_util::chk(&query).map(|_| ()));
    assert_eq!(checked.is_ok(), accepted, "for {src}: {checked:?}");
}

/// The metric's values come from the archive, not from a derivation.
#[test]
fn jamming_needs_no_parameters() {
    let checked =
        parse("points | where jamming > 10 %").and_then(|query| test_util::chk(&query).map(|_| ()));
    assert_eq!(checked, Ok(()), "no `with` stage is required");
}

/// A ratio compares against `%`, never a bare number - a bare number is the
/// neutral kind and is never accepted as a percentage.
#[rstest]
#[case("points | with mask 15 deg | where util_all < 50 %", true)]
#[case("points | with mask 15 deg | where util_all < 50", false)]
fn a_ratio_metric_needs_a_percent_literal(#[case] src: &str, #[case] accepted: bool) {
    assert_eq!(
        test_util::chk(&parse(src).unwrap()).is_ok(),
        accepted,
        "for {src}"
    );
}
