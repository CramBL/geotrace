/**
 * Write and read back a .gtd file containing event markers.
 *
 * Event markers are timed, hierarchical events anchored to the GPS track.
 * Each marker carries a slash-separated variant path (e.g. "power/boot" or
 * "connectivity/agps/request") that GeoTrace uses to group and filter events
 * in the Events panel.  Per-variant styles set an icon and color. Unlisted
 * variants get a deterministic fallback color derived from their path.
 */

#include "../geotrace.h"

#include <stdint.h>
#include <stdio.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

int main(void) {
    GtdFileBuilder *b = gtd_builder_create();

    gtd_builder_set_title(b, "Event marker tour");
    gtd_builder_set_device(b, "Example GPS v1.0");

    GtdStatus s;

    /* A short London track, one fix every 30 s. */
    const double track[][2] = {
        {51.5074, -0.1278}, {51.5080, -0.1265}, {51.5088, -0.1248},
        {51.5095, -0.1233}, {51.5103, -0.1217}, {51.5110, -0.1200},
    };
    for (size_t i = 0; i < sizeof track / sizeof track[0]; i++) {
        GtdTimestamp t = gtd_ts_from_seconds(BASE_EPOCH + (i * 30U));
        s = gtd_builder_add_nav_fix(b, t, gtd_ts_none(), track[i][0], track[i][1],
                                    GTD_SOME_F64(90.0), GTD_NONE_F64, GTD_NONE_F64);
        if (s != GTD_OK) {
            fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
            goto fail;
        }
    }

    /* (variant_path, second-offset, annotation) - flat and nested paths. */
    struct {
        const char *path;
        uint32_t offset;
        const char *note;
    } events[] = {
        {"power/boot", 2, "cold start"},
        {"connectivity/agps/request", 5, "EPO fetch started"},
        {"connectivity/agps/success", 18, "EPO applied, TTFF reduced"},
        {"sensor/gps/lock_acquired", 20, NULL},
        {"power/sleep", 145, NULL},
    };
    for (size_t i = 0; i < sizeof events / sizeof events[0]; i++) {
        GtdTimestamp t = gtd_ts_from_seconds(BASE_EPOCH + events[i].offset);
        s = gtd_builder_add_event_marker(b, events[i].path, t, events[i].note);
        if (s != GTD_OK) {
            fprintf(stderr, "add_event_marker(%s): %s\n", events[i].path, gtd_last_error());
            goto fail;
        }
    }

    s = gtd_builder_add_event_marker_style(b, "power/boot", GTD_ICON_LIGHTNING, "#44BB44");
    if (s != GTD_OK) {
        fprintf(stderr, "add_event_marker_style: %s\n", gtd_last_error());
        goto fail;
    }
    s = gtd_builder_add_event_marker_style(b, "power/sleep", GTD_ICON_PIN, "#4488FF");
    if (s != GTD_OK) {
        fprintf(stderr, "add_event_marker_style: %s\n", gtd_last_error());
        goto fail;
    }

    GtdNavFile *f = NULL;
    s = gtd_builder_finish(b, &f);
    b = NULL;
    if (s != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    const char *path = "geotrace_event_markers.gtd";
    s = gtd_nav_file_write_to_path(f, path);
    if (s != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
        gtd_nav_file_destroy(f);
        return 1;
    }
    gtd_nav_file_destroy(f);

    /* Read it back and print the markers, as GeoTrace would list them. */
    GtdNavFile *loaded = NULL;
    s = gtd_nav_file_open(path, &loaded);
    if (s != GTD_OK) {
        fprintf(stderr, "open: %s\n", gtd_last_error());
        return 1;
    }

    size_t n = gtd_nav_file_event_marker_count(loaded);
    printf("Event markers: %zu\n", n);
    for (size_t i = 0; i < n; i++) {
        GtdEventMarkerInfo m;
        if (gtd_nav_file_get_event_marker(loaded, i, &m) != GTD_OK)
            continue;
        printf("  %-28s %.5f, %.5f", m.variant_path, m.lat_deg, m.lon_deg);
        if (m.has_annotation)
            printf("  - %s", m.annotation);
        printf("\n");
    }

    gtd_nav_file_destroy(loaded);
    remove(path);
    return 0;

fail:
    gtd_builder_destroy(b);
    return 1;
}
