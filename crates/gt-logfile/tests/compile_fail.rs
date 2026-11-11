#[test]
fn association_requires_a_time_from_eligible_chronology() {
    let fixtures = trybuild::TestCases::new();
    fixtures.compile_fail("tests/compile_fail/raw_timestamp_cannot_associate.rs");
    fixtures.compile_fail("tests/compile_fail/eligible_log_time_constructor_is_private.rs");
}
