#include "../geotrace.h"
#include <criterion/criterion.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

#define assert_near(a, b, eps) cr_assert(fabs((a) - (b)) < (eps))

Test(builder, basic_write) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    cr_assert_eq(gtd_builder_set_title(b, "Test file"), GTD_OK);
    cr_assert_eq(gtd_builder_set_device(b, "criterion test"), GTD_OK);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    cr_assert_eq(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 51.5074, -0.1278, GTD_SOME_F64(180.0),
                                         GTD_SOME_F64(3.0), GTD_SOME_F64(5.0)),
                 GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);
    cr_assert_not_null(f);

    cr_assert_eq(gtd_nav_file_nav_point_count(f), 1);

    GtdNavPointInfo p;
    cr_assert_eq(gtd_nav_file_get_nav_point(f, 0, &p), GTD_OK);
    assert_near(p.lat_deg, 51.5074, 1e-9);
    assert_near(p.lon_deg, -0.1278, 1e-9);
    cr_assert_eq(p.speed_mps.present, 1);
    assert_near(p.speed_mps.value, 3.0, 1e-9);

    gtd_nav_file_destroy(f);
}

Test(builder, to_bytes_round_trip) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    cr_assert_eq(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 48.8566, 2.3522, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(f, &buf, &len), GTD_OK);
    cr_assert_not_null(buf);
    cr_assert(len > 0);
    gtd_nav_file_destroy(f);

    GtdNavFile *f2 = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(buf, len, &f2), GTD_OK);
    cr_assert_not_null(f2);
    cr_assert_eq(gtd_nav_file_nav_point_count(f2), 1);

    GtdNavPointInfo p;
    cr_assert_eq(gtd_nav_file_get_nav_point(f2, 0, &p), GTD_OK);
    assert_near(p.lat_deg, 48.8566, 1e-6);

    gtd_nav_file_destroy(f2);
    gtd_free_bytes(buf, len);
}

Test(builder, travel_mode_round_trip) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    cr_assert_eq(gtd_builder_set_travel_mode(b, GTD_TRAVEL_MODE_BICYCLE), GTD_OK);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    cr_assert_eq(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 51.5074, -0.1278, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(f, &buf, &len), GTD_OK);
    gtd_nav_file_destroy(f);

    GtdNavFile *f2 = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(buf, len, &f2), GTD_OK);

    const char *name = gtd_nav_file_travel_mode(f2);
    cr_assert_not_null(name);
    cr_assert_str_eq(name, "bicycle");

    GtdTravelMode mode;
    cr_assert_eq(gtd_travel_mode_from_name(name, &mode), GTD_OK);
    cr_assert_eq(mode, GTD_TRAVEL_MODE_BICYCLE);

    gtd_nav_file_destroy(f2);
    gtd_free_bytes(buf, len);
}

Test(builder, travel_mode_absent_is_null) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    cr_assert_eq(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 51.5074, -0.1278, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);

    cr_assert_null(gtd_nav_file_travel_mode(f));

    gtd_nav_file_destroy(f);
}

Test(travel_mode, name_round_trips_through_from_name) {
    const GtdTravelMode all[] = {
        GTD_TRAVEL_MODE_CAR,      GTD_TRAVEL_MODE_MOTORCYCLE, GTD_TRAVEL_MODE_BICYCLE,
        GTD_TRAVEL_MODE_BOAT,     GTD_TRAVEL_MODE_PEDESTRIAN, GTD_TRAVEL_MODE_RAIL,
        GTD_TRAVEL_MODE_AIRCRAFT,
    };
    for (size_t i = 0; i < sizeof(all) / sizeof(all[0]); i++) {
        const char *name = gtd_travel_mode_name(all[i]);
        cr_assert_not_null(name);
        GtdTravelMode parsed;
        cr_assert_eq(gtd_travel_mode_from_name(name, &parsed), GTD_OK);
        cr_assert_eq(parsed, all[i]);
    }
}

Test(travel_mode, from_name_rejects_unknown) {
    GtdTravelMode mode;
    cr_assert_eq(gtd_travel_mode_from_name("hovercraft", &mode), GTD_ERR_PARSE);
}

