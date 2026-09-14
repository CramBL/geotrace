#include "../geotrace.h"
#include "test_helpers.h"
#include <criterion/criterion.h>
#include <stdint.h>
#include <string.h>

static const int64_t FIRST_FIX_SECONDS = 1700000000;
static const int64_t MARKER_SECONDS = 1700000005;
static const int64_t LAST_FIX_SECONDS = 1700000010;
static const uint8_t UNRECOGNIZED_ICON_CODE = 200;

static GtdNavFile *build_markers_and_styles(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp first_fix;
    GtdTimestamp marker_time;
    GtdTimestamp last_fix;
    cr_assert_eq(gtd_ts_from_seconds(FIRST_FIX_SECONDS, &first_fix), GTD_OK);
    cr_assert_eq(gtd_ts_from_seconds(MARKER_SECONDS, &marker_time), GTD_OK);
    cr_assert_eq(gtd_ts_from_seconds(LAST_FIX_SECONDS, &last_fix), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, first_fix, gtd_ts_none(), 51.0, -1.0,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix(builder, last_fix, gtd_ts_none(), 52.0, -2.0, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    cr_assert_eq(gtd_builder_add_annotation(builder, marker_time, "waypoint", GTD_ICON_LIGHTNING),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_annotation(builder, marker_time, NULL, GTD_ICON_PIN), GTD_OK);

    cr_assert_eq(
        gtd_builder_add_event_marker_style(builder, "power/boot", GTD_ICON_WARNING, "#FF9900"),
        GTD_OK);
    cr_assert_eq(gtd_builder_add_event_marker_style(builder, "power/sleep", GTD_ICON_AUTO, NULL),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    cr_assert_not_null(file);
    return file;
}

Test(markers, labelled_marker_reads_back_with_its_icon_time_and_position) {
    GtdNavFile *file = build_markers_and_styles();
    cr_assert_eq(gtd_nav_file_marker_count(file), 2);

    GtdMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_marker(file, 0, &marker), GTD_OK);
    cr_assert_eq(marker.has_label, 1);
    cr_assert_str_eq(marker.label, "waypoint");
    cr_assert_eq(marker.icon, GTD_ICON_LIGHTNING);
    cr_assert_eq(marker.icon_code, GTD_ICON_LIGHTNING);
    cr_assert_eq(marker.time.unix_micros, MARKER_SECONDS * 1000000);
    assert_near(marker.lat_deg, 51.5, 1e-9);
    assert_near(marker.lon_deg, -1.5, 1e-9);

    gtd_nav_file_destroy(file);
}

Test(markers, unlabelled_marker_reads_back_with_an_empty_label) {
    GtdNavFile *file = build_markers_and_styles();

    GtdMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_marker(file, 1, &marker), GTD_OK);
    cr_assert_eq(marker.has_label, 0);
    cr_assert_str_eq(marker.label, "");
    cr_assert_eq(marker.icon, GTD_ICON_PIN);
    cr_assert_eq(marker.icon_code, GTD_ICON_PIN);

    gtd_nav_file_destroy(file);
}

Test(markers, marker_index_past_the_last_marker_is_out_of_range) {
    GtdNavFile *file = build_markers_and_styles();

    GtdMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_marker(file, 2, &marker), GTD_ERR_OUT_OF_RANGE);

    gtd_nav_file_destroy(file);
}

Test(markers, style_with_an_explicit_icon_and_color_reads_back) {
    GtdNavFile *file = build_markers_and_styles();
    cr_assert_eq(gtd_nav_file_event_marker_style_count(file), 2);

    GtdEventMarkerStyleInfo style;
    cr_assert_eq(gtd_nav_file_get_event_marker_style(file, 0, &style), GTD_OK);
    cr_assert_str_eq(style.variant_path, "power/boot");
    cr_assert_eq(style.icon, GTD_ICON_WARNING);
    cr_assert_str_eq(style.icon_name, "warning");
    cr_assert_eq(style.has_color, 1);
    cr_assert_str_eq(style.color_hex, "#FF9900");

    gtd_nav_file_destroy(file);
}

Test(markers, style_that_leaves_the_icon_and_color_to_the_application_reads_back_as_auto) {
    GtdNavFile *file = build_markers_and_styles();

    GtdEventMarkerStyleInfo style;
    cr_assert_eq(gtd_nav_file_get_event_marker_style(file, 1, &style), GTD_OK);
    cr_assert_str_eq(style.variant_path, "power/sleep");
    cr_assert_eq(style.icon, GTD_ICON_AUTO);
    cr_assert_str_eq(style.icon_name, "");
    cr_assert_eq(style.has_color, 0);
    cr_assert_str_eq(style.color_hex, "");

    gtd_nav_file_destroy(file);
}

Test(markers, style_index_past_the_last_style_is_out_of_range) {
    GtdNavFile *file = build_markers_and_styles();

    GtdEventMarkerStyleInfo style;
    cr_assert_eq(gtd_nav_file_get_event_marker_style(file, 2, &style), GTD_ERR_OUT_OF_RANGE);

    gtd_nav_file_destroy(file);
}

typedef struct {
    const char *variant_path;
    const char *color_hex;
    GtdStatus status;
    const char *message;
} RejectedStyle;

