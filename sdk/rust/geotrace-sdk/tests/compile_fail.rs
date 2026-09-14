// `TRYBUILD=overwrite cargo test -p geotrace-sdk --test compile_fail` rewrites the `.stderr` files
// when the compiler words a diagnostic differently.
#[test]
fn the_compile_fail_fixtures_produce_their_expected_errors() {
    trybuild::TestCases::new().compile_fail("tests/compile_fail/*.rs");
}
