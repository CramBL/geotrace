/**
 * Write a .gtd file that pairs each GPS fix with a satellite visibility report.
 *
 * A satellite report is a snapshot of every tracked satellite at one instant:
 * its constellation, PRN, whether it contributed to the fix, and signal
 * quality (elevation, azimuth, SNR).  Reports are matched to the nearest fix,
 * so giving each report the same timestamp as its fix keeps them aligned.
 *
 * The example writes the file, reads it back, and prints the per-fix satellite
 * counts and signal strengths - the data GeoTrace shows in its sky view.
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

    gtd_builder_set_title(builder, "Satellite quality tour");
    gtd_builder_set_device(builder, "Example GNSS v1.0");

    const double track[][2] = {
        {51.5074, -0.1278},
        {51.5080, -0.1265},
        {51.5088, -0.1248},
        {51.5095, -0.1233},
    };

    GtdStatus status;
    for (size_t i = 0; i < sizeof track / sizeof track[0]; i++) {
        GtdTimestamp fix_time = gtd_ts_from_seconds(BASE_EPOCH + i);

        status = gtd_builder_add_nav_fix(builder, fix_time, gtd_ts_none(), track[i][0], track[i][1],
                                         GTD_SOME_F64(90.0), GTD_SOME_F64(5.5), GTD_SOME_F64(3.0));
        if (status != GTD_OK) {
            fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
            goto fail;
        }

        /* SNR climbs slightly each second as the receiver settles. */
        float snr = 36.0F + (float)i;
        GtdSatellite sats[] = {
            {GTD_CONSTELLATION_GPS, 1, 1, GTD_SOME_F32(45.0F), GTD_SOME_F32(90.0F),
             GTD_SOME_F32(snr)},
            {GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F32(30.0F), GTD_SOME_F32(180.0F),
             GTD_SOME_F32(snr - 2.0F)},
            {GTD_CONSTELLATION_GALILEO, 3, 0, GTD_NONE_F32, GTD_NONE_F32, GTD_SOME_F32(21.0F)},
        };
        status = gtd_builder_add_satellite_report(builder, fix_time, gtd_ts_none(), sats,
                                                  sizeof sats / sizeof sats[0]);
        if (status != GTD_OK) {
            fprintf(stderr, "add_satellite_report: %s\n", gtd_last_error());
            goto fail;
        }
    }

    GtdNavFile *file = NULL;
    status = gtd_builder_finish(builder, &file);
    builder = NULL;
    if (status != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    const char *path = "geotrace_with_satellites.gtd";
    status = gtd_nav_file_write_to_path(file, path);
    gtd_nav_file_destroy(file);
    if (status != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
        return 1;
    }

    GtdNavFile *loaded = NULL;
    status = gtd_nav_file_open(path, &loaded);
    if (status != GTD_OK) {
        fprintf(stderr, "open: %s\n", gtd_last_error());
        return 1;
    }

    size_t nav_point_count = gtd_nav_file_nav_point_count(loaded);
    printf("Nav points: %zu\n", nav_point_count);
    for (size_t i = 0; i < nav_point_count; i++) {
        GtdNavPointInfo point;
        if (gtd_nav_file_get_nav_point(loaded, i, &point) != GTD_OK) {
            continue;
        }

        size_t in_fix = 0;
        for (size_t j = 0; j < point.sat_count; j++) {
            GtdSatInfo sat;
            if (gtd_nav_file_get_satellite(loaded, i, j, &sat) == GTD_OK && sat.in_fix) {
                in_fix++;
            }
        }
        printf("  [%zu] %zu tracked, %zu in fix\n", i, point.sat_count, in_fix);
    }

    gtd_nav_file_destroy(loaded);
    if (remove(path) != 0) {
        fprintf(stderr, "remove %s: %s\n", path, strerror(errno));
    }
    return 0;

fail:
    gtd_builder_destroy(builder);
    return 1;
}
