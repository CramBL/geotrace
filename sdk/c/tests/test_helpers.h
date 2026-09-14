#ifndef GEOTRACE_C_TEST_HELPERS_H
#define GEOTRACE_C_TEST_HELPERS_H

#include "../geotrace.h"
#include <criterion/criterion.h>
#include <math.h>
#include <stddef.h>
#include <stdint.h>

#define assert_near(a, b, eps) cr_assert(fabs((a) - (b)) < (eps))

/* The message `gtd_ts_from_micros()` reports for `INT64_MAX`. A builder entry point reports it
   after the name of the timestamp argument. */
#define INT64_MAX_MICROS_PAST_THE_RANGE_MESSAGE                                                    \
    "9223372036854775807 microseconds since the Unix epoch is past the range a UTC timestamp "     \
    "covers"

static inline GtdFileBuilder *builder_with_a_nav_fix(GtdTimestamp *time) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);
    cr_assert_eq(gtd_ts_from_seconds(1700000000, time), GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix(builder, *time, gtd_ts_none(), 51.5, -0.1, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    return builder;
}

/* Write `file` to bytes, destroy it, and return the file read back from those bytes. */
static inline GtdNavFile *reload_through_bytes(GtdNavFile *file) {
    uint8_t *bytes = NULL;
    size_t length = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(file, &bytes, &length), GTD_OK);
    gtd_nav_file_destroy(file);
    GtdNavFile *reloaded = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(bytes, length, &reloaded), GTD_OK);
    gtd_free_bytes(bytes, length);
    return reloaded;
}

/* One fix and one satellite report whose satellites have a PRN of 0 and an SNR
   of 99 dB-Hz: the two data quality issues the builder reports at finish. */
static inline GtdNavFile *build_file_with_satellite_issues(void) {
    GtdTimestamp timestamp;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);

    GtdSatellite satellites[2] = {
        {GTD_CONSTELLATION_GPS, 0, 1, GTD_SOME_F32(45.0F), GTD_SOME_F32(90.0F),
         GTD_SOME_F32(40.0F)},
        {GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F32(30.0F), GTD_SOME_F32(120.0F),
         GTD_SOME_F32(99.0F)},
    };
    cr_assert_eq(gtd_builder_add_satellite_report(builder, timestamp, gtd_ts_none(), satellites, 2),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    cr_assert_not_null(file);
    return file;
}

#endif /* GEOTRACE_C_TEST_HELPERS_H */
