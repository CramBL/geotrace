/* Each test here passes three values that the `enum` of a parameter or struct
   field does not declare, and asserts what the entry point returns for such a
   value. */

#include "../geotrace.h"
#include "test_helpers.h"
#include <criterion/criterion.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

Test(invalid_enums, a_travel_mode_outside_the_enum_is_rejected) {
    static const uint32_t modes[] = {7, 99, 200};

    for (size_t i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
        GtdFileBuilder *builder = gtd_builder_create();
        cr_assert_not_null(builder);
        cr_assert_eq(gtd_builder_set_travel_mode(builder, modes[i]), GTD_ERR_INVALID_ARGUMENT);
        gtd_builder_destroy(builder);
    }
}

Test(invalid_enums, the_name_of_a_travel_mode_outside_the_enum_is_unknown) {
    static const uint32_t modes[] = {7, 99, 200};

    for (size_t i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
        const char *name = gtd_travel_mode_name(modes[i]);
        cr_assert_not_null(name);
        cr_assert_str_eq(name, "unknown");
    }
}

/* NOLINTBEGIN(clang-analyzer-optin.core.EnumCastOutOfRange): the cast below
   produces a discriminant outside `GtdLogLevel`'s declared range. */

/* NOLINTBEGIN(bugprone-easily-swappable-parameters): the C SDK fixes the order
   of a log callback's parameters. */
static void count_records(GtdLogLevel level, const char *target, const char *message,
                          void *user_data) {
    /* NOLINTEND(bugprone-easily-swappable-parameters) */
    (void)level;
    (void)target;
    (void)message;
    size_t *count = (size_t *)user_data;
    (*count)++;
}

Test(invalid_enums, a_log_level_outside_the_enum_leaves_the_forwarded_level_in_force) {
    static const int32_t levels[] = {0, 6, 99};

    size_t count = 0;
    cr_assert_eq(gtd_set_log_callback(count_records, &count), GTD_OK);

    for (size_t i = 0; i < sizeof(levels) / sizeof(levels[0]); i++) {
        gtd_set_log_level(GTD_LOG_ERROR);
        gtd_set_log_level((GtdLogLevel)levels[i]);
        /* The builder reports the PRN of 0 and the SNR of 99 dB-Hz at
           GTD_LOG_WARN, which GTD_LOG_ERROR holds back. */
        gtd_nav_file_destroy(build_file_with_satellite_issues());
        cr_assert_eq(count, 0);
    }

    gtd_clear_log_callback();
}
/* NOLINTEND(clang-analyzer-optin.core.EnumCastOutOfRange) */

Test(invalid_enums, an_annotation_icon_outside_the_enum_is_rejected) {
    /* The icons run 0 to 13 and GTD_ICON_AUTO is 255. These three values lie
       in the gap between them. */
    static const uint32_t icons[] = {14, 200, 254};

    for (size_t i = 0; i < sizeof(icons) / sizeof(icons[0]); i++) {
        GtdTimestamp time;
        GtdFileBuilder *builder = builder_with_a_nav_fix(&time);
        cr_assert_eq(gtd_builder_add_annotation(builder, time, "waypoint", icons[i]),
                     GTD_ERR_INVALID_ARGUMENT);
        cr_assert_not_null(strstr(gtd_last_error(), "not a valid GtdMarkerIcon"));
        gtd_builder_destroy(builder);
    }
}

Test(invalid_enums, an_event_marker_style_icon_outside_the_enum_is_rejected) {
    static const uint32_t icons[] = {14, 200, 254};

    for (size_t i = 0; i < sizeof(icons) / sizeof(icons[0]); i++) {
        GtdTimestamp time;
        GtdFileBuilder *builder = builder_with_a_nav_fix(&time);
        cr_assert_eq(gtd_builder_add_event_marker_style(builder, "power/boot", icons[i], "#FFAA00"),
                     GTD_ERR_INVALID_ARGUMENT);
        cr_assert_not_null(strstr(gtd_last_error(), "not a valid GtdMarkerIcon"));
        gtd_builder_destroy(builder);
    }
}

Test(invalid_enums, a_satellite_constellation_outside_the_enum_is_rejected) {
    static const uint32_t constellations[] = {6, 42, 255};

    for (size_t i = 0; i < sizeof(constellations) / sizeof(constellations[0]); i++) {
        GtdTimestamp time;
        GtdFileBuilder *builder = builder_with_a_nav_fix(&time);
        /* The second entry holds the value outside the `enum`: the builder
           checks the constellation of every satellite of the report. */
        GtdSatellite satellites[2] = {
            {GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F32(45.0F), GTD_SOME_F32(90.0F),
             GTD_SOME_F32(40.0F)},
            {constellations[i], 7, 1, GTD_SOME_F32(30.0F), GTD_SOME_F32(120.0F),
             GTD_SOME_F32(38.0F)},
        };
        cr_assert_eq(gtd_builder_add_satellite_report(builder, time, gtd_ts_none(), satellites, 2),
                     GTD_ERR_INVALID_ARGUMENT);
        gtd_builder_destroy(builder);
    }
}

Test(invalid_enums, the_builder_stays_unchanged_when_it_rejects_a_satellite_report) {
    const uint32_t constellation_outside_the_enum = 42;

    GtdTimestamp time;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&time);
    GtdSatellite satellites[2] = {
        {GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F32(45.0F), GTD_SOME_F32(90.0F),
         GTD_SOME_F32(40.0F)},
        {constellation_outside_the_enum, 7, 1, GTD_SOME_F32(30.0F), GTD_SOME_F32(120.0F),
         GTD_SOME_F32(38.0F)},
    };
    cr_assert_eq(gtd_builder_add_satellite_report(builder, time, gtd_ts_none(), satellites, 2),
                 GTD_ERR_INVALID_ARGUMENT);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &point), GTD_OK);
    cr_assert_eq(point.sat_count, 0);
    gtd_nav_file_destroy(file);
}

Test(invalid_enums, a_channel_unit_mode_outside_the_enum_is_rejected) {
    static const uint32_t unit_modes[] = {2, 7, UINT32_MAX};

    for (size_t i = 0; i < sizeof(unit_modes) / sizeof(unit_modes[0]); i++) {
        char canonical[16];
        size_t required = 0;
        cr_assert_eq(
            gtd_channel_unit_parse("m/s", unit_modes[i], canonical, sizeof canonical, &required),
            GTD_ERR_INVALID_CHANNEL);

        GtdTimestamp time;
        GtdFileBuilder *builder = builder_with_a_nav_fix(&time);
        double values[1] = {1.0};
        GtdChannel channel = {0};
        channel.name = "speed";
        channel.unit = "m/s";
        channel.period_deg = GTD_NONE_F64;
        channel.times = &time;
        channel.n_times = 1;
        channel.values = values;
        channel.n_values = 1;
        cr_assert_eq(gtd_builder_add_channel_with_unit_mode(builder, &channel, unit_modes[i]),
                     GTD_ERR_INVALID_CHANNEL);
        gtd_builder_destroy(builder);
    }
}
