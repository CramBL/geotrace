/**
 * Open a .gtd file and print a summary of its contents.
 *
 * Pass a path on the command line to inspect an existing file:
 *
 *     ./read_file path/to/file.gtd
 *
 * With no argument the example first writes a small file to the working
 * directory and then reads that back, so it is runnable on its own.
 */

#include "../geotrace.h"

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

static void print_nav_points(const GtdNavFile *file) {
    size_t count = gtd_nav_file_nav_point_count(file);
    if (count == 0) {
        return;
    }

    printf("Nav points: %zu\n", count);
    for (size_t i = 0; i < count; i++) {
        GtdNavPointInfo point;
        if (gtd_nav_file_get_nav_point(file, i, &point) != GTD_OK) {
            continue;
        }

        printf("  [%zu] %.5f, %.5f", i, point.lat_deg, point.lon_deg);
        if (point.speed_mps.present) {
            printf("  %.1f m/s", point.speed_mps.value);
        }
        if (point.sat_count > 0) {
            printf("  sats=%zu", point.sat_count);
        }
        printf("\n");
    }
}

static void print_markers(const GtdNavFile *file) {
    size_t count = gtd_nav_file_marker_count(file);
    if (count == 0) {
        return;
    }

    printf("Markers: %zu\n", count);
    for (size_t i = 0; i < count; i++) {
        GtdMarkerInfo marker;
        if (gtd_nav_file_get_marker(file, i, &marker) != GTD_OK) {
            continue;
        }
        printf("  [%zu] %.5f, %.5f  icon=%u", i, marker.lat_deg, marker.lon_deg, marker.icon_code);
        if (marker.has_label) {
            printf(" - %s", marker.label);
        }
        printf("\n");
    }
}

static void print_event_markers(const GtdNavFile *file) {
    size_t count = gtd_nav_file_event_marker_count(file);
    if (count == 0) {
        return;
    }

    printf("Event markers: %zu\n", count);
    for (size_t i = 0; i < count; i++) {
        GtdEventMarkerInfo marker;
        if (gtd_nav_file_get_event_marker(file, i, &marker) != GTD_OK) {
            continue;
        }
        printf("  [%zu] %s", i, marker.variant_path);
        if (marker.has_annotation) {
            printf(" - %s", marker.annotation);
        }
        printf("\n");
    }
}

static void print_event_marker_styles(const GtdNavFile *file) {
    size_t count = gtd_nav_file_event_marker_style_count(file);
    if (count == 0) {
        return;
    }

    printf("Event marker styles: %zu\n", count);
    for (size_t i = 0; i < count; i++) {
        GtdEventMarkerStyleInfo style;
        if (gtd_nav_file_get_event_marker_style(file, i, &style) != GTD_OK) {
            continue;
        }
        const char *icon = style.icon_name[0] == '\0' ? "auto" : style.icon_name;
        const char *color = style.has_color ? style.color_hex : "auto";
        printf("  [%zu] %s  icon=%s  color=%s\n", i, style.variant_path, icon, color);
    }
}

static void print_channels(const GtdNavFile *file) {
    size_t count = gtd_nav_file_channel_count(file);
    if (count == 0) {
        return;
    }

    printf("Channels: %zu\n", count);
    for (size_t i = 0; i < count; i++) {
        GtdChannelInfo info;
        if (gtd_nav_file_get_channel(file, i, &info) != GTD_OK) {
            continue;
        }
        printf("  [%zu] %s %zu samples", i, info.name, info.sample_count);
        if (info.has_unit) {
            printf(" [%s]", info.unit);
        }
        if (info.component_count > 0) {
            printf(" components:");
            for (size_t c = 0; c < info.component_count; c++) {
                char label[32];
                if (gtd_nav_file_get_channel_component(file, i, c, label, sizeof label) == GTD_OK) {
                    printf(" %s", label);
                }
            }
        }
        printf("\n");
    }
}

