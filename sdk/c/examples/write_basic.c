#include "../geotrace.h"

#include <stdio.h>
#include <time.h>

int main(void) {
    GtdFileBuilder *b = gtd_builder_create();

    gtd_builder_set_title (b, "Quick tour");
    gtd_builder_set_device(b, "Example GPS v1.0");

    GtdTimestamp t0 = gtd_ts_from_seconds((uint64_t)time(NULL));

    GtdSatellite sats[] = {
        { GTD_CONSTELLATION_GPS, 1, 1, GTD_SOME_F64(45.0), GTD_SOME_F64(90.0),  GTD_SOME_F64(38.0) },
        { GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F64(30.0), GTD_SOME_F64(180.0), GTD_SOME_F64(35.5) },
        { GTD_CONSTELLATION_GALILEO, 3, 0, GTD_NONE_F64, GTD_NONE_F64, GTD_SOME_F64(22.0) },
    };

    GtdStatus s;

    s = gtd_builder_add_nav_fix(b,
        t0, gtd_ts_none(),
        51.5074, -0.1278,
        GTD_SOME_F64(90.0),
        GTD_SOME_F64(5.5),
        GTD_SOME_F64(3.2));
    if (s != GTD_OK) { fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error()); goto fail; }

    s = gtd_builder_add_satellite_report(b,
        t0, gtd_ts_none(),
        sats, sizeof sats / sizeof sats[0]);
    if (s != GTD_OK) { fprintf(stderr, "add_satellite_report: %s\n", gtd_last_error()); goto fail; }

    s = gtd_builder_add_nav_fix(b,
        gtd_ts_from_seconds((uint64_t)time(NULL) + 10), gtd_ts_none(),
        51.5080, -0.1265,
        GTD_SOME_F64(85.0),
        GTD_SOME_F64(5.8),
        GTD_SOME_F64(2.9));
    if (s != GTD_OK) { fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error()); goto fail; }

    s = gtd_builder_add_annotation(b, t0, "Start point", GTD_ICON_PIN);
    if (s != GTD_OK) { fprintf(stderr, "add_annotation: %s\n", gtd_last_error()); goto fail; }

    GtdNavFile *f = NULL;
    s = gtd_builder_finish(b, &f);
    b = NULL;
    if (s != GTD_OK) { fprintf(stderr, "finish: %s\n", gtd_last_error()); return 1; }

    s = gtd_nav_file_write_to_path(f, "output.gtd");
    if (s != GTD_OK) { fprintf(stderr, "write: %s\n", gtd_last_error()); }

    gtd_nav_file_destroy(f);
    return s == GTD_OK ? 0 : 1;

fail:
    gtd_builder_destroy(b);
    return 1;
}
