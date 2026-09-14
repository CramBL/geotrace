#include "../geotrace.h"
#include <criterion/criterion.h>
#include <stddef.h>
#include <stdint.h>

Test(null_guards, builder_null) {
    cr_assert_eq(gtd_builder_set_title(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_device(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_notes(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_identity(NULL, "x"), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_travel_mode(NULL, GTD_TRAVEL_MODE_CAR), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_lenient(NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_set_satellite_window_us(NULL, 0), GTD_ERR_NULL_ARGUMENT);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(0, &timestamp), GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix(NULL, timestamp, timestamp, 0.0, 0.0, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_ERR_NULL_ARGUMENT);

    cr_assert_eq(gtd_builder_add_satellite_report(NULL, timestamp, timestamp, NULL, 0),
                 GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, finish_sets_out_to_null_when_the_builder_is_null) {
    static char stale_handle_target;
    GtdNavFile *out = (GtdNavFile *)&stale_handle_target;

    GtdStatus status = gtd_builder_finish(NULL, &out);
    cr_assert_eq(status, GTD_ERR_NULL_ARGUMENT);
    cr_assert_null(out);
    cr_assert_str_eq(gtd_last_error(), "null pointer argument (builder)");
}

/* `ctest` fails this test on the LeakSanitizer report when the call does not free the builder. */
Test(null_guards, finish_consumes_the_builder_when_out_is_null) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdStatus status = gtd_builder_finish(builder, NULL);
    cr_assert_eq(status, GTD_ERR_NULL_ARGUMENT);
    cr_assert_str_eq(gtd_last_error(), "null pointer argument (out)");
}

Test(null_guards, open_sets_out_to_null_when_the_path_is_null) {
    static char stale_handle_target;
    GtdNavFile *out = (GtdNavFile *)&stale_handle_target;

    GtdStatus status = gtd_nav_file_open(NULL, &out);
    cr_assert_eq(status, GTD_ERR_NULL_ARGUMENT);
    cr_assert_null(out);
    cr_assert_str_eq(gtd_last_error(), "null string argument");
}

Test(null_guards, open_sets_out_to_null_when_the_path_is_not_utf8) {
    static char stale_handle_target;
    GtdNavFile *out = (GtdNavFile *)&stale_handle_target;

    GtdStatus status = gtd_nav_file_open("\xff.gtd", &out);
    cr_assert_eq(status, GTD_ERR_UTF8);
    cr_assert_null(out);
    cr_assert_str_eq(gtd_last_error(), "string argument is not valid UTF-8");
}

Test(null_guards, nav_file_null) {
    cr_assert_eq(gtd_nav_file_nav_point_count(NULL), 0);
    cr_assert_eq(gtd_nav_file_event_marker_count(NULL), 0);
    cr_assert_eq(gtd_nav_file_marker_count(NULL), 0);
    cr_assert_eq(gtd_nav_file_event_marker_style_count(NULL), 0);
    cr_assert_eq(gtd_nav_file_satellite_warning_count(NULL), 0);
    cr_assert_null(gtd_nav_file_title(NULL));
    cr_assert_null(gtd_nav_file_device(NULL));
    cr_assert_null(gtd_nav_file_notes(NULL));
    cr_assert_null(gtd_nav_file_identity(NULL));
    cr_assert_null(gtd_nav_file_travel_mode(NULL));

    size_t length = SIZE_MAX;
    cr_assert_null(gtd_nav_file_title_with_length(NULL, &length));
    cr_assert_eq(length, 0);
    length = SIZE_MAX;
    cr_assert_null(gtd_nav_file_device_with_length(NULL, &length));
    cr_assert_eq(length, 0);
    length = SIZE_MAX;
    cr_assert_null(gtd_nav_file_notes_with_length(NULL, &length));
    cr_assert_eq(length, 0);
    length = SIZE_MAX;
    cr_assert_null(gtd_nav_file_identity_with_length(NULL, &length));
    cr_assert_eq(length, 0);
    length = SIZE_MAX;
    cr_assert_null(gtd_nav_file_travel_mode_with_length(NULL, &length));
    cr_assert_eq(length, 0);

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(NULL, 0, &point), GTD_ERR_NULL_ARGUMENT);

    GtdSatInfo satellite;
    cr_assert_eq(gtd_nav_file_get_satellite(NULL, 0, 0, &satellite), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerInfo event_marker;
    cr_assert_eq(gtd_nav_file_get_event_marker(NULL, 0, &event_marker), GTD_ERR_NULL_ARGUMENT);

    GtdMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_marker(NULL, 0, &marker), GTD_ERR_NULL_ARGUMENT);

    GtdEventMarkerStyleInfo style;
    cr_assert_eq(gtd_nav_file_get_event_marker_style(NULL, 0, &style), GTD_ERR_NULL_ARGUMENT);

    GtdSatelliteWarningInfo satellite_warning;
    cr_assert_eq(gtd_nav_file_get_satellite_warning(NULL, 0, &satellite_warning),
                 GTD_ERR_NULL_ARGUMENT);

    GtdChannelInfo channel;
    cr_assert_eq(gtd_nav_file_get_channel(NULL, 0, &channel), GTD_ERR_NULL_ARGUMENT);

    char label[16];
    size_t required = 0;
    cr_assert_eq(gtd_nav_file_get_channel_unit(NULL, 0, label, sizeof label, &required, NULL),
                 GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, travel_mode_from_name_null) {
    GtdTravelMode mode;
    cr_assert_eq(gtd_travel_mode_from_name(NULL, &mode), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_travel_mode_from_name("car", NULL), GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, constellation_from_name_null) {
    GtdConstellation constellation;
    cr_assert_eq(gtd_constellation_from_name(NULL, &constellation), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_constellation_from_name("gps", NULL), GTD_ERR_NULL_ARGUMENT);
}

Test(null_guards, marker_icon_from_name_null) {
    GtdMarkerIcon icon;
    cr_assert_eq(gtd_marker_icon_from_name(NULL, &icon), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_marker_icon_from_name("pin", NULL), GTD_ERR_NULL_ARGUMENT);
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
