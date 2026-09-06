#ifndef GEOTRACE_C_TEST_HELPERS_H
#define GEOTRACE_C_TEST_HELPERS_H

#include "../geotrace.h"
#include <criterion/criterion.h>
#include <math.h>

#define assert_near(a, b, eps) cr_assert(fabs((a) - (b)) < (eps))

/* One fix and one satellite report whose satellites have a PRN of 0 and an SNR
   of 99 dB-Hz: the two data quality issues the builder reports at finish. */
static inline GtdNavFile *build_file_with_satellite_issues(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5, -0.1,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

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
