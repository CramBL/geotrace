#include "../geotrace.h"
#include "test_helpers.h"
#include <criterion/criterion.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    const char *name;
    const char *unit;
    GtdOptF64 period_deg;
    const char *description;
    const char *const *components;
    size_t n_components;
    const GtdTimestamp *times;
    size_t n_times;
    const double *values;
    size_t n_values;
} FrozenGtdChannelV040;

Test(channels, published_v040_input_layout_is_preserved) {
    cr_assert_eq(sizeof(GtdChannel), sizeof(FrozenGtdChannelV040));
    cr_assert_eq(offsetof(GtdChannel, period_deg), offsetof(FrozenGtdChannelV040, period_deg));
}

Test(channels, frozen_v040_input_layout_calls_current_library) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp time;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &time), GTD_OK);
    double value = 1.0;
    FrozenGtdChannelV040 channel = {0};
    channel.name = "incline";
    channel.unit = "deg";
    channel.period_deg = GTD_NONE_F64;
    channel.times = &time;
    channel.n_times = 1;
    channel.values = &value;
    channel.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(builder, (const GtdChannel *)&channel), GTD_OK);
    gtd_builder_destroy(builder);
}

/* Write a scalar and a vector channel, then read them back from a byte buffer. */
Test(channels, round_trip) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp first_time;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &first_time), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, first_time, gtd_ts_none(), 51.5, -0.1,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdTimestamp second_time;
    cr_assert_eq(gtd_ts_from_seconds(1700000001, &second_time), GTD_OK);
    GtdTimestamp times[2] = {first_time, second_time};

    /* A scalar channel carrying a wrap period. */
    double incline_vals[2] = {1.5, 2.0};
    GtdChannel incline = {0};
    incline.name = "incline";
    incline.unit = "deg";
    incline.period_deg = GTD_SOME_F64(360.0);
    incline.times = times;
    incline.n_times = 2;
    incline.values = incline_vals;
    incline.n_values = 2;
    cr_assert_eq(gtd_builder_add_channel(builder, &incline), GTD_OK);

    /* A vector channel, values row-major: [x0, y0, z0, x1, y1, z1]. */
    const char *comps[3] = {"x", "y", "z"};
    double accel_vals[6] = {0.1, 0.2, 0.98, -0.1, 0.3, 1.02};
    GtdChannel accel = {0};
    accel.name = "accel";
    accel.unit = "g";
    accel.period_deg = GTD_NONE_F64;
    accel.components = comps;
    accel.n_components = 3;
    accel.times = times;
    accel.n_times = 2;
    accel.values = accel_vals;
    accel.n_values = 6;
    cr_assert_eq(gtd_builder_add_channel(builder, &accel), GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(file, &buf, &len), GTD_OK);
    gtd_nav_file_destroy(file);

    GtdNavFile *reloaded = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(buf, len, &reloaded), GTD_OK);
    cr_assert_eq(gtd_nav_file_channel_count(reloaded), 2);

    /* Channels sort by name: accel (vector) at 0, incline (scalar) at 1. */
    GtdChannelInfo info;
    cr_assert_eq(gtd_nav_file_get_channel(reloaded, 0, &info), GTD_OK);
    cr_assert_str_eq(info.name, "accel");
    cr_assert_str_eq(info.unit, "g");
    cr_assert_eq(info.component_count, 3);
    cr_assert_str_eq(info.components[2], "z");
    cr_assert_eq(info.sample_count, 2);
    cr_assert_eq(info.period_deg.present, 0);

    GtdTimestamp got_times[2];
    cr_assert_eq(gtd_nav_file_channel_times(reloaded, 0, got_times, 2), 2);
    cr_assert_eq(got_times[0].unix_micros, times[0].unix_micros);
    cr_assert_eq(got_times[1].unix_micros, times[1].unix_micros);

    double got_vals[6];
    cr_assert_eq(gtd_nav_file_channel_values(reloaded, 0, got_vals, 6), 6);
    assert_near(got_vals[0], 0.1, 1e-12);
    assert_near(got_vals[5], 1.02, 1e-12);

    /* A smaller cap copies only `cap` values but still reports the true total. */
    double partial[6] = {-1, -1, -1, -1, -1, -1};
    cr_assert_eq(gtd_nav_file_channel_values(reloaded, 0, partial, 3), 6);
    assert_near(partial[0], 0.1, 1e-12);
    cr_assert(partial[3] == -1.0); /* untouched beyond cap */

    /* A NULL out / zero cap queries the count without copying. */
    cr_assert_eq(gtd_nav_file_channel_times(reloaded, 0, NULL, 0), 2);
    cr_assert_eq(gtd_nav_file_channel_values(reloaded, 0, NULL, 0), 6);

    cr_assert_eq(gtd_nav_file_get_channel(reloaded, 1, &info), GTD_OK);
    cr_assert_str_eq(info.name, "incline");
    cr_assert_null(info.description);
    cr_assert_eq(info.component_count, 0);
    cr_assert_null(info.components);
    cr_assert_eq(info.period_deg.present, 1);
    assert_near(info.period_deg.value, 360.0, 1e-9);

    gtd_nav_file_destroy(reloaded);
    gtd_free_bytes(buf, len);
}

