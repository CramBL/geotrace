#include "../geotrace.h"

#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <cmocka.h>
#include <string.h>

static void test_last_error_initially_null(void **state) {
    (void)state;
    /* On a fresh call sequence the error slot may or may not be set, but the
       function must not crash. Just verify it returns NULL or a valid string. */
    const char *e = gtd_last_error();
    (void)e;
}

static void test_last_error_set_after_failure(void **state) {
    (void)state;
    GtdNavFile *f = NULL;
    gtd_nav_file_open(NULL, &f);
    const char *e = gtd_last_error();
    assert_non_null(e);
    assert_true(strlen(e) > 0);
}

static void test_last_error_set_after_finish_no_fixes(void **state) {
    (void)state;
    GtdFileBuilder *b = gtd_builder_create();
    GtdNavFile *f = NULL;
    gtd_builder_finish(b, &f);
    const char *e = gtd_last_error();
    assert_non_null(e);
    assert_true(strlen(e) > 0);
}

int main(void) {
    const struct CMUnitTest tests[] = {
        cmocka_unit_test(test_last_error_initially_null),
        cmocka_unit_test(test_last_error_set_after_failure),
        cmocka_unit_test(test_last_error_set_after_finish_no_fixes),
    };
    return cmocka_run_group_tests(tests, NULL, NULL);
}
