/**
 * Aggregate data from multiple sources into a single .gtd GeoTrace data file.
 *
 * Scenario: your GPS unit logs fixes to one source, and a separate system (a
 * test harness, an annotation tool, a sensor log) records named events with
 * their own timestamps.  Both are added independently to the builder. finish()
 * sorts everything by time and interpolates each annotation's map position
 * from the two surrounding GPS fixes.
 */

#include "../geotrace.h"

#include <stdint.h>
#include <stdio.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

int main(void) {
    GtdFileBuilder *b = gtd_builder_create();

    gtd_builder_set_title(b, "Merged GPS + annotations");
    gtd_builder_set_device(b, "Aggregator v1.0");

    GtdStatus s;

    /* Source 1: GPS track (lat, lon, heading), one fix every 10 s. */
    const double gps[][3] = {
        {51.5074, -0.1278, 90.0}, {51.5075, -0.1276, 91.0}, {51.5076, -0.1274, 89.5},
        {51.5077, -0.1272, 88.0}, {51.5078, -0.1270, 90.0}, {51.5079, -0.1268, 90.5},
    };
    for (size_t i = 0; i < sizeof gps / sizeof gps[0]; i++) {
        s = gtd_builder_add_nav_fix(b, gtd_ts_from_seconds(BASE_EPOCH + (i * 10U)), gtd_ts_none(),
                                    gps[i][0], gps[i][1], GTD_SOME_F64(gps[i][2]), GTD_NONE_F64,
                                    GTD_NONE_F64);
        if (s != GTD_OK) {
            fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
            goto fail;
        }
    }

    /* Source 2: annotations from a separate log. Their map positions are not
       supplied - finish() interpolates them from the GPS fixes by timestamp. */
    struct {
        uint32_t offset;
        const char *label;
        GtdMarkerIcon icon;
    } annotations[] = {
        {5, "Pothole", GTD_ICON_WARNING},
        {15, "Speed camera", GTD_ICON_CIRCLE},
        {25, "Junction", GTD_ICON_PIN},
    };
    for (size_t i = 0; i < sizeof annotations / sizeof annotations[0]; i++) {
        s = gtd_builder_add_annotation(b, gtd_ts_from_seconds(BASE_EPOCH + annotations[i].offset),
                                       annotations[i].label, annotations[i].icon);
        if (s != GTD_OK) {
            fprintf(stderr, "add_annotation(%s): %s\n", annotations[i].label, gtd_last_error());
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

    const char *path = "geotrace_from_multiple_sources.gtd";
    s = gtd_nav_file_write_to_path(f, path);
    if (s != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
        gtd_nav_file_destroy(f);
        return 1;
    }

    printf("Merged %zu GPS fixes + 3 annotations -> %s\n", gtd_nav_file_nav_point_count(f), path);
    printf("Annotations were interpolated onto the track by timestamp.\n");

    gtd_nav_file_destroy(f);
    remove(path);
    return 0;

fail:
    gtd_builder_destroy(b);
    return 1;
}