Test(channels, get_channel_strings_stay_valid_until_the_file_is_destroyed) {
    GtdTimestamp timestamp;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);
    const char *components[3] = {"x", "y", "z"};
    double accel_values[3] = {0.1, 0.2, 0.98};
    GtdChannel accel = {0};
    accel.name = "accel";
    accel.unit = "g";
    accel.period_deg = GTD_NONE_F64;
    accel.components = components;
    accel.n_components = 3;
    accel.times = &timestamp;
    accel.n_times = 1;
    accel.values = accel_values;
    accel.n_values = 3;
    cr_assert_eq(gtd_builder_add_channel(builder, &accel), GTD_OK);
    double temp_value = 20.0;
    GtdChannel temp = {0};
    temp.name = "temp";
    temp.period_deg = GTD_NONE_F64;
    temp.times = &timestamp;
    temp.n_times = 1;
    temp.values = &temp_value;
    temp.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(builder, &temp), GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    GtdChannelInfo accel_info;
    cr_assert_eq(gtd_nav_file_get_channel(file, 0, &accel_info), GTD_OK);
    GtdChannelInfo temp_info;
    cr_assert_eq(gtd_nav_file_get_channel(file, 1, &temp_info), GTD_OK);
    GtdChannelInfo accel_info_again;
    cr_assert_eq(gtd_nav_file_get_channel(file, 0, &accel_info_again), GTD_OK);

    cr_assert_eq(accel_info.name, accel_info_again.name);
    cr_assert_eq(accel_info.components, accel_info_again.components);
    cr_assert_str_eq(accel_info.name, "accel");
    cr_assert_str_eq(accel_info.unit, "g");
    cr_assert_str_eq(accel_info.components[0], "x");
    cr_assert_str_eq(temp_info.name, "temp");
    cr_assert_null(temp_info.unit);
    cr_assert_null(temp_info.description);
    cr_assert_null(temp_info.components);
    gtd_nav_file_destroy(file);
}

#ifdef GTD_CHANNEL_DESCRIPTION_WITH_A_NUL_BYTE_FIXTURE_PATH
Test(channels, get_channel_returns_invalid_channel_for_a_description_with_a_nul_byte) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(GTD_CHANNEL_DESCRIPTION_WITH_A_NUL_BYTE_FIXTURE_PATH, &file),
                 GTD_OK);
    GtdChannelInfo info;
    cr_assert_eq(gtd_nav_file_get_channel(file, 0, &info), GTD_ERR_INVALID_CHANNEL);
    cr_assert_str_eq(gtd_last_error(), "channel 0: the description has a nul byte at offset 6");
    cr_assert_eq(gtd_nav_file_channel_times(file, 0, NULL, 0), 1);
    gtd_nav_file_destroy(file);
}
#endif

static void repeat_into(char *out, const char *piece, size_t count) {
    size_t piece_length = strlen(piece);
    for (size_t i = 0; i < count; i++) {
        memcpy(out + (i * piece_length), piece, piece_length);
    }
    out[count * piece_length] = '\0';
}