static GtdStatus add_sample_fixes_and_satellite_report(GtdFileBuilder *builder) {
    const double track[][2] = {
        {51.5074, -0.1278},
        {51.5088, -0.1248},
        {51.5103, -0.1217},
    };

    for (size_t i = 0; i < sizeof track / sizeof track[0]; i++) {
        GtdTimestamp fix_time;
        GtdStatus status = gtd_ts_from_seconds(BASE_EPOCH + ((int64_t)i * 30), &fix_time);
        if (status != GTD_OK) {
            return status;
        }

        status = gtd_builder_add_nav_fix(builder, fix_time, gtd_ts_none(), track[i][0], track[i][1],
                                         GTD_SOME_F64(90.0), GTD_SOME_F64(5.5), GTD_NONE_F64);
        if (status != GTD_OK) {
            return status;
        }
    }

    GtdTimestamp report_time;
    GtdStatus status = gtd_ts_from_seconds(BASE_EPOCH, &report_time);
    if (status != GTD_OK) {
        return status;
    }

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 1, 1, GTD_SOME_F32(45.0F), GTD_SOME_F32(90.0F),
         GTD_SOME_F32(38.0F)},
        {GTD_CONSTELLATION_GALILEO, 3, 0, GTD_NONE_F32, GTD_NONE_F32, GTD_SOME_F32(22.0F)},
    };
    return gtd_builder_add_satellite_report(builder, report_time, gtd_ts_none(), sats,
                                            sizeof sats / sizeof sats[0]);
}

static GtdStatus add_sample_annotation_event_marker_and_style(GtdFileBuilder *builder) {
    GtdTimestamp annotation_time;
    GtdStatus status = gtd_ts_from_seconds(BASE_EPOCH + 10, &annotation_time);
    if (status != GTD_OK) {
        return status;
    }
    status = gtd_builder_add_annotation(builder, annotation_time, "Start point", GTD_ICON_PIN);
    if (status != GTD_OK) {
        return status;
    }

    GtdTimestamp event_time;
    status = gtd_ts_from_seconds(BASE_EPOCH + 2, &event_time);
    if (status != GTD_OK) {
        return status;
    }
    status = gtd_builder_add_event_marker(builder, "power/boot", event_time, "cold start");
    if (status != GTD_OK) {
        return status;
    }

    return gtd_builder_add_event_marker_style(builder, "power/boot", GTD_ICON_LIGHTNING, "#44BB44");
}

static GtdStatus add_sample_channel(GtdFileBuilder *builder) {
    GtdTimestamp times[3];
    double incline_vals[3];
    for (size_t i = 0; i < 3; i++) {
        GtdStatus status = gtd_ts_from_seconds(BASE_EPOCH + ((int64_t)i * 30), &times[i]);
        if (status != GTD_OK) {
            return status;
        }
        incline_vals[i] = 1.0 + ((double)i * 0.5);
    }

    GtdChannel incline = {0};
    incline.name = "incline";
    incline.unit = "deg";
    incline.period_deg = GTD_NONE_F64;
    incline.times = times;
    incline.n_times = 3;
    incline.values = incline_vals;
    incline.n_values = 3;
    return gtd_builder_add_channel(builder, &incline);
}

/* Write a sample file holding one of every section this example prints. */
static GtdStatus write_sample_file(const char *path) {
    GtdFileBuilder *builder = gtd_builder_create();
    gtd_builder_set_title(builder, "Sample track");
    gtd_builder_set_device(builder, "Example GPS v1.0");

    GtdStatus status = add_sample_fixes_and_satellite_report(builder);
    if (status == GTD_OK) {
        status = add_sample_annotation_event_marker_and_style(builder);
    }
    if (status == GTD_OK) {
        status = add_sample_channel(builder);
    }
    if (status != GTD_OK) {
        fprintf(stderr, "write_sample_file: %s\n", gtd_last_error());
        gtd_builder_destroy(builder);
        return status;
    }

    GtdNavFile *file = NULL;
    status = gtd_builder_finish(builder, &file);
    if (status != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return status;
    }

    status = gtd_nav_file_write_to_path(file, path);
    gtd_nav_file_destroy(file);
    if (status != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
    }
    return status;
}

int main(int argc, char **argv) {
    const char *sample_path = "geotrace_read_file_sample.gtd";
    const char *path = argc >= 2 ? argv[1] : sample_path;

    if (argc < 2 && write_sample_file(sample_path) != GTD_OK) {
        return 1;
    }

    GtdNavFile *file = NULL;
    GtdStatus status = gtd_nav_file_open(path, &file);
    if (status != GTD_OK) {
        fprintf(stderr, "open: %s\n", gtd_last_error());
        return 1;
    }

    const char *title = gtd_nav_file_title(file);
    if (title) {
        printf("Title:  %s\n", title);
    }

    const char *device = gtd_nav_file_device(file);
    if (device) {
        printf("Device: %s\n", device);
    }

    print_nav_points(file);
    print_markers(file);
    print_event_markers(file);
    print_event_marker_styles(file);
    print_channels(file);

    gtd_nav_file_destroy(file);

    if (argc < 2 && remove(sample_path) != 0) {
        fprintf(stderr, "remove %s: %s\n", sample_path, strerror(errno));
    }
    return 0;
}
