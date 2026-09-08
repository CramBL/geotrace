/**
 * Write a .gtd file that pairs each GPS fix with a satellite visibility report.
 *
 * A satellite report is a snapshot of every tracked satellite at one instant:
 * its constellation, PRN, whether it contributed to the fix, and signal
 * quality (elevation, azimuth, SNR).  Reports are matched to the nearest fix,
 * so giving each report the same timestamp as its fix keeps them aligned.
 *
 * The example writes the file, reads it back, and prints the per-fix satellite
 * counts - the data GeoTrace shows in its sky view.
 */

#include "../geotrace.h"

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

#define SKY_SIZE 8

int main(void) {
    /* A short urban loop through Southwark, London, one fix every 10 s. */
    const struct {
        uint32_t offset_s;
        double lat;
        double lon;
        double heading_deg;
        double speed_mps;
        double eph_m;
    } track[] = {
        {0, 51.5030, -0.0978, 5.0, 0.0, 4.2},   {10, 51.5038, -0.0975, 8.0, 3.1, 3.8},
        {20, 51.5045, -0.0971, 12.0, 4.4, 3.5}, {30, 51.5053, -0.0966, 10.0, 4.6, 3.1},
        {40, 51.5060, -0.0961, 7.0, 4.4, 2.9},  {50, 51.5067, -0.0957, 5.0, 3.8, 3.0},
    };

    /* A mixed GPS, Galileo and GLONASS sky: eight satellites, five in the fix.
       GLONASS 5 has an SNR and no elevation or azimuth. A receiver reports that
       for a satellite whose position it has not computed. */
    const GtdSatellite sky[SKY_SIZE] = {
        {GTD_CONSTELLATION_GPS, 3, 1, GTD_SOME_F32(72.0F), GTD_SOME_F32(145.0F),
         GTD_SOME_F32(44.0F)},
        {GTD_CONSTELLATION_GPS, 8, 1, GTD_SOME_F32(58.0F), GTD_SOME_F32(230.0F),
         GTD_SOME_F32(41.0F)},
        {GTD_CONSTELLATION_GPS, 14, 1, GTD_SOME_F32(41.0F), GTD_SOME_F32(60.0F),
         GTD_SOME_F32(37.0F)},
        {GTD_CONSTELLATION_GPS, 22, 0, GTD_SOME_F32(18.0F), GTD_SOME_F32(310.0F),
         GTD_SOME_F32(28.0F)},
        {GTD_CONSTELLATION_GALILEO, 7, 1, GTD_SOME_F32(65.0F), GTD_SOME_F32(195.0F),
         GTD_SOME_F32(42.0F)},
        {GTD_CONSTELLATION_GALILEO, 12, 1, GTD_SOME_F32(33.0F), GTD_SOME_F32(90.0F),
         GTD_SOME_F32(35.0F)},
        {GTD_CONSTELLATION_GALILEO, 19, 0, GTD_SOME_F32(12.0F), GTD_SOME_F32(15.0F),
         GTD_SOME_F32(22.0F)},
        {GTD_CONSTELLATION_GLONASS, 5, 0, GTD_NONE_F32, GTD_NONE_F32, GTD_SOME_F32(31.0F)},
    };

    GtdFileBuilder *builder = gtd_builder_create();

    gtd_builder_set_title(builder, "Satellite quality tour");
    gtd_builder_set_device(builder, "Example GNSS v1.0");

    GtdStatus status;
    for (size_t i = 0; i < sizeof track / sizeof track[0]; i++) {
        GtdTimestamp fix_time;
        status = gtd_ts_from_seconds(BASE_EPOCH + track[i].offset_s, &fix_time);
        if (status != GTD_OK) {
            fprintf(stderr, "ts_from_seconds: %s\n", gtd_last_error());
            goto fail;
        }

        status =
            gtd_builder_add_nav_fix(builder, fix_time, gtd_ts_none(), track[i].lat, track[i].lon,
                                    GTD_SOME_F64(track[i].heading_deg),
                                    GTD_SOME_F64(track[i].speed_mps), GTD_SOME_F64(track[i].eph_m));
        if (status != GTD_OK) {
            fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
            goto fail;
        }

        /* SNR climbs slightly along the track as the receiver settles. */
        float snr_gain = 0.5F * (float)i;
        GtdSatellite sats[SKY_SIZE];
        for (size_t j = 0; j < SKY_SIZE; j++) {
            sats[j] = sky[j];
            sats[j].snr_dbhz.value += snr_gain;
        }

        status = gtd_builder_add_satellite_report(builder, fix_time, gtd_ts_none(), sats, SKY_SIZE);
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
