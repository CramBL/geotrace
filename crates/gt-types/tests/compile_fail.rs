// Regenerate the `.stderr` files with `TRYBUILD=overwrite cargo test -p gt-types
// --test compile_fail` when the compiler words a diagnostic differently. One
// `TestCases` runs every fixture: two instances would each write to the same
// generated project and collide.
#[test]
fn the_compile_fail_fixtures_produce_their_expected_errors() {
    let fixtures = trybuild::TestCases::new();

    fixtures.compile_fail("tests/compile_fail/generation_type_mismatch.rs");
    fixtures.compile_fail("tests/compile_fail/inner_type_is_not_versionable.rs");
}
