#include "../geotrace.h"
#include <criterion/criterion.h>

Test(null_guards, builder_null) {
    cr_assert_eq(gtd_builder_set_title(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_device(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_notes(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_identity(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_travel_mode(NULL, GTD_TRAVEL_MODE_CAR), GTD_ERR_NULL_ARGUMENT);

    GtdTimestamp timestamp = gtd_ts_from_seconds(0);
    cr_assert_eq(gtd_builder_add_nav_fix(NULL, timestamp, timestamp, 0.0, 0.0, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_ERR_NULL_ARGUMENT);

    cr_assert_eq(gtd_builder_add_satellite_report(NULL, timestamp, timestamp, NULL, 0),
                 GTD_ERR_NULL_ARGUMENT);

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

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(NULL, 0, &point), GTD_ERR_NULL_ARGUMENT);

    GtdSatInfo satellite;
    cr_assert_eq(gtd_nav_file_get_satellite(NULL, 0, 0, &satellite), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_event_marker(NULL, 0, &marker), GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, travel_mode_from_name_null) {
    GtdTravelMode mode;
    cr_assert_eq(gtd_travel_mode_from_name(NULL, &mode), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_travel_mode_from_name("car", NULL), GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, open_null_path) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(NULL, &file), GTD_ERR_NULL_ARGUMENT);
    cr_assert_null(file);
}

Test(null_guards, from_bytes_null_data) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(NULL, 10, &file), GTD_ERR_NULL_ARGUMENT);
    cr_assert_null(file);
}

Test(null_guards, from_bytes_empty_slice) {
    GtdNavFile *file = NULL;
    /* zero-length slice with NULL data pointer must not crash */
    GtdStatus status = gtd_nav_file_from_bytes(NULL, 0, &file);
    /* it won't succeed (not a valid gtd file), but must not segfault */
    (void)status;
    if (file) {
        gtd_nav_file_destroy(file);
    }
}

Test(null_guards, out_of_range_index) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp timestamp = gtd_ts_from_seconds(1700000000ULL);
    gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 0.0, 0.0, GTD_NONE_F64, GTD_NONE_F64,
                            GTD_NONE_F64);

    GtdNavFile *file = NULL;
    gtd_builder_finish(builder, &file);

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 999, &point), GTD_ERR_NULL_ARGUMENT);

    GtdSatInfo satellite;
    // index 0 has no satellite report, so it should return ErrNullArgument
    cr_assert_eq(gtd_nav_file_get_satellite(file, 0, 0, &satellite), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_event_marker(file, 999, &marker), GTD_ERR_NULL_ARGUMENT);

    gtd_nav_file_destroy(file);
}
