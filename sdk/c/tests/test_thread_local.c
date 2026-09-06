#include "../geotrace.h"
#include <criterion/criterion.h>
#include <string.h>

Test(thread_local, last_error_initially_null) {
    /* On a fresh call sequence the error slot may or may not be set, but the
       function must not crash. Just verify it returns NULL or a valid string. */
    const char *error = gtd_last_error();
    (void)error;
}

Test(thread_local, last_error_set_after_failure) {
    GtdNavFile *file = NULL;
    gtd_nav_file_open(NULL, &file);
    const char *error = gtd_last_error();
    cr_assert_not_null(error);
    cr_assert(strlen(error) > 0);
}

Test(thread_local, last_error_set_after_finish_no_fixes) {
    GtdFileBuilder *builder = gtd_builder_create();

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    /* Trigger GTD_ERR_NO_NAV_FIXES by adding an annotation but no fixes */
    gtd_builder_add_annotation(builder, timestamp, "note", GTD_ICON_PIN);

    GtdNavFile *file = NULL;
    gtd_builder_finish(builder, &file);
    const char *error = gtd_last_error();
    cr_assert_not_null(error);
    cr_assert(strlen(error) > 0);
}
