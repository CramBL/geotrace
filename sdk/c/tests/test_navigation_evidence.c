#include "test_helpers.h"
#include <string.h>
#include <stdlib.h>

static GtdSatelliteReportInput report_at(GtdTimestamp time) {
    GtdSatelliteReportInput report = {0};
    report.gps_time = gtd_ts_none();
    report.sys_time = time;
    report.scope.kind = GTD_SATELLITE_SCOPE_KIND_GLOBAL;
    report.coverage = GTD_SATELLITE_COVERAGE_COMPLETE;
    return report;
}

Test(navigation_evidence, navigation_metadata_and_optional_values_round_trip) {
    const uint32_t states[] = {GTD_SOLUTION_STATE_UNREPORTED, GTD_SOLUTION_STATE_FIX,
                               GTD_SOLUTION_STATE_NO_FIX, GTD_SOLUTION_STATE_ESTIMATED_ONLY,
                               GTD_SOLUTION_STATE_UNKNOWN};
    const uint32_t methods[] = {
        GTD_SOLUTION_METHOD_UNREPORTED,   GTD_SOLUTION_METHOD_AUTONOMOUS,
        GTD_SOLUTION_METHOD_DIFFERENTIAL, GTD_SOLUTION_METHOD_RTK_FLOAT,
        GTD_SOLUTION_METHOD_RTK_FIXED,    GTD_SOLUTION_METHOD_DEAD_RECKONING,
        GTD_SOLUTION_METHOD_HYBRID,       GTD_SOLUTION_METHOD_OTHER};
    GtdFileBuilder *builder = gtd_builder_create();
    GtdNavFixV2 fix = gtd_nav_fix_v2_init();
    cr_assert_eq(fix.position_provenance, GTD_POSITION_PROVENANCE_SOURCE_PROVIDED);
    cr_assert_eq(fix.state, GTD_SOLUTION_STATE_UNREPORTED);
    cr_assert_eq(fix.used_count.present, 0);
    for (size_t i = 0; i < 8; ++i) {
        cr_assert_eq(gtd_ts_from_seconds(1700000000 + (int64_t)i, &fix.sys_time), GTD_OK);
        fix.state = states[i % 5];
        fix.method = methods[i];
        fix.dimension = (uint32_t)(i % 3);
        fix.position_provenance = (uint32_t)(i % 3);
        fix.used_count = (GtdOptU32){i == 2 ? UINT32_MAX : 0, i == 0 ? 0 : 1};
        fix.hdop = GTD_SOME_F64(0.9);
        fix.vdop = GTD_SOME_F64(1.2);
        fix.pdop = GTD_SOME_F64(1.5);
        cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, &fix), GTD_OK);
    }
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    file = reload_through_bytes(file);
    for (size_t i = 0; i < 8; ++i) {
        GtdNavPointInfoV2 point;
        cr_assert_eq(gtd_nav_file_get_nav_point_v2(file, i, &point), GTD_OK);
        cr_assert_eq(point.fix.state, states[i % 5]);
        cr_assert_eq(point.fix.method, methods[i]);
        cr_assert_eq(point.fix.dimension, i % 3);
        cr_assert_eq(point.fix.position_provenance, i % 3);
        cr_assert_eq(point.fix.used_count.present, i != 0);
        cr_assert_eq(point.fix.used_count.value, i == 2 ? UINT32_MAX : 0);
        cr_assert_eq(point.fix.heading_deg.present, 0);
        cr_assert_eq(point.fix.eph_m.present, 0);
        cr_assert_eq(point.fix.hdop.present, 1);
        assert_near(point.fix.hdop.value, 0.9, 1e-10);
        assert_near(point.fix.vdop.value, 1.2, 1e-10);
        assert_near(point.fix.pdop.value, 1.5, 1e-10);
        cr_assert_eq(point.usage_report_count, 0);
        cr_assert_eq(point.observation_report_count, 0);
    }
    gtd_nav_file_destroy(file);
}

