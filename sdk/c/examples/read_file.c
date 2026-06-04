#include "../geotrace.h"

#include <stdio.h>

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <file.gtd>\n", argv[0]);
        return 1;
    }

    GtdNavFile *f = NULL;
    GtdStatus s = gtd_nav_file_open(argv[1], &f);
    if (s != GTD_OK) {
        fprintf(stderr, "open: %s\n", gtd_last_error());
        return 1;
    }

    const char *title = gtd_nav_file_title(f);
    if (title) printf("Title: %s\n", title);

    const char *device = gtd_nav_file_device(f);
    if (device) printf("Device: %s\n", device);

    size_t n = gtd_nav_file_nav_point_count(f);
    printf("Nav points: %zu\n", n);

    for (size_t i = 0; i < n; i++) {
        GtdNavPointInfo p;
        if (gtd_nav_file_get_nav_point(f, i, &p) != GTD_OK) continue;

        printf("  [%zu] lat=%.6f lon=%.6f", i, p.lat_deg, p.lon_deg);

        if (p.speed_mps.present)
            printf(" speed=%.2f m/s", p.speed_mps.value);

        if (p.sat_count > 0) {
            printf(" sats=%zu", p.sat_count);
            for (size_t j = 0; j < p.sat_count; j++) {
                GtdSatInfo sat;
                if (gtd_nav_file_get_satellite(f, i, j, &sat) == GTD_OK && sat.in_fix)
                    printf(" (prn=%u in_fix)", sat.prn);
            }
        }

        printf("\n");
    }

    size_t markers = gtd_nav_file_event_marker_count(f);
    if (markers > 0) {
        printf("Event markers: %zu\n", markers);
        for (size_t i = 0; i < markers; i++) {
            GtdEventMarkerInfo m;
            if (gtd_nav_file_get_event_marker(f, i, &m) != GTD_OK) continue;
            printf("  [%zu] %s", i, m.variant_path);
            if (m.has_annotation) printf(" - %s", m.annotation);
            printf("\n");
        }
    }

    gtd_nav_file_destroy(f);
    return 0;
}
