use trybuild::TestCases;

#[test]
fn initial_log_saves_require_context_preparation() {
    let tests = TestCases::new();
    tests.compile_fail("tests/compile_fail/log_save_request_fields_are_private.rs");
}
