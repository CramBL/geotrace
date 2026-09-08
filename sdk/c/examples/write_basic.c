/**
 * Write a basic .gtd file from a hardcoded GPS track.
 *
 * The minimal write workflow: create a builder, set some metadata, add a few
 * nav fixes (plus an optional satellite report and a map annotation), call
 * finish(), and write the result to disk.
 */

#include "../geotrace.h"

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();

    gtd_builder_set_title(builder, "Quick tour");
    gtd_builder_set_device(builder, "Example GPS v1.0");

    GtdStatus status;

    GtdTimestamp first_fix_time;
    status = gtd_ts_from_seconds(BASE_EPOCH, &first_fix_time);
    if (status != GTD_OK) {
        fprintf(stderr, "ts_from_seconds: %s\n", gtd_last_error());
        goto fail;
    }

    status = gtd_builder_add_nav_fix(builder, first_fix_time, gtd_ts_none(), 51.5074, -0.1278,
                                     GTD_SOME_F64(90.0), GTD_SOME_F64(5.5), GTD_SOME_F64(3.2));
    if (status != GTD_OK) {
        fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
        goto fail;
    }

    /* Elevation and azimuth are optional. A receiver reports an SNR for a
       satellite whose position it has not computed. */
    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 1, 1, GTD_SOME_F32(45.0F), GTD_SOME_F32(90.0F),
         GTD_SOME_F32(38.0F)},
        {GTD_CONSTELLATION_GALILEO, 3, 0, GTD_NONE_F32, GTD_NONE_F32, GTD_SOME_F32(22.0F)},
    };
    status = gtd_builder_add_satellite_report(builder, first_fix_time, gtd_ts_none(), sats,
                                              sizeof sats / sizeof sats[0]);
    if (status != GTD_OK) {
        fprintf(stderr, "add_satellite_report: %s\n", gtd_last_error());
        goto fail;
    }

    GtdTimestamp second_fix_time;
    status = gtd_ts_from_seconds(BASE_EPOCH + 10, &second_fix_time);
    if (status != GTD_OK) {
        fprintf(stderr, "ts_from_seconds: %s\n", gtd_last_error());
        goto fail;
    }

    status = gtd_builder_add_nav_fix(builder, second_fix_time, gtd_ts_none(), 51.5080, -0.1265,
                                     GTD_SOME_F64(85.0), GTD_SOME_F64(5.8), GTD_NONE_F64);
    if (status != GTD_OK) {
        fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
        goto fail;
    }

    status = gtd_builder_add_annotation(builder, first_fix_time, "Start point", GTD_ICON_PIN);
    if (status != GTD_OK) {
        fprintf(stderr, "add_annotation: %s\n", gtd_last_error());
        goto fail;
    }

    GtdNavFile *file = NULL;
    status = gtd_builder_finish(builder, &file);
    builder = NULL;
    if (status != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    const char *path = "geotrace_write_basic.gtd";
    status = gtd_nav_file_write_to_path(file, path);
    if (status != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
        gtd_nav_file_destroy(file);
        return 1;
    }

    printf("Wrote %zu nav points to %s\n", gtd_nav_file_nav_point_count(file), path);

    gtd_nav_file_destroy(file);
    if (remove(path) != 0) {
        fprintf(stderr, "remove %s: %s\n", path, strerror(errno));
    }
    return 0;

fail:
    gtd_builder_destroy(builder);
    return 1;
}