Test(navigation_evidence, ordered_samples_scopes_and_copied_rows_round_trip) {
    GtdTimestamp time;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&time);
    GtdSatelliteReportInput report = report_at(time);
    report.gps_time = time;
    report.sys_time.unix_micros += 1000000;
    GtdSatelliteUsage *rows = malloc(3 * sizeof(*rows));
    cr_assert_not_null(rows);
    rows[0] = (GtdSatelliteUsage){GTD_CONSTELLATION_GPS, 1, GTD_SATELLITE_MEMBERSHIP_UNUSED};
    rows[1] = (GtdSatelliteUsage){GTD_CONSTELLATION_GPS, 1, GTD_SATELLITE_MEMBERSHIP_USED};
    rows[2] = (GtdSatelliteUsage){GTD_CONSTELLATION_GPS, 2, GTD_SATELLITE_MEMBERSHIP_UNKNOWN};
    report.reported_count = (GtdOptU32){8, 1};
    report.coverage = GTD_SATELLITE_COVERAGE_PARTIAL;
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, rows, 3), GTD_OK);
    rows[0].prn = 99;
    free(rows);
    report.scope =
        (GtdSatelliteScope){GTD_SATELLITE_SCOPE_KIND_CONSTELLATIONS, 1U << GTD_CONSTELLATION_GPS};
    report.reported_count = (GtdOptU32){0, 1};
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, NULL, 0), GTD_OK);
    report.scope.constellation_mask = (1U << 6) - 1U;
    report.reported_count.present = 0;
    report.coverage = GTD_SATELLITE_COVERAGE_COMPLETE;
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, NULL, 0), GTD_OK);
    GtdSatelliteObservation observations[2] = {
        {GTD_CONSTELLATION_GPS, 1, GTD_NONE_F32, GTD_SOME_F32(0), GTD_SOME_F32(0)},
        {GTD_CONSTELLATION_GPS, 1, GTD_SOME_F32(45), GTD_NONE_F32, GTD_NONE_F32}};
    report.scope = (GtdSatelliteScope){GTD_SATELLITE_SCOPE_KIND_GLOBAL, 0};
    report.reported_count = (GtdOptU32){12, 1};
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, observations, 2),
                 GTD_OK);
    observations[0].prn = 99;
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, NULL, 0), GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    file = reload_through_bytes(file);
    GtdNavPointInfoV2 point;
    cr_assert_eq(gtd_nav_file_get_nav_point_v2(file, 0, &point), GTD_OK);
    cr_assert_eq(point.usage_report_count, 3);
    cr_assert_eq(point.observation_report_count, 2);
    cr_assert_eq(point.fix.position_provenance, GTD_POSITION_PROVENANCE_SOURCE_PROVIDED);
    for (size_t i = 0; i < 3; ++i) {
        GtdSatelliteReportInfo info;
        cr_assert_eq(gtd_nav_file_get_satellite_usage_report(file, 0, i, &info), GTD_OK);
        cr_assert_eq(info.source.scope.kind, i == 0 ? GTD_SATELLITE_SCOPE_KIND_GLOBAL
                                                    : GTD_SATELLITE_SCOPE_KIND_CONSTELLATIONS);
        cr_assert_eq(info.source.scope.constellation_mask, i == 0 ? 0 : i == 1 ? 1 : 63);
        cr_assert_eq(info.source.coverage,
                     i == 2 ? GTD_SATELLITE_COVERAGE_COMPLETE : GTD_SATELLITE_COVERAGE_PARTIAL);
        cr_assert_eq(info.source.reported_count.present, i != 2);
        cr_assert_eq(info.source.reported_count.value, i == 0 ? 8 : 0);
        cr_assert_eq(info.source.gps_time.unix_micros, time.unix_micros);
        cr_assert_eq(info.source.sys_time.unix_micros, time.unix_micros + 1000000);
        cr_assert_eq(info.normalized_time.unix_micros, time.unix_micros);
        cr_assert_eq(info.placement_present, 1);
        cr_assert_eq(info.input_order, i);
        cr_assert_eq(info.row_count, i == 0 ? 3 : 0);
    }
    for (size_t i = 0; i < 3; ++i) {
        GtdSatelliteUsage row;
        cr_assert_eq(gtd_nav_file_get_satellite_usage(file, 0, 0, i, &row), GTD_OK);
        cr_assert_eq(row.prn, i == 2 ? 2 : 1);
        cr_assert_eq(row.membership, i == 0   ? GTD_SATELLITE_MEMBERSHIP_UNUSED
                                     : i == 1 ? GTD_SATELLITE_MEMBERSHIP_USED
                                              : GTD_SATELLITE_MEMBERSHIP_UNKNOWN);
    }
    GtdSatelliteObservation row;
    cr_assert_eq(gtd_nav_file_get_satellite_observation(file, 0, 0, 0, &row), GTD_OK);
    cr_assert_eq(row.prn, 1);
    cr_assert_eq(row.elevation_deg.present, 0);
    cr_assert_eq(row.azimuth_deg.present, 1);
    cr_assert_eq(row.snr_dbhz.present, 1);
    cr_assert_eq(row.snr_dbhz.value, 0);
    cr_assert_eq(gtd_nav_file_get_satellite_observation(file, 0, 0, 1, &row), GTD_OK);
    cr_assert_eq(row.prn, 1);
    cr_assert_eq(row.elevation_deg.present, 1);
    cr_assert_eq(row.snr_dbhz.present, 0);
    GtdSatelliteReportInfo info;
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(file, 0, 0, &info), GTD_OK);
    cr_assert_eq(info.input_order, 3);
    cr_assert_eq(info.source.reported_count.value, 12);
    cr_assert_eq(info.row_count, 2);
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(file, 0, 1, &info), GTD_OK);
    cr_assert_eq(info.input_order, 4);
    cr_assert_eq(info.row_count, 0);
    GtdNavPointInfo legacy;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &legacy), GTD_OK);
    cr_assert_eq(legacy.sat_count, 0);
    cr_assert(gtd_ts_is_none(legacy.sat_report_gps_time));
    cr_assert(gtd_ts_is_none(legacy.sat_report_sys_time));
    gtd_nav_file_destroy(file);
}