Test(builder, no_fixes_error) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    /* NoNavFixes is only returned when there are annotations but no fixes.
       An empty builder (no fixes, no annotations) is valid and returns OK. */
    cr_assert_eq(
        gtd_builder_add_annotation(b, gtd_ts_from_seconds(1700000000ULL), "note", GTD_ICON_AUTO),
        GTD_OK);

    GtdNavFile *f = NULL;
    GtdStatus s = gtd_builder_finish(b, &f);
    cr_assert_eq(s, GTD_ERR_NO_NAV_FIXES);
    cr_assert_null(f);
    cr_assert_not_null(gtd_last_error());
}

Test(builder, satellite_report) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_SOME_F64(55.0), GTD_SOME_F64(120.0), GTD_SOME_F64(40.0)},
        {GTD_CONSTELLATION_GLONASS, 2, 0, GTD_NONE_F64, GTD_NONE_F64, GTD_SOME_F64(28.0)},
    };

    cr_assert_eq(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 40.7128, -74.0060, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    cr_assert_eq(gtd_builder_add_satellite_report(b, t, gtd_ts_none(), sats, 2), GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);

    GtdNavPointInfo p;
    cr_assert_eq(gtd_nav_file_get_nav_point(f, 0, &p), GTD_OK);
    cr_assert_eq(p.sat_count, 2);

    GtdSatInfo s0;
    cr_assert_eq(gtd_nav_file_get_satellite(f, 0, 0, &s0), GTD_OK);
    cr_assert_eq(s0.prn, 7);
    cr_assert_eq(s0.in_fix, 1);
    cr_assert_eq(s0.snr_dbhz.present, 1);
    assert_near(s0.snr_dbhz.value, 40.0, 1e-6);

    gtd_nav_file_destroy(f);
}

Test(builder, event_marker) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    cr_assert_eq(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 35.6762, 139.6503, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    cr_assert_eq(gtd_builder_add_event_marker(b, "system/startup", t, "Device started"), GTD_OK);

    cr_assert_eq(gtd_builder_add_event_marker_style(b, "system/startup", GTD_ICON_GEAR, "#00FF00"),
                 GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);

    cr_assert_eq(gtd_nav_file_event_marker_count(f), 1);

    GtdEventMarkerInfo em;
    cr_assert_eq(gtd_nav_file_get_event_marker(f, 0, &em), GTD_OK);
    cr_assert_str_eq(em.variant_path, "system/startup");
    cr_assert_eq(em.has_annotation, 1);
    cr_assert_str_eq(em.annotation, "Device started");

    gtd_nav_file_destroy(f);
}

Test(builder, event_marker_variant_path_past_its_field_is_too_long) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    char variant_path[257];
    memset(variant_path, 'a', sizeof(variant_path) - 1);
    variant_path[sizeof(variant_path) - 1] = '\0';

    cr_assert_eq(gtd_builder_add_event_marker(b, variant_path, t, NULL), GTD_ERR_FIELD_TOO_LONG);

    gtd_builder_destroy(b);
}

Test(builder, event_marker_annotation_past_its_field_is_too_long) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    char annotation[513];
    memset(annotation, 'a', sizeof(annotation) - 1);
    annotation[sizeof(annotation) - 1] = '\0';

    cr_assert_eq(gtd_builder_add_event_marker(b, "system/startup", t, annotation),
                 GTD_ERR_FIELD_TOO_LONG);

    gtd_builder_destroy(b);
}

Test(builder, annotation_label_past_its_field_is_too_long) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    char label[257];
    memset(label, 'l', sizeof(label) - 1);
    label[sizeof(label) - 1] = '\0';

    cr_assert_eq(gtd_builder_add_annotation(b, t, label, GTD_ICON_AUTO), GTD_ERR_FIELD_TOO_LONG);

    gtd_builder_destroy(b);
}

Test(builder, event_marker_style_color_past_its_field_is_too_long_when_written) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    cr_assert_eq(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 35.6762, 139.6503, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    cr_assert_eq(
        gtd_builder_add_event_marker_style(b, "system/startup", GTD_ICON_AUTO, "#00FF00FF"),
        GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(f, &buf, &len), GTD_ERR_FIELD_TOO_LONG);

    gtd_nav_file_destroy(f);
}

#ifdef GTD_FIXTURE_PATH
Test(builder, open_fixture) {
    GtdNavFile *f = NULL;
    GtdStatus s = gtd_nav_file_open(GTD_FIXTURE_PATH, &f);
    cr_assert_eq(s, GTD_OK);
    cr_assert_not_null(f);
    cr_assert(gtd_nav_file_nav_point_count(f) >= 1);
    gtd_nav_file_destroy(f);
}
#endif
