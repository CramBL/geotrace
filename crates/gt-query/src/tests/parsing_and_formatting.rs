use super::*;

#[test]
fn deep_nesting_errors_instead_of_overflowing() {
    // The MAX_DEPTH guard must fire well before the stack gives out.
    let src = format!("points | where {}velocity > 0 km/h", "not ".repeat(70));
    let err = parse(&src).unwrap_err();
    assert_eq!(err.message, "expression is too deeply nested");
}

#[test]
fn caret_and_superscript_powers_agree() {
    // The caret form is a convenience for the canonical superscript. Both
    // parse to the same tree, so they print identically.
    let same = |caret: &str, superscript: &str| {
        assert_eq!(
            parse(caret).unwrap().to_string(),
            parse(superscript).unwrap().to_string()
        );
    };
    same(
        "points | where velocity^2 > velocity^2",
        "points | where velocity² > velocity²",
    );
    same(
        "points | where sats_fix^-1 < 0.5",
        "points | where sats_fix⁻¹ < 0.5",
    );
}

/// A power binds tighter than unary minus and than `*`/`/`, and its base can
/// be a parenthesized expression.
#[rstest]
#[case("points | where -accel² < 0", "(-(accel²))")]
#[case("points | where velocity² * eph > 0", "((velocity²) * eph)")]
#[case("points | where (velocity + eph)² > 0", "((velocity + eph)²)")]
fn power_binds_tighter_than_minus_and_mul(#[case] src: &str, #[case] fragment: &str) {
    let canonical = parse(src).expect(src).to_string();
    assert!(
        canonical.contains(fragment),
        "{canonical} should contain {fragment}"
    );
}

#[test]
fn uc1_parses_checks_and_formats() {
    let query = parse(UC1).unwrap();
    test_util::chk(&query).unwrap();
    insta::assert_debug_snapshot!("uc1_ast", query);
    insta::assert_snapshot!("uc1_canonical", query.to_string());
}
