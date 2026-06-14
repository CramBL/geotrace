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

#include <stdint.h>
#include <stdio.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

int main(void) {
    GtdFileBuilder *b = gtd_builder_create();

    gtd_builder_set_title(b, "Satellite quality tour");
    gtd_builder_set_device(b, "Example GNSS v1.0");

    const double track[][2] = {
        {51.5074, -0.1278},
        {51.5080, -0.1265},
        {51.5088, -0.1248},
        {51.5095, -0.1233},
    };

    GtdStatus s;
    for (size_t i = 0; i < sizeof track / sizeof track[0]; i++) {
        GtdTimestamp t = gtd_ts_from_seconds(BASE_EPOCH + i);

        s = gtd_builder_add_nav_fix(b, t, gtd_ts_none(), track[i][0], track[i][1],
                                    GTD_SOME_F64(90.0), GTD_SOME_F64(5.5), GTD_SOME_F64(3.0));
        if (s != GTD_OK) {
            fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
            goto fail;
        }

        /* SNR climbs slightly each second as the receiver settles. */
        double snr = 36.0 + (double)i;
        GtdSatellite sats[] = {
            {GTD_CONSTELLATION_GPS, 1, 1, GTD_SOME_F64(45.0), GTD_SOME_F64(90.0),
             GTD_SOME_F64(snr)},
            {GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F64(30.0), GTD_SOME_F64(180.0),
             GTD_SOME_F64(snr - 2.0)},
            {GTD_CONSTELLATION_GALILEO, 3, 0, GTD_NONE_F64, GTD_NONE_F64, GTD_SOME_F64(21.0)},
        };
        s = gtd_builder_add_satellite_report(b, t, gtd_ts_none(), sats,
                                             sizeof sats / sizeof sats[0]);
        if (s != GTD_OK) {
            fprintf(stderr, "add_satellite_report: %s\n", gtd_last_error());
            goto fail;
        }
    }

    GtdNavFile *f = NULL;
    s = gtd_builder_finish(b, &f);
    b = NULL;
    if (s != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    const char *path = "geotrace_with_satellites.gtd";
    s = gtd_nav_file_write_to_path(f, path);
    gtd_nav_file_destroy(f);
    if (s != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
        return 1;
    }

    GtdNavFile *loaded = NULL;
    s = gtd_nav_file_open(path, &loaded);
    if (s != GTD_OK) {
        fprintf(stderr, "open: %s\n", gtd_last_error());
        return 1;
    }

    size_t n = gtd_nav_file_nav_point_count(loaded);
    printf("Nav points: %zu\n", n);
    for (size_t i = 0; i < n; i++) {
        GtdNavPointInfo p;
        if (gtd_nav_file_get_nav_point(loaded, i, &p) != GTD_OK)
            continue;

        size_t in_fix = 0;
        for (size_t j = 0; j < p.sat_count; j++) {
            GtdSatInfo sat;
            if (gtd_nav_file_get_satellite(loaded, i, j, &sat) == GTD_OK && sat.in_fix)
                in_fix++;
        }
        printf("  [%zu] %zu tracked, %zu in fix\n", i, p.sat_count, in_fix);
    }

    gtd_nav_file_destroy(loaded);
    remove(path);
    return 0;

fail:
    gtd_builder_destroy(b);
    return 1;
}
