#include "../geotrace.h"
#include "test_helpers.h"
#include <criterion/criterion.h>
#include <stdio.h>
#include <string.h>

Test(builder, basic_write) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    cr_assert_eq(gtd_builder_set_title(builder, "Test file"), GTD_OK);
    cr_assert_eq(gtd_builder_set_device(builder, "criterion test"), GTD_OK);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5074, -0.1278,
                                         GTD_SOME_F64(180.0), GTD_SOME_F64(3.0), GTD_SOME_F64(5.0)),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    cr_assert_not_null(file);

    cr_assert_eq(gtd_nav_file_nav_point_count(file), 1);

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &point), GTD_OK);
    assert_near(point.lat_deg, 51.5074, 1e-9);
    assert_near(point.lon_deg, -0.1278, 1e-9);
    cr_assert_eq(point.speed_mps.present, 1);
    assert_near(point.speed_mps.value, 3.0, 1e-9);

    gtd_nav_file_destroy(file);
}

Test(builder, to_bytes_round_trip) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 48.8566, 2.3522,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(file, &buf, &len), GTD_OK);
    cr_assert_not_null(buf);
    cr_assert(len > 0);
    gtd_nav_file_destroy(file);

    GtdNavFile *reloaded = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(buf, len, &reloaded), GTD_OK);
    cr_assert_not_null(reloaded);
    cr_assert_eq(gtd_nav_file_nav_point_count(reloaded), 1);

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(reloaded, 0, &point), GTD_OK);
    assert_near(point.lat_deg, 48.8566, 1e-6);

    gtd_nav_file_destroy(reloaded);
    gtd_free_bytes(buf, len);
}

Test(builder, travel_mode_round_trip) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    cr_assert_eq(gtd_builder_set_travel_mode(builder, GTD_TRAVEL_MODE_BICYCLE), GTD_OK);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5074, -0.1278,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(file, &buf, &len), GTD_OK);
    gtd_nav_file_destroy(file);

    GtdNavFile *reloaded = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(buf, len, &reloaded), GTD_OK);

    const char *name = gtd_nav_file_travel_mode(reloaded);
    cr_assert_not_null(name);
    cr_assert_str_eq(name, "bicycle");

    GtdTravelMode mode;
    cr_assert_eq(gtd_travel_mode_from_name(name, &mode), GTD_OK);
    cr_assert_eq(mode, GTD_TRAVEL_MODE_BICYCLE);

    gtd_nav_file_destroy(reloaded);
    gtd_free_bytes(buf, len);
}

Test(builder, travel_mode_absent_is_null) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5074, -0.1278,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    cr_assert_null(gtd_nav_file_travel_mode(file));

    gtd_nav_file_destroy(file);
}

Test(builder, a_build_without_provenance_writes_only_the_sdk_version) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5074, -0.1278,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(file, &buf, &len), GTD_OK);
    gtd_nav_file_destroy(file);

    GtdNavFile *reloaded = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(buf, len, &reloaded), GTD_OK);

    const char *version = gtd_nav_file_sdk_version(reloaded);
    cr_assert_not_null(version);
    cr_assert_str_eq(version, GEOTRACE_C_VERSION);

    cr_assert_null(gtd_nav_file_sdk_git_commit(reloaded));
    cr_assert(gtd_ts_is_none(gtd_nav_file_sdk_commit_time(reloaded)));

    gtd_nav_file_destroy(reloaded);
    gtd_free_bytes(buf, len);
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
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    /* NoNavFixes is only returned when there are annotations but no fixes.
       An empty builder (no fixes, no annotations) is valid and returns OK. */
    cr_assert_eq(gtd_builder_add_annotation(builder, timestamp, "note", GTD_ICON_PIN), GTD_OK);

    GtdNavFile *file = NULL;
    GtdStatus status = gtd_builder_finish(builder, &file);
    cr_assert_eq(status, GTD_ERR_NO_NAV_FIXES);
    cr_assert_null(file);
    cr_assert_not_null(gtd_last_error());
}

Test(builder, nav_fix_without_a_timestamp) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, gtd_ts_none(), gtd_ts_none(), 51.5074, -0.1278,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    gtd_builder_destroy(builder);
}

