#include "../geotrace.h"
#include <criterion/criterion.h>
#include <stdlib.h>

/* One nav fix without a satellite report, and one two-component channel. */
static GtdNavFile *build_file(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5, -0.1,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    double values[2] = {1.0, 2.0};
    const char *components[2] = {"x", "y"};
    GtdChannel channel = {0};
    channel.name = "accel";
    channel.unit = "m/s2";
    channel.period_deg = GTD_NONE_F64;
    channel.components = components;
    channel.n_components = 2;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = values;
    channel.n_values = 2;
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    cr_assert_not_null(file);
    return file;
}

static GtdFileBuilder *builder_with_a_nav_fix(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5, -0.1,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    return builder;
}

Test(status_codes, index_past_the_end_is_out_of_range) {
    GtdNavFile *file = build_file();

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 99, &point), GTD_ERR_OUT_OF_RANGE);

    GtdSatInfo satellite;
    cr_assert_eq(gtd_nav_file_get_satellite(file, 99, 0, &satellite), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite(file, 0, 0, &satellite), GTD_ERR_OUT_OF_RANGE);

    GtdEventMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_event_marker(file, 99, &marker), GTD_ERR_OUT_OF_RANGE);

    GtdChannelInfo channel;
    cr_assert_eq(gtd_nav_file_get_channel(file, 99, &channel), GTD_ERR_OUT_OF_RANGE);

    char label[16];
    cr_assert_eq(gtd_nav_file_get_channel_component(file, 0, 99, label, sizeof label),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_channel_component(file, 99, 0, label, sizeof label),
                 GTD_ERR_OUT_OF_RANGE);

    size_t required = 0;
    cr_assert_eq(gtd_nav_file_get_channel_unit(file, 99, label, sizeof label, &required, NULL),
                 GTD_ERR_OUT_OF_RANGE);

    gtd_nav_file_destroy(file);
}

Test(status_codes, index_within_the_file_is_ok) {
    GtdNavFile *file = build_file();

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &point), GTD_OK);

    GtdChannelInfo channel;
    cr_assert_eq(gtd_nav_file_get_channel(file, 0, &channel), GTD_OK);

    char label[16];
    cr_assert_eq(gtd_nav_file_get_channel_component(file, 0, 1, label, sizeof label), GTD_OK);
    cr_assert_str_eq(label, "y");

    size_t required = 0;
    cr_assert_eq(gtd_nav_file_get_channel_unit(file, 0, label, sizeof label, &required, NULL),
                 GTD_OK);
    cr_assert_str_eq(label, "m/s2");

    gtd_nav_file_destroy(file);
}

Test(status_codes, zero_component_buffer_capacity_is_out_of_range) {
    GtdNavFile *file = build_file();

    char label[16];
    cr_assert_eq(gtd_nav_file_get_channel_component(file, 0, 0, label, 0), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_channel_component(file, 0, 0, NULL, sizeof label),
                 GTD_ERR_NULL_ARGUMENT);

    gtd_nav_file_destroy(file);
}

Test(status_codes, short_unit_parse_buffer_is_out_of_range) {
    size_t required = 0;
    cr_assert_eq(gtd_channel_unit_parse("kph", GTD_CHANNEL_UNIT_RECOGNIZED, NULL, 0, &required),
                 GTD_OK);
    cr_assert_eq(required, 5); /* "km/h" plus the terminating NUL */

    char *exact = malloc(required);
    cr_assert_not_null(exact);
    cr_assert_eq(
        gtd_channel_unit_parse("kph", GTD_CHANNEL_UNIT_RECOGNIZED, exact, required, &required),
        GTD_OK);
    cr_assert_str_eq(exact, "km/h");

    cr_assert_eq(
        gtd_channel_unit_parse("kph", GTD_CHANNEL_UNIT_RECOGNIZED, exact, required - 1, &required),
        GTD_ERR_OUT_OF_RANGE);
    free(exact);
}

Test(status_codes, a_setter_after_a_nav_fix_is_a_call_order_error) {
    GtdFileBuilder *builder = builder_with_a_nav_fix();

    cr_assert_eq(gtd_builder_set_title(builder, "late"), GTD_ERR_CALL_ORDER);
    cr_assert_eq(gtd_builder_set_device(builder, "late"), GTD_ERR_CALL_ORDER);
    cr_assert_eq(gtd_builder_set_notes(builder, "late"), GTD_ERR_CALL_ORDER);
    cr_assert_eq(gtd_builder_set_identity(builder, "late"), GTD_ERR_CALL_ORDER);
    cr_assert_eq(gtd_builder_set_travel_mode(builder, GTD_TRAVEL_MODE_CAR), GTD_ERR_CALL_ORDER);
    cr_assert_eq(gtd_builder_set_lenient(builder), GTD_ERR_CALL_ORDER);
    cr_assert_eq(gtd_builder_set_satellite_window_us(builder, 2000000), GTD_ERR_CALL_ORDER);
    cr_assert_not_null(gtd_last_error());

    gtd_builder_destroy(builder);
}

Test(status_codes, a_setter_before_any_data_is_ok) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    cr_assert_eq(gtd_builder_set_title(builder, "early"), GTD_OK);
    cr_assert_eq(gtd_builder_set_device(builder, "early"), GTD_OK);
    cr_assert_eq(gtd_builder_set_notes(builder, "early"), GTD_OK);
    cr_assert_eq(gtd_builder_set_identity(builder, "early"), GTD_OK);
    cr_assert_eq(gtd_builder_set_travel_mode(builder, GTD_TRAVEL_MODE_CAR), GTD_OK);
    cr_assert_eq(gtd_builder_set_lenient(builder), GTD_OK);
    cr_assert_eq(gtd_builder_set_satellite_window_us(builder, 2000000), GTD_OK);

    gtd_builder_destroy(builder);
}
