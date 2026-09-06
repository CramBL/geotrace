#include "../geotrace.h"
#include "test_helpers.h"
#include <criterion/criterion.h>
#include <stdint.h>

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