static GtdNavFile *reload_through_bytes(GtdNavFile *file) {
    uint8_t *bytes = NULL;
    size_t length = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(file, &bytes, &length), GTD_OK);
    gtd_nav_file_destroy(file);
    GtdNavFile *reloaded = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(bytes, length, &reloaded), GTD_OK);
    gtd_free_bytes(bytes, length);
    return reloaded;
}

Test(channels, get_channel_returns_long_multibyte_strings_whole) {
    char name[301];
    name[0] = 'n';
    repeat_into(name + 1, "a", 299);
    char unit[81];
    repeat_into(unit, "µ", 40);
    char description[1201];
    repeat_into(description, "µ", 600);
    char first_component[301];
    first_component[0] = 'c';
    repeat_into(first_component + 1, "b", 299);
    const char *components[2] = {first_component, "y"};

    GtdTimestamp timestamp;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);
    double values[2] = {1.0, 2.0};
    GtdChannel channel = {0};
    channel.name = name;
    channel.unit = unit;
    channel.period_deg = GTD_NONE_F64;
    channel.description = description;
    channel.components = components;
    channel.n_components = 2;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = values;
    channel.n_values = 2;
    cr_assert_eq(gtd_builder_add_channel_with_unit_mode(builder, &channel, GTD_CHANNEL_UNIT_CUSTOM),
                 GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    GtdNavFile *reloaded = reload_through_bytes(file);

    GtdChannelInfo info;
    cr_assert_eq(gtd_nav_file_get_channel(reloaded, 0, &info), GTD_OK);
    cr_assert_str_eq(info.name, name);
    cr_assert_str_eq(info.unit, unit);
    cr_assert_str_eq(info.description, description);
    cr_assert_eq(info.component_count, 2);
    cr_assert_str_eq(info.components[0], first_component);
    cr_assert_str_eq(info.components[1], "y");
    gtd_nav_file_destroy(reloaded);
}

Test(channels, get_channel_unit_returns_out_of_range_and_leaves_a_short_buffer_unwritten) {
    GtdTimestamp timestamp;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);
    double value = 1.0;
    GtdChannel channel = {0};
    channel.name = "speed";
    channel.unit = "km/h";
    channel.period_deg = GTD_NONE_F64;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = &value;
    channel.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    char unit[3] = {'z', 'z', 'z'};
    size_t required_length = 0;
    cr_assert_eq(gtd_nav_file_get_channel_unit(file, 0, unit, sizeof unit, &required_length, NULL),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(required_length, sizeof "km/h");
    cr_assert_arr_eq(unit, "zzz", sizeof unit);
    gtd_nav_file_destroy(file);
}

Test(channels, invalid_name_is_rejected) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    double value = 1.0;
    GtdChannel channel = {0};
    channel.name = "Bad Name";
    channel.period_deg = GTD_NONE_F64;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = &value;
    channel.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_ERR_INVALID_CHANNEL);
    gtd_builder_destroy(builder);
}

Test(channels, unrecognized_unit_requires_custom_mode) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    double value = 1200.0;
    GtdChannel channel = {0};
    channel.name = "shaft_speed";
    channel.unit = "rpm";
    channel.period_deg = GTD_NONE_F64;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = &value;
    channel.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_ERR_INVALID_CHANNEL);

    cr_assert_eq(gtd_builder_add_channel_with_unit_mode(builder, &channel, GTD_CHANNEL_UNIT_CUSTOM),
                 GTD_OK);
    gtd_builder_destroy(builder);
}

Test(channels, long_custom_unit_uses_lossless_accessor) {
    GtdFileBuilder *builder = gtd_builder_create();
    char label[160];
    memset(label, 'x', sizeof(label) - 1);
    label[sizeof(label) - 1] = '\0';
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    double value = 1.0;
    GtdChannel channel = {0};
    channel.name = "quality";
    channel.unit = label;
    channel.period_deg = GTD_NONE_F64;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = &value;
    channel.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel_with_unit_mode(builder, &channel, GTD_CHANNEL_UNIT_CUSTOM),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    size_t required_len = 0;
    uint8_t is_custom = 0;
    cr_assert_eq(gtd_nav_file_get_channel_unit(file, 0, NULL, 0, &required_len, &is_custom),
                 GTD_OK);
    cr_assert_eq(required_len, sizeof(label));
    cr_assert_eq(is_custom, 1);
    char *read_label = malloc(required_len);
    cr_assert_not_null(read_label);
    cr_assert_eq(
        gtd_nav_file_get_channel_unit(file, 0, read_label, required_len, &required_len, &is_custom),
        GTD_OK);
    cr_assert_str_eq(read_label, label);
    free(read_label);
    gtd_nav_file_destroy(file);
}

