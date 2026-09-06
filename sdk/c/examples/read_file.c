#include "../geotrace.h"

#include <stdio.h>

static void print_nav_points(const GtdNavFile *file) {
    size_t count = gtd_nav_file_nav_point_count(file);
    printf("Nav points: %zu\n", count);

    for (size_t i = 0; i < count; i++) {
        GtdNavPointInfo point;
        if (gtd_nav_file_get_nav_point(file, i, &point) != GTD_OK) {
            continue;
        }

        printf("  [%zu] lat=%.6f lon=%.6f", i, point.lat_deg, point.lon_deg);

        if (point.speed_mps.present) {
            printf(" speed=%.2f m/s", point.speed_mps.value);
        }

        if (point.sat_count > 0) {
            printf(" sats=%zu", point.sat_count);
            for (size_t j = 0; j < point.sat_count; j++) {
                GtdSatInfo sat;
                if (gtd_nav_file_get_satellite(file, i, j, &sat) == GTD_OK && sat.in_fix) {
                    printf(" (prn=%u in_fix)", sat.prn);
                }
            }
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
        printf("  [%zu] lat=%.6f lon=%.6f icon=%u", i, marker.lat_deg, marker.lon_deg,
               marker.icon_code);
        if (marker.has_label) {
            printf(" - %s", marker.label);
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
        printf("  [%zu] %s icon=%s color=%s\n", i, style.variant_path, icon, color);
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

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <file.gtd>\n", argv[0]);
        return 1;
    }

    GtdNavFile *file = NULL;
    GtdStatus status = gtd_nav_file_open(argv[1], &file);
    if (status != GTD_OK) {
        fprintf(stderr, "open: %s\n", gtd_last_error());
        return 1;
    }

    const char *title = gtd_nav_file_title(file);
    if (title) {
        printf("Title: %s\n", title);
    }

    const char *device = gtd_nav_file_device(file);
    if (device) {
        printf("Device: %s\n", device);
    }

    print_nav_points(file);
    print_markers(file);
    print_event_markers(file);
    print_event_marker_styles(file);

    gtd_nav_file_destroy(file);
    return 0;
}