Test(navigation_evidence, invalid_metadata_and_late_rows_leave_the_builder_unchanged) {
    GtdTimestamp time;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &time), GTD_OK);
    GtdFileBuilder *builder = gtd_builder_create();
    GtdNavFixV2 fix = gtd_nav_fix_v2_init();
    fix.sys_time = time;
    uint32_t *tags[] = {&fix.state, &fix.method, &fix.dimension, &fix.position_provenance};
    const char *fields[] = {"state", "method", "dimension", "position_provenance"};
    const uint32_t first_invalid[] = {5, 8, 3, 3};
    for (size_t i = 0; i < 4; ++i) {
        uint32_t saved = *tags[i];
        const uint32_t invalid[] = {first_invalid[i], 257, UINT32_MAX};
        for (size_t j = 0; j < 3; ++j) {
            *tags[i] = invalid[j];
            cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, &fix), GTD_ERR_INVALID_ARGUMENT);
            cr_assert_not_null(strstr(gtd_last_error(), fields[i]));
        }
        *tags[i] = saved;
    }
    GtdSatelliteReportInput report = report_at(time);
    const GtdSatelliteScope scopes[] = {{99, 0},
                                        {GTD_SATELLITE_SCOPE_KIND_GLOBAL, 1},
                                        {GTD_SATELLITE_SCOPE_KIND_CONSTELLATIONS, 0},
                                        {GTD_SATELLITE_SCOPE_KIND_CONSTELLATIONS, UINT32_MAX}};
    for (size_t i = 0; i < 4; ++i) {
        report.scope = scopes[i];
        cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, NULL, 0),
                     GTD_ERR_INVALID_ARGUMENT);
        cr_assert_not_null(strstr(gtd_last_error(), "scope"));
    }
    report.scope = (GtdSatelliteScope){GTD_SATELLITE_SCOPE_KIND_GLOBAL, 0};
    report.coverage = UINT32_MAX;
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, NULL, 0),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(strstr(gtd_last_error(), "coverage"));
    report.coverage = GTD_SATELLITE_COVERAGE_COMPLETE;
    GtdSatelliteUsage rows[] = {{GTD_CONSTELLATION_GPS, 1, GTD_SATELLITE_MEMBERSHIP_USED},
                                {GTD_CONSTELLATION_GPS, 2, UINT32_MAX}};
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, rows, 2),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(strstr(gtd_last_error(), "rows[1].membership"));
    rows[1].membership = GTD_SATELLITE_MEMBERSHIP_UNUSED;
    rows[1].constellation = UINT32_MAX;
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, rows, 2),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(strstr(gtd_last_error(), "rows[1].constellation"));
    GtdSatelliteObservation observations[] = {
        {GTD_CONSTELLATION_GPS, 1, GTD_NONE_F32, GTD_NONE_F32, GTD_NONE_F32},
        {UINT32_MAX, 2, GTD_NONE_F32, GTD_NONE_F32, GTD_NONE_F32}};
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, observations, 2),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(strstr(gtd_last_error(), "rows[1].constellation"));
    cr_assert_eq(gtd_builder_set_title(builder, "Still configurable"), GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, &fix), GTD_OK);
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, rows, 1), GTD_OK);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, observations, 1),
                 GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    GtdNavPointInfoV2 point;
    cr_assert_eq(gtd_nav_file_get_nav_point_v2(file, 0, &point), GTD_OK);
    cr_assert_eq(point.usage_report_count, 1);
    cr_assert_eq(point.observation_report_count, 1);
    GtdSatelliteReportInfo info;
    cr_assert_eq(gtd_nav_file_get_satellite_usage_report(file, 0, 0, &info), GTD_OK);
    cr_assert_eq(info.row_count, 1);
    cr_assert_eq(info.input_order, 0);
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(file, 0, 0, &info), GTD_OK);
    cr_assert_eq(info.row_count, 1);
    cr_assert_eq(info.input_order, 1);
    gtd_nav_file_destroy(file);
}