static const RejectedStyle REJECTED_STYLES[] = {
    {"power/boot", "red", GTD_ERR_INVALID_ARGUMENT,
     "invalid event marker color \"red\": expected the #RRGGBB form"},
    {"power/boot", "FF9900", GTD_ERR_INVALID_ARGUMENT,
     "invalid event marker color \"FF9900\": expected the #RRGGBB form"},
    {"power/boot", "   ", GTD_ERR_INVALID_ARGUMENT,
     "invalid event marker color \"   \": expected the #RRGGBB form"},
    {"", NULL, GTD_ERR_INVALID_PATH, "invalid event marker variant path \"\": path is empty"},
    {"über_lang", NULL, GTD_ERR_INVALID_PATH,
     "invalid event marker variant path \"über_lang\": contains characters outside ASCII "
     "alphanumeric, hyphen, underscore, and slash"},
};

Test(markers, a_style_the_rust_builder_rejects_is_rejected_where_it_is_added) {
    size_t case_count = sizeof(REJECTED_STYLES) / sizeof(REJECTED_STYLES[0]);
    for (size_t i = 0; i < case_count; i++) {
        const RejectedStyle *rejected = &REJECTED_STYLES[i];
        GtdTimestamp timestamp;
        GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);
        cr_assert_eq(gtd_builder_add_event_marker_style(builder, rejected->variant_path,
                                                        GTD_ICON_AUTO, rejected->color_hex),
                     rejected->status);
        cr_assert_str_eq(gtd_last_error(), rejected->message);
        gtd_builder_destroy(builder);
    }
}

Test(markers, a_style_path_past_its_field_is_rejected_where_it_is_added) {
    GtdTimestamp timestamp;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);
    char long_path[257];
    memset(long_path, 'p', sizeof long_path - 1);
    long_path[sizeof long_path - 1] = '\0';
    cr_assert_eq(gtd_builder_add_event_marker_style(builder, long_path, GTD_ICON_AUTO, NULL),
                 GTD_ERR_FIELD_TOO_LONG);
    cr_assert_not_null(strstr(gtd_last_error(), "256 bytes, past the 255 bytes the field holds"));
    gtd_builder_destroy(builder);
}

typedef struct {
    const char *value;
    uint8_t has_value;
    const char *read_back;
} EmptyOrWhitespaceOnlyString;

static const EmptyOrWhitespaceOnlyString EMPTY_OR_WHITESPACE_ONLY_STRINGS[] = {
    {"", 0, ""},
    {"   ", 1, "   "},
};

static void assert_label_and_annotation(const GtdNavFile *file,
                                        const EmptyOrWhitespaceOnlyString *expected) {
    GtdMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_marker(file, 0, &marker), GTD_OK);
    cr_assert_eq(marker.has_label, expected->has_value);
    cr_assert_str_eq(marker.label, expected->read_back);
    GtdEventMarkerInfo event_marker;
    cr_assert_eq(gtd_nav_file_get_event_marker(file, 0, &event_marker), GTD_OK);
    cr_assert_eq(event_marker.has_annotation, expected->has_value);
    cr_assert_str_eq(event_marker.annotation, expected->read_back);
}

Test(markers, a_label_or_annotation_is_absent_when_empty_and_kept_when_whitespace_only) {
    size_t case_count =
        sizeof(EMPTY_OR_WHITESPACE_ONLY_STRINGS) / sizeof(EMPTY_OR_WHITESPACE_ONLY_STRINGS[0]);
    for (size_t i = 0; i < case_count; i++) {
        const EmptyOrWhitespaceOnlyString *string = &EMPTY_OR_WHITESPACE_ONLY_STRINGS[i];
        GtdTimestamp timestamp;
        GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);
        cr_assert_eq(gtd_builder_add_annotation(builder, timestamp, string->value, GTD_ICON_PIN),
                     GTD_OK);
        cr_assert_eq(gtd_builder_add_event_marker(builder, "power/boot", timestamp, string->value),
                     GTD_OK);
        GtdNavFile *built = NULL;
        cr_assert_eq(gtd_builder_finish(builder, &built), GTD_OK);
        assert_label_and_annotation(built, string);
        GtdNavFile *read_back = reload_through_bytes(built);
        assert_label_and_annotation(read_back, string);
        gtd_nav_file_destroy(read_back);
    }
}

#ifdef GTD_UNRECOGNIZED_MARKER_ICON_FIXTURE_PATH
Test(markers, an_icon_code_outside_the_icon_set_reads_back_as_a_pin_with_its_code) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(GTD_UNRECOGNIZED_MARKER_ICON_FIXTURE_PATH, &file), GTD_OK);
    cr_assert_eq(gtd_nav_file_marker_count(file), 1);

    GtdMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_marker(file, 0, &marker), GTD_OK);
    cr_assert_str_eq(marker.label, "hovercraft");
    cr_assert_eq(marker.icon, GTD_ICON_PIN);
    cr_assert_eq(marker.icon_code, UNRECOGNIZED_ICON_CODE);

    gtd_nav_file_destroy(file);
}
#endif

#ifdef GTD_UNRECOGNIZED_STYLE_VALUES_FIXTURE_PATH
Test(markers, an_icon_name_and_color_outside_the_known_values_read_back_verbatim) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(GTD_UNRECOGNIZED_STYLE_VALUES_FIXTURE_PATH, &file), GTD_OK);
    cr_assert_eq(gtd_nav_file_event_marker_style_count(file), 1);

    GtdEventMarkerStyleInfo style;
    cr_assert_eq(gtd_nav_file_get_event_marker_style(file, 0, &style), GTD_OK);
    cr_assert_str_eq(style.variant_path, "power/boot");
    cr_assert_eq(style.icon, GTD_ICON_AUTO);
    cr_assert_str_eq(style.icon_name, "hovercraft");
    cr_assert_eq(style.has_color, 1);
    cr_assert_str_eq(style.color_hex, "FFAA00");

    gtd_nav_file_destroy(file);
}
#endif
