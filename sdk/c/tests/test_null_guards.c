#include "../geotrace.h"

#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <cmocka.h>

static void test_builder_null(void **state) {
    (void)state;
    assert_int_equal(gtd_builder_set_title(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    assert_int_equal(gtd_builder_set_device(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    assert_int_equal(gtd_builder_set_notes(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    assert_int_equal(gtd_builder_set_identity(NULL, "x"), GTD_ERR_NULL_ARGUMENT);

    GtdTimestamp t = gtd_ts_from_seconds(0);
    assert_int_equal(
        gtd_builder_add_nav_fix(NULL, t, t, 0.0, 0.0, GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
        GTD_ERR_NULL_ARGUMENT);

    assert_int_equal(gtd_builder_add_satellite_report(NULL, t, t, NULL, 0), GTD_ERR_NULL_ARGUMENT);

    GtdNavFile *out = NULL;
    assert_int_equal(gtd_builder_finish(NULL, &out), GTD_ERR_NULL_ARGUMENT);
    assert_null(out);
}

static void test_nav_file_null(void **state) {
    (void)state;
    assert_int_equal(gtd_nav_file_nav_point_count(NULL), 0);
    assert_int_equal(gtd_nav_file_event_marker_count(NULL), 0);
    assert_null(gtd_nav_file_title(NULL));
    assert_null(gtd_nav_file_device(NULL));
    assert_null(gtd_nav_file_notes(NULL));
    assert_null(gtd_nav_file_identity(NULL));

    GtdNavPointInfo pi;
    assert_int_equal(gtd_nav_file_get_nav_point(NULL, 0, &pi), GTD_ERR_NULL_ARGUMENT);

    GtdSatInfo si;
    assert_int_equal(gtd_nav_file_get_satellite(NULL, 0, 0, &si), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerInfo em;
    assert_int_equal(gtd_nav_file_get_event_marker(NULL, 0, &em), GTD_ERR_NULL_ARGUMENT);
}

static void test_open_null_path(void **state) {
    (void)state;
    GtdNavFile *f = NULL;
    assert_int_equal(gtd_nav_file_open(NULL, &f), GTD_ERR_NULL_ARGUMENT);
    assert_null(f);
}

static void test_from_bytes_null_data(void **state) {
    (void)state;
    GtdNavFile *f = NULL;
    assert_int_equal(gtd_nav_file_from_bytes(NULL, 10, &f), GTD_ERR_NULL_ARGUMENT);
    assert_null(f);
}

static void test_from_bytes_empty_slice(void **state) {
    (void)state;
    GtdNavFile *f = NULL;
    /* zero-length slice with NULL data pointer must not crash */
    GtdStatus s = gtd_nav_file_from_bytes(NULL, 0, &f);
    /* it won't succeed (not a valid gtd file), but must not segfault */
    (void)s;
    if (f)
        gtd_nav_file_destroy(f);
}

static void test_out_of_range_index(void **state) {
    (void)state;

    GtdFileBuilder *b = gtd_builder_create();
    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 0.0, 0.0, GTD_NONE_F64, GTD_NONE_F64,
                            GTD_NONE_F64);

    GtdNavFile *f = NULL;
    gtd_builder_finish(b, &f);

    GtdNavPointInfo pi;
    assert_int_equal(gtd_nav_file_get_nav_point(f, 999, &pi), GTD_ERR_NULL_ARGUMENT);

    GtdSatInfo si;
    assert_int_equal(gtd_nav_file_get_satellite(f, 0, 0, &si), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerInfo em;
    assert_int_equal(gtd_nav_file_get_event_marker(f, 0, &em), GTD_ERR_NULL_ARGUMENT);

    gtd_nav_file_destroy(f);
}

int main(void) {
    const struct CMUnitTest tests[] = {
        cmocka_unit_test(test_builder_null),           cmocka_unit_test(test_nav_file_null),
        cmocka_unit_test(test_open_null_path),         cmocka_unit_test(test_from_bytes_null_data),
        cmocka_unit_test(test_from_bytes_empty_slice), cmocka_unit_test(test_out_of_range_index),
    };
    return cmocka_run_group_tests(tests, NULL, NULL);
}
