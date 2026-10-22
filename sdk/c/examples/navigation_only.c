#include "../geotrace.h"
#include <stdio.h>

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdNavFile *file = NULL;
    GtdNavFixV2 fix = gtd_nav_fix_v2_init();
    fix.lat_deg = 55.67;
    fix.lon_deg = 12.56;
    fix.state = GTD_SOLUTION_STATE_FIX;
    fix.used_count = GTD_SOME_U32(8);
    fix.hdop = GTD_SOME_F64(0.9);
    fix.heading_deg = GTD_SOME_F64(75);
    fix.speed_mps = GTD_SOME_F64(4.2);
    if (gtd_builder_set_title(builder, "Basic navigation recording") != GTD_OK ||
        gtd_ts_from_seconds(1792670400, &fix.gps_time) != GTD_OK ||
        gtd_builder_add_nav_fix_v2(builder, &fix) != GTD_OK) {
        goto fail;
    }
    fix = gtd_nav_fix_v2_status_only_init(GTD_SOLUTION_STATE_NO_FIX);
    if (gtd_ts_from_seconds(1792670402, &fix.gps_time) != GTD_OK ||
        gtd_builder_add_nav_fix_v2(builder, &fix) != GTD_OK) {
        goto fail;
    }
    fix = gtd_nav_fix_v2_init();
    fix.lat_deg = 55.671;
    fix.lon_deg = 12.562;
    fix.state = GTD_SOLUTION_STATE_FIX;
    fix.used_count = GTD_SOME_U32(9);
    if (gtd_ts_from_seconds(1792670405, &fix.gps_time) != GTD_OK ||
        gtd_builder_add_nav_fix_v2(builder, &fix) != GTD_OK) {
        goto fail;
    }
    GtdStatus status = gtd_builder_finish(builder, &file);
    builder = NULL;
    if (status != GTD_OK || gtd_nav_file_write_to_path(file, "navigation_only.gtd") != GTD_OK) {
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