Test(builder, satellite_report_without_a_timestamp) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_NONE_F32, GTD_NONE_F32, GTD_NONE_F32},
    };

    cr_assert_eq(gtd_builder_add_satellite_report(builder, gtd_ts_none(), gtd_ts_none(), sats, 1),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    gtd_builder_destroy(builder);
}

Test(builder, satellite_report) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_SOME_F32(55.0F), GTD_SOME_F32(120.0F),
         GTD_SOME_F32(40.0F)},
        {GTD_CONSTELLATION_GLONASS, 2, 0, GTD_NONE_F32, GTD_NONE_F32, GTD_SOME_F32(28.0F)},
    };

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 40.7128, -74.0060,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    cr_assert_eq(gtd_builder_add_satellite_report(builder, timestamp, gtd_ts_none(), sats, 2),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &point), GTD_OK);
    cr_assert_eq(point.sat_count, 2);

    GtdSatInfo satellite;
    cr_assert_eq(gtd_nav_file_get_satellite(file, 0, 0, &satellite), GTD_OK);
    cr_assert_eq(satellite.prn, 7);
    cr_assert_eq(satellite.in_fix, 1);
    cr_assert_eq(satellite.snr_dbhz.present, 1);
    cr_assert_eq(satellite.snr_dbhz.value, 40.0F);

    gtd_nav_file_destroy(file);
}

Test(builder, a_nav_point_reads_back_the_timestamps_of_its_satellite_report) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp first_fix;
    GtdTimestamp report_sys_time;
    GtdTimestamp second_fix;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &first_fix), GTD_OK);
    cr_assert_eq(gtd_ts_from_seconds(1700000001, &report_sys_time), GTD_OK);
    cr_assert_eq(gtd_ts_from_seconds(1700000010, &second_fix), GTD_OK);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_SOME_F32(55.0F), GTD_SOME_F32(120.0F),
         GTD_SOME_F32(40.0F)},
    };

    cr_assert_eq(gtd_builder_add_nav_fix(builder, first_fix, gtd_ts_none(), 40.7128, -74.0060,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_satellite_report(builder, first_fix, report_sys_time, sats, 1),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix(builder, second_fix, gtd_ts_none(), 40.7130, -74.0065,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    GtdNavPointInfo with_report;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &with_report), GTD_OK);
    cr_assert_eq(with_report.sat_report_gps_time.unix_micros, first_fix.unix_micros);
    cr_assert_eq(with_report.sat_report_sys_time.unix_micros, report_sys_time.unix_micros);

    GtdNavPointInfo without_report;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 1, &without_report), GTD_OK);
    cr_assert_eq(without_report.sat_count, 0);
    cr_assert(gtd_ts_is_none(without_report.sat_report_gps_time));
    cr_assert(gtd_ts_is_none(without_report.sat_report_sys_time));

    gtd_nav_file_destroy(file);
}

Test(builder, a_report_with_only_a_receiver_time_reads_back_without_a_host_time) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp fix_time;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &fix_time), GTD_OK);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_NONE_F32, GTD_NONE_F32, GTD_NONE_F32},
    };

    cr_assert_eq(gtd_builder_add_nav_fix(builder, fix_time, gtd_ts_none(), 40.7128, -74.0060,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_satellite_report(builder, fix_time, gtd_ts_none(), sats, 1),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &point), GTD_OK);
    cr_assert_eq(point.sat_count, 1);
    cr_assert_eq(point.sat_report_gps_time.unix_micros, fix_time.unix_micros);
    cr_assert(gtd_ts_is_none(point.sat_report_sys_time));

    gtd_nav_file_destroy(file);
}

Test(builder, a_report_with_only_a_host_time_reads_back_without_a_receiver_time) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp fix_time;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &fix_time), GTD_OK);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_NONE_F32, GTD_NONE_F32, GTD_NONE_F32},
    };

    cr_assert_eq(gtd_builder_add_nav_fix(builder, fix_time, fix_time, 40.7128, -74.0060,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_satellite_report(builder, gtd_ts_none(), fix_time, sats, 1),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    GtdNavPointInfo point;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &point), GTD_OK);
    cr_assert_eq(point.sat_count, 1);
    cr_assert(gtd_ts_is_none(point.sat_report_gps_time));
    cr_assert_eq(point.sat_report_sys_time.unix_micros, fix_time.unix_micros);

    gtd_nav_file_destroy(file);
}

