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

typedef struct {
    char name[256];
    uint8_t has_unit;
    char unit[64];
    GtdOptF64 period_deg;
    uint8_t has_description;
    char description[1024];
    size_t component_count;
    size_t sample_count;
} FrozenGtdChannelInfoV040;

Test(channels, published_v040_layout_is_preserved) {
    cr_assert_eq(sizeof(GtdChannel), sizeof(FrozenGtdChannelV040));
    cr_assert_eq(sizeof(GtdChannelInfo), sizeof(FrozenGtdChannelInfoV040));
    cr_assert_eq(offsetof(GtdChannel, period_deg), offsetof(FrozenGtdChannelV040, period_deg));
    cr_assert_eq(offsetof(GtdChannelInfo, period_deg),
                 offsetof(FrozenGtdChannelInfoV040, period_deg));
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
    cr_assert_eq(info.has_unit, 1);
    cr_assert_str_eq(info.unit, "g");
    cr_assert_eq(info.component_count, 3);
    cr_assert_eq(info.sample_count, 2);
    cr_assert_eq(info.period_deg.present, 0);

    char label[16];
    cr_assert_eq(gtd_nav_file_get_channel_component(reloaded, 0, 2, label, sizeof(label)), GTD_OK);
    cr_assert_str_eq(label, "z");

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
    cr_assert_eq(info.component_count, 0);
    cr_assert_eq(info.period_deg.present, 1);
    assert_near(info.period_deg.value, 360.0, 1e-9);

    gtd_nav_file_destroy(reloaded);
    gtd_free_bytes(buf, len);
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
