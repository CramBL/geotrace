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

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

/* A short London track, one fix every 30 s. */
static GtdStatus add_track_fixes(GtdFileBuilder *builder) {
    const double track[][2] = {
        {51.5074, -0.1278}, {51.5080, -0.1265}, {51.5088, -0.1248},
        {51.5095, -0.1233}, {51.5103, -0.1217}, {51.5110, -0.1200},
    };
    for (size_t i = 0; i < sizeof track / sizeof track[0]; i++) {
        GtdTimestamp fix_time;
        GtdStatus status = gtd_ts_from_seconds(BASE_EPOCH + ((int64_t)i * 30), &fix_time);
        if (status != GTD_OK) {
            fprintf(stderr, "ts_from_seconds: %s\n", gtd_last_error());
            return status;
        }

        status = gtd_builder_add_nav_fix(builder, fix_time, gtd_ts_none(), track[i][0], track[i][1],
                                         GTD_SOME_F64(90.0), GTD_NONE_F64, GTD_NONE_F64);
        if (status != GTD_OK) {
            fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
            return status;
        }
    }
    return GTD_OK;
}

/* (`variant_path`, second-offset, annotation) - flat and nested paths. */
static GtdStatus add_event_markers(GtdFileBuilder *builder) {
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
        GtdTimestamp event_time;
        GtdStatus status = gtd_ts_from_seconds(BASE_EPOCH + events[i].offset, &event_time);
        if (status != GTD_OK) {
            fprintf(stderr, "ts_from_seconds: %s\n", gtd_last_error());
            return status;
        }

        status = gtd_builder_add_event_marker(builder, events[i].path, event_time, events[i].note);
        if (status != GTD_OK) {
            fprintf(stderr, "add_event_marker(%s): %s\n", events[i].path, gtd_last_error());
            return status;
        }
    }
    return GTD_OK;
}

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();

    gtd_builder_set_title(builder, "Event marker tour");
    gtd_builder_set_device(builder, "Example GPS v1.0");

    GtdStatus status = add_track_fixes(builder);
    if (status != GTD_OK) {
        goto fail;
    }

    status = add_event_markers(builder);
    if (status != GTD_OK) {
        goto fail;
    }

    status =
        gtd_builder_add_event_marker_style(builder, "power/boot", GTD_ICON_LIGHTNING, "#44BB44");
    if (status != GTD_OK) {
        fprintf(stderr, "add_event_marker_style: %s\n", gtd_last_error());
        goto fail;
    }
    status = gtd_builder_add_event_marker_style(builder, "power/sleep", GTD_ICON_PIN, "#4488FF");
    if (status != GTD_OK) {
        fprintf(stderr, "add_event_marker_style: %s\n", gtd_last_error());
        goto fail;
    }

    GtdNavFile *file = NULL;
    status = gtd_builder_finish(builder, &file);
    builder = NULL;
    if (status != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    const char *path = "geotrace_event_markers.gtd";
    status = gtd_nav_file_write_to_path(file, path);
    if (status != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
        gtd_nav_file_destroy(file);
        return 1;
    }
    gtd_nav_file_destroy(file);

    /* Read it back and print the markers, as GeoTrace would list them. */
    GtdNavFile *loaded = NULL;
    status = gtd_nav_file_open(path, &loaded);
    if (status != GTD_OK) {
        fprintf(stderr, "open: %s\n", gtd_last_error());
        return 1;
    }

    size_t marker_count = gtd_nav_file_event_marker_count(loaded);
    printf("Event markers: %zu\n", marker_count);
    for (size_t i = 0; i < marker_count; i++) {
        GtdEventMarkerInfo marker;
        if (gtd_nav_file_get_event_marker(loaded, i, &marker) != GTD_OK) {
            continue;
        }
        printf("  %-28s %.5f, %.5f", marker.variant_path, marker.lat_deg, marker.lon_deg);
        if (marker.has_annotation) {
            printf("  - %s", marker.annotation);
        }
        printf("\n");
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