Test(channels, a_sample_time_past_the_range_is_out_of_range) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp times[2];
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &times[0]), GTD_OK);
    times[1].unix_micros = INT64_MAX;
    double values[2] = {1.0, 2.0};
    GtdChannel channel = {0};
    channel.name = "accel";
    channel.period_deg = GTD_NONE_F64;
    channel.times = times;
    channel.n_times = 2;
    channel.values = values;
    channel.n_values = 2;
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_ERR_OUT_OF_RANGE);
    cr_assert_str_eq(gtd_last_error(), "times[1]: " INT64_MAX_MICROS_PAST_THE_RANGE_MESSAGE);
    gtd_builder_destroy(builder);
}

Test(channels, length_mismatch_is_rejected) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    double values[2] = {1.0, 2.0};
    GtdChannel channel = {0};
    channel.name = "accel";
    channel.period_deg = GTD_NONE_F64;
    channel.times = &timestamp;
    channel.n_times = 1; /* one sample */
    channel.values = values;
    channel.n_values = 2; /* but two scalar values */
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_ERR_INVALID_CHANNEL);
    gtd_builder_destroy(builder);
}

Test(channels, invalid_component_is_rejected) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    double values[2] = {1.0, 2.0};
    const char *dup[2] = {"x", "x"}; /* duplicate component label */
    GtdChannel channel = {0};
    channel.name = "accel";
    channel.period_deg = GTD_NONE_F64;
    channel.components = dup;
    channel.n_components = 2;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = values;
    channel.n_values = 2;
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_ERR_INVALID_CHANNEL);
    gtd_builder_destroy(builder);
}

Test(channels, duplicate_name_fails_at_finish) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    double value = 1.0;
    GtdChannel channel = {0};
    channel.name = "accel";
    channel.period_deg = GTD_NONE_F64;
    channel.times = &timestamp;
    channel.n_times = 1;
    channel.values = &value;
    channel.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_OK);
    cr_assert_eq(gtd_builder_add_channel(builder, &channel), GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_ERR_INVALID_CHANNEL);
    cr_assert_null(file);
}

Test(channels, unit_validation_uses_shared_unicode_rules) {
    struct UnitCase {
        const char *label;
        GtdChannelUnitMode mode;
        GtdStatus expected;
        const char *canonical;
    } cases[] = {
        {"\xC2\xA0", GTD_CHANNEL_UNIT_CUSTOM, GTD_ERR_INVALID_CHANNEL, NULL},
        {"\xE2\x80\x83", GTD_CHANNEL_UNIT_CUSTOM, GTD_ERR_INVALID_CHANNEL, NULL},
        {"bad\xC2\x85"
         "unit",
         GTD_CHANNEL_UNIT_CUSTOM, GTD_ERR_INVALID_CHANNEL, NULL},
        {"micrograms", GTD_CHANNEL_UNIT_CUSTOM, GTD_OK, "micrograms"},
        {"m/s\xC2\xB2", GTD_CHANNEL_UNIT_RECOGNIZED, GTD_OK, "m/s2"},
        {"m/s\xC2\xB2", GTD_CHANNEL_UNIT_CUSTOM, GTD_ERR_INVALID_CHANNEL, NULL},
    };

    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i) {
        size_t required = 0;
        cr_assert_eq(gtd_channel_unit_parse(cases[i].label, cases[i].mode, NULL, 0, &required),
                     cases[i].expected);
        if (cases[i].expected != GTD_OK) {
            continue;
        }
        char *canonical = malloc(required);
        cr_assert_not_null(canonical);
        cr_assert_eq(
            gtd_channel_unit_parse(cases[i].label, cases[i].mode, canonical, required, &required),
            GTD_OK);
        cr_assert_str_eq(canonical, cases[i].canonical);
        free(canonical);
    }
}