Test(navigation_evidence, timestamp_null_and_index_errors_preserve_outputs) {
    GtdTimestamp time;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&time);
    GtdNavFixV2 fix = gtd_nav_fix_v2_init();
    cr_assert_eq(gtd_builder_add_nav_fix_v2(NULL, &fix), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, &fix), GTD_ERR_INVALID_ARGUMENT);
    fix.sys_time.unix_micros = INT64_MAX;
    cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, &fix), GTD_ERR_OUT_OF_RANGE);
    cr_assert_not_null(strstr(gtd_last_error(), "sys_time"));
    GtdSatelliteReportInput report = report_at(time);
    cr_assert_eq(gtd_builder_add_satellite_usage_report(NULL, &report, NULL, 0),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(NULL, &report, NULL, 0),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, NULL, NULL, 0),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, NULL, NULL, 0),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, NULL, 1),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, NULL, 1),
                 GTD_ERR_NULL_ARGUMENT);
    GtdSatelliteUsage oversized_usage = {0};
    GtdSatelliteObservation oversized_observation = {0};
    cr_assert_eq(
        gtd_builder_add_satellite_usage_report(builder, &report, &oversized_usage, SIZE_MAX),
        GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report,
                                                              &oversized_observation, SIZE_MAX),
                 GTD_ERR_OUT_OF_RANGE);
    report.sys_time = gtd_ts_none();
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, NULL, 0),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, NULL, 0),
                 GTD_ERR_INVALID_ARGUMENT);
    report.gps_time.unix_micros = INT64_MAX;
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, NULL, 0),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, NULL, 0),
                 GTD_ERR_OUT_OF_RANGE);
    report = report_at(time);
    GtdSatelliteUsage usage = {GTD_CONSTELLATION_GPS, 1, GTD_SATELLITE_MEMBERSHIP_UNKNOWN};
    GtdSatelliteObservation observation = {GTD_CONSTELLATION_GPS, 1, GTD_NONE_F32, GTD_NONE_F32,
                                           GTD_NONE_F32};
    cr_assert_eq(gtd_builder_add_satellite_usage_report(builder, &report, &usage, 1), GTD_OK);
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, &observation, 1),
                 GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    GtdNavPointInfoV2 point = {.usage_report_count = 123};
    GtdSatelliteReportInfo info = {.row_count = 123};
    usage.prn = 123;
    observation.prn = 123;
    cr_assert_eq(gtd_nav_file_get_nav_point_v2(NULL, 0, &point), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_nav_point_v2(file, 0, NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_nav_point_v2(file, 1, &point), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(point.usage_report_count, 123);
    cr_assert_eq(gtd_nav_file_get_satellite_usage_report(NULL, 0, 0, &info), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(NULL, 0, 0, &info),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_usage_report(file, 0, 0, NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(file, 0, 0, NULL),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_usage_report(file, 1, 0, &info), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(file, 1, 0, &info),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_usage_report(file, 0, 1, &info), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(file, 0, 1, &info),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(info.row_count, 123);
    cr_assert_eq(gtd_nav_file_get_satellite_usage(NULL, 0, 0, 0, &usage), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_observation(NULL, 0, 0, 0, &observation),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_usage(file, 0, 0, 0, NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_observation(file, 0, 0, 0, NULL),
                 GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_nav_file_get_satellite_usage(file, 1, 0, 0, &usage), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_observation(file, 1, 0, 0, &observation),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_usage(file, 0, 1, 0, &usage), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_observation(file, 0, 1, 0, &observation),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_usage(file, 0, 0, 1, &usage), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_nav_file_get_satellite_observation(file, 0, 0, 1, &observation),
                 GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(usage.prn, 123);
    cr_assert_eq(observation.prn, 123);
    gtd_nav_file_destroy(file);
}

Test(navigation_evidence, common_host_clock_orphan_retains_time_and_position) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdNavFixV2 fix = gtd_nav_fix_v2_init();
    fix.state = GTD_SOLUTION_STATE_FIX;
    fix.used_count = (GtdOptU32){8, 1};
    fix.hdop = GTD_SOME_F64(0.9);
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &fix.sys_time), GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, &fix), GTD_OK);
    cr_assert_eq(gtd_ts_from_seconds(1700000010, &fix.sys_time), GTD_OK);
    fix.lon_deg = 10;
    cr_assert_eq(gtd_builder_add_nav_fix_v2(builder, &fix), GTD_OK);
    GtdTimestamp poll;
    cr_assert_eq(gtd_ts_from_seconds(1700000002, &poll), GTD_OK);
    GtdSatelliteReportInput report = report_at(poll);
    report.reported_count = (GtdOptU32){12, 1};
    cr_assert_eq(gtd_builder_add_satellite_observation_report(builder, &report, NULL, 0), GTD_OK);
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    file = reload_through_bytes(file);
    cr_assert_eq(gtd_nav_file_nav_point_count(file), 3);
    GtdNavPointInfoV2 point;
    cr_assert_eq(gtd_nav_file_get_nav_point_v2(file, 0, &point), GTD_OK);
    cr_assert_eq(point.fix.state, GTD_SOLUTION_STATE_FIX);
    cr_assert_eq(point.fix.used_count.value, 8);
    cr_assert_eq(point.observation_report_count, 0);
    cr_assert_eq(gtd_nav_file_get_nav_point_v2(file, 1, &point), GTD_OK);
    cr_assert_eq(point.fix.position_provenance, GTD_POSITION_PROVENANCE_SDK_SYNTHESIZED);
    cr_assert_eq(point.fix.state, GTD_SOLUTION_STATE_UNREPORTED);
    cr_assert_eq(point.fix.gps_time.unix_micros, poll.unix_micros);
    assert_near(point.fix.lon_deg, 2, 1e-8);
    cr_assert_eq(point.usage_report_count, 0);
    cr_assert_eq(point.observation_report_count, 1);
    GtdSatelliteReportInfo info;
    cr_assert_eq(gtd_nav_file_get_satellite_observation_report(file, 1, 0, &info), GTD_OK);
    cr_assert_eq(info.normalized_time.unix_micros, poll.unix_micros);
    cr_assert_eq(info.source.sys_time.unix_micros, poll.unix_micros);
    cr_assert(gtd_ts_is_none(info.source.gps_time));
    cr_assert_eq(info.input_order, 0);
    GtdNavPointInfo legacy;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 1, &legacy), GTD_OK);
    cr_assert_eq(legacy.sat_count, 0);
    cr_assert(gtd_ts_is_none(legacy.sat_report_sys_time));
    gtd_nav_file_destroy(file);
}