Test(builder, satellite_metrics_round_trip_bit_exact) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_SOME_F32(38.5F), GTD_SOME_F32(359.9999F),
         GTD_SOME_F32(38.123456789F)},
    };

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 40.7128, -74.0060,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_satellite_report(builder, timestamp, gtd_ts_none(), sats, 1),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    GtdSatInfo satellite;
    cr_assert_eq(gtd_nav_file_get_satellite(file, 0, 0, &satellite), GTD_OK);
    cr_assert_eq(satellite.elevation_deg.value, 38.5F);
    cr_assert_eq(satellite.azimuth_deg.value, 359.9999F);
    cr_assert_eq(satellite.snr_dbhz.value, 38.123456789F);

    gtd_nav_file_destroy(file);
}

Test(builder, event_marker) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 35.6762, 139.6503,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    cr_assert_eq(
        gtd_builder_add_event_marker(builder, "system/startup", timestamp, "Device started"),
        GTD_OK);

    cr_assert_eq(
        gtd_builder_add_event_marker_style(builder, "system/startup", GTD_ICON_GEAR, "#00FF00"),
        GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    cr_assert_eq(gtd_nav_file_event_marker_count(file), 1);

    GtdEventMarkerInfo marker;
    cr_assert_eq(gtd_nav_file_get_event_marker(file, 0, &marker), GTD_OK);
    cr_assert_str_eq(marker.variant_path, "system/startup");
    cr_assert_eq(marker.has_annotation, 1);
    cr_assert_str_eq(marker.annotation, "Device started");

    gtd_nav_file_destroy(file);
}

Test(builder, event_marker_variant_path_past_its_field_is_too_long) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    char variant_path[257];
    memset(variant_path, 'a', sizeof(variant_path) - 1);
    variant_path[sizeof(variant_path) - 1] = '\0';

    cr_assert_eq(gtd_builder_add_event_marker(builder, variant_path, timestamp, NULL),
                 GTD_ERR_FIELD_TOO_LONG);

    gtd_builder_destroy(builder);
}

Test(builder, event_marker_annotation_past_its_field_is_too_long) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    char annotation[513];
    memset(annotation, 'a', sizeof(annotation) - 1);
    annotation[sizeof(annotation) - 1] = '\0';

    cr_assert_eq(gtd_builder_add_event_marker(builder, "system/startup", timestamp, annotation),
                 GTD_ERR_FIELD_TOO_LONG);

    gtd_builder_destroy(builder);
}

Test(builder, annotation_label_past_its_field_is_too_long) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    char label[257];
    memset(label, 'l', sizeof(label) - 1);
    label[sizeof(label) - 1] = '\0';

    cr_assert_eq(gtd_builder_add_annotation(builder, timestamp, label, GTD_ICON_PIN),
                 GTD_ERR_FIELD_TOO_LONG);

    gtd_builder_destroy(builder);
}

Test(builder, annotation_rejects_the_auto_icon) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_annotation(builder, timestamp, "note", GTD_ICON_AUTO),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    gtd_builder_destroy(builder);
}

Test(builder, event_marker_style_color_past_its_field_is_too_long_when_written) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 35.6762, 139.6503,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    cr_assert_eq(
        gtd_builder_add_event_marker_style(builder, "system/startup", GTD_ICON_AUTO, "#00FF00FF"),
        GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(file, &buf, &len), GTD_ERR_FIELD_TOO_LONG);

    gtd_nav_file_destroy(file);
}

#ifdef GTD_FIXTURE_PATH
Test(builder, open_fixture) {
    GtdNavFile *file = NULL;
    GtdStatus status = gtd_nav_file_open(GTD_FIXTURE_PATH, &file);
    cr_assert_eq(status, GTD_OK);
    cr_assert_not_null(file);
    cr_assert(gtd_nav_file_nav_point_count(file) >= 1);
    gtd_nav_file_destroy(file);
}
#endif
