#include "../geotrace.h"
#include <criterion/criterion.h>

Test(null_guards, builder_null) {
    cr_assert_eq(gtd_builder_set_title(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_device(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_notes(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_identity(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_travel_mode(NULL, GTD_TRAVEL_MODE_CAR), GTD_ERR_NULL_ARGUMENT);

    GtdTimestamp t = gtd_ts_from_seconds(0);
    cr_assert_eq(
        gtd_builder_add_nav_fix(NULL, t, t, 0.0, 0.0, GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
        GTD_ERR_NULL_ARGUMENT);

    cr_assert_eq(gtd_builder_add_satellite_report(NULL, t, t, NULL, 0), GTD_ERR_NULL_ARGUMENT);

    GtdNavFile *out = NULL;
    cr_assert_eq(gtd_builder_finish(NULL, &out), GTD_ERR_NULL_ARGUMENT);
    cr_assert_null(out);
}

Test(null_guards, nav_file_null) {
    cr_assert_eq(gtd_nav_file_nav_point_count(NULL), 0);
    cr_assert_eq(gtd_nav_file_event_marker_count(NULL), 0);
    cr_assert_null(gtd_nav_file_title(NULL));
    cr_assert_null(gtd_nav_file_device(NULL));
    cr_assert_null(gtd_nav_file_notes(NULL));
    cr_assert_null(gtd_nav_file_identity(NULL));
    cr_assert_null(gtd_nav_file_travel_mode(NULL));

    GtdNavPointInfo pi;
    cr_assert_eq(gtd_nav_file_get_nav_point(NULL, 0, &pi), GTD_ERR_NULL_ARGUMENT);

    GtdSatInfo si;
    cr_assert_eq(gtd_nav_file_get_satellite(NULL, 0, 0, &si), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerInfo em;
    cr_assert_eq(gtd_nav_file_get_event_marker(NULL, 0, &em), GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, travel_mode_from_name_null) {
    GtdTravelMode mode;
    cr_assert_eq(gtd_travel_mode_from_name(NULL, &mode), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_travel_mode_from_name("car", NULL), GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, open_null_path) {
    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_nav_file_open(NULL, &f), GTD_ERR_NULL_ARGUMENT);
    cr_assert_null(f);
}

Test(null_guards, from_bytes_null_data) {
    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(NULL, 10, &f), GTD_ERR_NULL_ARGUMENT);
    cr_assert_null(f);
}

Test(null_guards, from_bytes_empty_slice) {
    GtdNavFile *f = NULL;
    /* zero-length slice with NULL data pointer must not crash */
    GtdStatus s = gtd_nav_file_from_bytes(NULL, 0, &f);
    /* it won't succeed (not a valid gtd file), but must not segfault */
    (void)s;
    if (f)
        gtd_nav_file_destroy(f);
}

Test(null_guards, out_of_range_index) {
    GtdFileBuilder *b = gtd_builder_create();
    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 0.0, 0.0, GTD_NONE_F64, GTD_NONE_F64,
                            GTD_NONE_F64);

    GtdNavFile *f = NULL;
    gtd_builder_finish(b, &f);

    GtdNavPointInfo pi;
    cr_assert_eq(gtd_nav_file_get_nav_point(f, 999, &pi), GTD_ERR_NULL_ARGUMENT);

    GtdSatInfo si;
    // index 0 has no satellite report, so it should return ErrNullArgument
    cr_assert_eq(gtd_nav_file_get_satellite(f, 0, 0, &si), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerInfo em;
    cr_assert_eq(gtd_nav_file_get_event_marker(f, 999, &em), GTD_ERR_NULL_ARGUMENT);

    gtd_nav_file_destroy(f);
}
