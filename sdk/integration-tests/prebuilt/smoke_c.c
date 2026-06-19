#include <geotrace.h>
#include <stdio.h>
#include <stdlib.h>

int main(void) {
    GtdFileBuilder *b = gtd_builder_create();
    if (!b) {
        fputs("gtd_builder_create returned NULL\n", stderr);
        return 1;
    }

    GtdStatus s =
        gtd_builder_add_nav_fix(b, gtd_ts_from_seconds(1700000000U), gtd_ts_none(), 51.5074,
                                -0.1278, GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64);
    if (s != GTD_OK) {
        fprintf(stderr, "gtd_builder_add_nav_fix: %d (%s)\n", s, gtd_last_error());
        gtd_builder_destroy(b);
        return 1;
    }

    GtdNavFile *f = NULL;
    s = gtd_builder_finish(b, &f);
    if (s != GTD_OK) {
        fprintf(stderr, "gtd_builder_finish: %d (%s)\n", s, gtd_last_error());
        return 1;
    }

    size_t n = gtd_nav_file_nav_point_count(f);
    gtd_nav_file_destroy(f);

    if (n != 1) {
        fprintf(stderr, "expected 1 nav point, got %zu\n", n);
        return 1;
    }

    // Surface the SDK version the header was compiled against.
    printf("smoke OK, geotrace-c %s\n", GEOTRACE_C_VERSION);
    return 0;
}
