#include "../geotrace.h"
#include "test_helpers.h"
#include <criterion/criterion.h>
#include <math.h>

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

Test(value_ranges, a_file_without_satellite_reports_has_no_satellite_warnings) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(GTD_OUT_OF_RANGE_FIXTURE_PATH, &file), GTD_OK);

    cr_assert_eq(gtd_nav_file_satellite_warning_count(file), 0);

    GtdSatelliteWarningInfo warning;
    cr_assert_eq(gtd_nav_file_get_satellite_warning(file, 0, &warning), GTD_ERR_OUT_OF_RANGE);

    gtd_nav_file_destroy(file);
}
#endif

Test(value_ranges, satellite_warnings_report_the_prn_and_the_snr_sentinel) {
    GtdNavFile *file = build_file_with_satellite_issues();

    cr_assert_eq(gtd_nav_file_satellite_warning_count(file), 2);

    GtdSatelliteWarningInfo prn_zero;
    cr_assert_eq(gtd_nav_file_get_satellite_warning(file, 0, &prn_zero), GTD_OK);
    cr_assert_eq(prn_zero.count, 1);
    cr_assert_str_eq(prn_zero.issue, "satellite(s) with PRN 0");
    cr_assert_str_eq(prn_zero.description, "PRN 0 is reserved and undefined in NMEA");

    GtdSatelliteWarningInfo snr_sentinel;
    cr_assert_eq(gtd_nav_file_get_satellite_warning(file, 1, &snr_sentinel), GTD_OK);
    cr_assert_eq(snr_sentinel.count, 1);
    /* The issue text contains "\xe2\x89\x88 99 dB-Hz". Writing that
       character as its UTF-8 bytes keeps the comparison independent of the
       compiler's source and execution character sets. */
    cr_assert_str_eq(snr_sentinel.issue, "satellite(s) with SNR \xe2\x89\x88 99 dB-Hz");
    cr_assert_str_eq(snr_sentinel.description,
                     "common firmware sentinel for unavailable signal strength; omit the SNR "
                     "field when no measurement is available");

    GtdSatelliteWarningInfo past_the_end;
    cr_assert_eq(gtd_nav_file_get_satellite_warning(file, 2, &past_the_end), GTD_ERR_OUT_OF_RANGE);

    gtd_nav_file_destroy(file);
}
