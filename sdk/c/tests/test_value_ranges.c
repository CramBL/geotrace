#include "../geotrace.h"
#include <criterion/criterion.h>
#include <math.h>

#define assert_near(a, b, eps) cr_assert(fabs((a) - (b)) < (eps))

#ifdef GTD_OUT_OF_RANGE_FIXTURE_PATH
Test(value_ranges, out_of_range_coordinates_read_verbatim) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(GTD_OUT_OF_RANGE_FIXTURE_PATH, &file), GTD_OK);
    cr_assert_not_null(file);
    cr_assert_eq(gtd_nav_file_nav_point_count(file), 4);

    GtdNavPointInfo nan_lat;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 0, &nan_lat), GTD_OK);
    cr_assert(isnan(nan_lat.lat_deg));

    GtdNavPointInfo lat_91;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 1, &lat_91), GTD_OK);
    assert_near(lat_91.lat_deg, 91.0, 1e-9);

    GtdNavPointInfo lon_minus_181;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 2, &lon_minus_181), GTD_OK);
    assert_near(lon_minus_181.lon_deg, -181.0, 1e-9);

    GtdNavPointInfo heading_675;
    cr_assert_eq(gtd_nav_file_get_nav_point(file, 3, &heading_675), GTD_OK);
    cr_assert_eq(heading_675.heading_deg.present, 1);
    assert_near(heading_675.heading_deg.value, 675.0, 1e-9);

    gtd_nav_file_destroy(file);
}
#endif
