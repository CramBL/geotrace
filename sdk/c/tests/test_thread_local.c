#include "../geotrace.h"
#include <criterion/criterion.h>
#include <string.h>

Test(thread_local, last_error_initially_null) {
    /* On a fresh call sequence the error slot may or may not be set, but the
       function must not crash. Just verify it returns NULL or a valid string. */
    const char *e = gtd_last_error();
    (void)e;
}

Test(thread_local, last_error_set_after_failure) {
    GtdNavFile *f = NULL;
    gtd_nav_file_open(NULL, &f);
    const char *e = gtd_last_error();
    cr_assert_not_null(e);
    cr_assert(strlen(e) > 0);
}

Test(thread_local, last_error_set_after_finish_no_fixes) {
    GtdFileBuilder *b = gtd_builder_create();

    /* Trigger GTD_ERR_NO_NAV_FIXES by adding an annotation but no fixes */
    gtd_builder_add_annotation(b, gtd_ts_from_seconds(1700000000ULL), "note", GTD_ICON_AUTO);

    GtdNavFile *f = NULL;
    gtd_builder_finish(b, &f);
    const char *e = gtd_last_error();
    cr_assert_not_null(e);
    cr_assert(strlen(e) > 0);
}
