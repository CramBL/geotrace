use gt_ui_types::MapEligibility;

#[test]
fn map_eligibility_has_no_default_escape_hatch() {
    // If `MapEligibility` implements `Default`, both implementations apply and
    // the marker type becomes ambiguous, turning this into a compile error.
    trait AmbiguousIfDefault<Marker> {
        fn marker() {}
    }
    impl<T: ?Sized> AmbiguousIfDefault<()> for T {}
    impl<T: Default> AmbiguousIfDefault<u8> for T {}

    let _ = <MapEligibility<'static> as AmbiguousIfDefault<_>>::marker;
}

// Regenerate the `.stderr` files with `TRYBUILD=overwrite cargo test -p gt-ui-types
// --test compile_fail` when the pinned compiler changes its diagnostics.
#[test]
fn map_policy_proofs_cannot_be_forged_or_partially_constructed() {
    let fixtures = trybuild::TestCases::new();

    fixtures.compile_fail("tests/compile_fail/map_eligibility_fields_are_private.rs");
    fixtures.compile_fail("tests/compile_fail/map_eligibility_requires_all_sources.rs");
    fixtures.compile_fail("tests/compile_fail/present_element_ref_fields_are_private.rs");
    fixtures.compile_fail("tests/compile_fail/raw_ref_cannot_pin.rs");
}
