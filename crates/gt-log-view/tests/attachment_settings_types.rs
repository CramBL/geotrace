use trybuild::TestCases;

#[test]
fn attachment_settings_writes_require_state_preparation() {
    let tests = TestCases::new();
    tests.compile_fail("tests/compile_fail/attachment_settings_fields_are_private.rs");
}
