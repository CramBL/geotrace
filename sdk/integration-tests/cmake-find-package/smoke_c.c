#include <geotrace.h>

#include <math.h>
#include <stdint.h>
#include <stdio.h>

// 2024-06-01T08:00:00Z keeps the written file deterministic.
#define FIX_SECONDS INT64_C(1717228800)
#define FIX_LAT_DEG 51.5074
#define FIX_LON_DEG (-0.1278)

// The tolerance is this tight because the format stores a coordinate as an f64:
// a lossless round trip reproduces the written latitude exactly.
#define LAT_TOLERANCE_DEG 1e-9

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    if (!builder) {
        fputs("gtd_builder_create returned NULL\n", stderr);
        return 1;
    }

    GtdTimestamp fix_time;
    GtdStatus status = gtd_ts_from_seconds(FIX_SECONDS, &fix_time);
    if (status != GTD_OK) {
        fprintf(stderr, "gtd_ts_from_seconds: %d (%s)\n", status, gtd_last_error());
        gtd_builder_destroy(builder);
        return 1;
    }

    status = gtd_builder_add_nav_fix(builder, fix_time, gtd_ts_none(), FIX_LAT_DEG, FIX_LON_DEG,
                                     GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64);
    if (status != GTD_OK) {
        fprintf(stderr, "gtd_builder_add_nav_fix: %d (%s)\n", status, gtd_last_error());
        gtd_builder_destroy(builder);
        return 1;
    }

    GtdNavFile *written = NULL;
    status = gtd_builder_finish(builder, &written);
    if (status != GTD_OK) {
        fprintf(stderr, "gtd_builder_finish: %d (%s)\n", status, gtd_last_error());
        return 1;
    }

    status = gtd_nav_file_write_to_path(written, GEOTRACE_SMOKE_PATH);
    gtd_nav_file_destroy(written);
    if (status != GTD_OK) {
        fprintf(stderr, "gtd_nav_file_write_to_path: %d (%s)\n", status, gtd_last_error());
        return 1;
    }

    GtdNavFile *read_back = NULL;
    status = gtd_nav_file_open(GEOTRACE_SMOKE_PATH, &read_back);
    if (status != GTD_OK) {
        fprintf(stderr, "gtd_nav_file_open: %d (%s)\n", status, gtd_last_error());
        return 1;
    }

    const size_t count = gtd_nav_file_nav_point_count(read_back);
    GtdNavPointInfo point;
    status = gtd_nav_file_get_nav_point(read_back, 0, &point);
    gtd_nav_file_destroy(read_back);

    if (count != 1) {
        fprintf(stderr, "expected 1 nav point, got %zu\n", count);
        return 1;
    }
    if (status != GTD_OK) {
        fprintf(stderr, "gtd_nav_file_get_nav_point: %d (%s)\n", status, gtd_last_error());
        return 1;
    }
    if (fabs(point.lat_deg - FIX_LAT_DEG) > LAT_TOLERANCE_DEG) {
        fprintf(stderr, "expected latitude %f, got %f\n", FIX_LAT_DEG, point.lat_deg);
        return 1;
    }

    printf("smoke_c OK, geotrace-c %s\n", GEOTRACE_C_VERSION);
    return 0;
}
