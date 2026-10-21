#include "../geotrace.h"
#include <stdio.h>

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdNavFile *file = NULL;
    GtdNavFixV2 fix = gtd_nav_fix_v2_init();
    fix.state = GTD_SOLUTION_STATE_FIX;
    fix.used_count = GTD_SOME_U32(8);
    fix.hdop = GTD_SOME_F64(0.9);
    if (gtd_ts_from_seconds(1700000000, &fix.sys_time) != GTD_OK ||
        gtd_builder_add_nav_fix_v2(builder, &fix) != GTD_OK) {
        goto fail;
    }
    fix.lon_deg = 10;
    if (gtd_ts_from_seconds(1700000010, &fix.sys_time) != GTD_OK ||
        gtd_builder_add_nav_fix_v2(builder, &fix) != GTD_OK) {
        goto fail;
    }
    GtdSatelliteReportInput poll = {0};
    poll.gps_time = gtd_ts_none();
    poll.scope.kind = GTD_SATELLITE_SCOPE_KIND_GLOBAL;
    poll.coverage = GTD_SATELLITE_COVERAGE_PARTIAL;
    poll.reported_count = GTD_SOME_U32(12);
    GtdSatelliteObservation row = {GTD_CONSTELLATION_GPS, 1, GTD_NONE_F32, GTD_NONE_F32,
                                   GTD_SOME_F32(40)};
    if (gtd_ts_from_seconds(1700000002, &poll.sys_time) != GTD_OK ||
        gtd_builder_add_satellite_observation_report(builder, &poll, &row, 1) != GTD_OK) {
        goto fail;
    }
    GtdStatus status = gtd_builder_finish(builder, &file);
    builder = NULL;
    if (status != GTD_OK || gtd_nav_file_write_to_path(file, "navigation_polling.gtd") != GTD_OK) {
        goto fail;
    }
    gtd_nav_file_destroy(file);
    return 0;
fail:
    fprintf(stderr, "%s\n", gtd_last_error());
    gtd_builder_destroy(builder);
    gtd_nav_file_destroy(file);
    return 1;
}
