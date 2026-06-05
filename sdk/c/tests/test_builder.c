#include "../geotrace.h"

#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <cmocka.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

#define assert_near(a, b, eps) assert_true(fabs((a) - (b)) < (eps))

static void test_builder_basic_write(void **state) {
    (void)state;

    GtdFileBuilder *b = gtd_builder_create();
    assert_non_null(b);

    assert_int_equal(gtd_builder_set_title(b, "Test file"), GTD_OK);
    assert_int_equal(gtd_builder_set_device(b, "cmocka test"), GTD_OK);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    assert_int_equal(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 51.5074, -0.1278,
                                             GTD_SOME_F64(180.0), GTD_SOME_F64(3.0),
                                             GTD_SOME_F64(5.0)),
                     GTD_OK);

    GtdNavFile *f = NULL;
    assert_int_equal(gtd_builder_finish(b, &f), GTD_OK);
    assert_non_null(f);

    assert_int_equal(gtd_nav_file_nav_point_count(f), 1);

    GtdNavPointInfo p;
    assert_int_equal(gtd_nav_file_get_nav_point(f, 0, &p), GTD_OK);
    assert_near(p.lat_deg, 51.5074, 1e-9);
    assert_near(p.lon_deg, -0.1278, 1e-9);
    assert_int_equal(p.speed_mps.present, 1);
    assert_near(p.speed_mps.value, 3.0, 1e-9);

    gtd_nav_file_destroy(f);
}

static void test_builder_to_bytes_round_trip(void **state) {
    (void)state;

    GtdFileBuilder *b = gtd_builder_create();
    assert_non_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    assert_int_equal(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 48.8566, 2.3522, GTD_NONE_F64,
                                             GTD_NONE_F64, GTD_NONE_F64),
                     GTD_OK);

    GtdNavFile *f = NULL;
    assert_int_equal(gtd_builder_finish(b, &f), GTD_OK);

    uint8_t *buf = NULL;
    size_t len = 0;
    assert_int_equal(gtd_nav_file_to_bytes(f, &buf, &len), GTD_OK);
    assert_non_null(buf);
    assert_true(len > 0);
    gtd_nav_file_destroy(f);

    GtdNavFile *f2 = NULL;
    assert_int_equal(gtd_nav_file_from_bytes(buf, len, &f2), GTD_OK);
    assert_non_null(f2);
    assert_int_equal(gtd_nav_file_nav_point_count(f2), 1);

    GtdNavPointInfo p;
    assert_int_equal(gtd_nav_file_get_nav_point(f2, 0, &p), GTD_OK);
    assert_near(p.lat_deg, 48.8566, 1e-6);

    gtd_nav_file_destroy(f2);
    gtd_free_bytes(buf, len);
}

static void test_builder_no_fixes_error(void **state) {
    (void)state;

    GtdFileBuilder *b = gtd_builder_create();
    assert_non_null(b);

    /* NoNavFixes is only returned when there are annotations but no fixes;
       an empty builder (no fixes, no annotations) is valid and returns OK. */
    assert_int_equal(
        gtd_builder_add_annotation(b, gtd_ts_from_seconds(1700000000ULL), "note", GTD_ICON_AUTO),
        GTD_OK);

    GtdNavFile *f = NULL;
    GtdStatus s = gtd_builder_finish(b, &f);
    assert_int_equal(s, GTD_ERR_NO_NAV_FIXES);
    assert_null(f);
    assert_non_null(gtd_last_error());
}

static void test_builder_satellite_report(void **state) {
    (void)state;

    GtdFileBuilder *b = gtd_builder_create();
    assert_non_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 7, 1, GTD_SOME_F64(55.0), GTD_SOME_F64(120.0), GTD_SOME_F64(40.0)},
        {GTD_CONSTELLATION_GLONASS, 2, 0, GTD_NONE_F64, GTD_NONE_F64, GTD_SOME_F64(28.0)},
    };

    assert_int_equal(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 40.7128, -74.0060, GTD_NONE_F64,
                                             GTD_NONE_F64, GTD_NONE_F64),
                     GTD_OK);

    assert_int_equal(gtd_builder_add_satellite_report(b, t, gtd_ts_none(), sats, 2), GTD_OK);

    GtdNavFile *f = NULL;
    assert_int_equal(gtd_builder_finish(b, &f), GTD_OK);

    GtdNavPointInfo p;
    assert_int_equal(gtd_nav_file_get_nav_point(f, 0, &p), GTD_OK);
    assert_int_equal(p.sat_count, 2);

    GtdSatInfo s0;
    assert_int_equal(gtd_nav_file_get_satellite(f, 0, 0, &s0), GTD_OK);
    assert_int_equal(s0.prn, 7);
    assert_int_equal(s0.in_fix, 1);
    assert_int_equal(s0.snr_dbhz.present, 1);
    assert_near(s0.snr_dbhz.value, 40.0, 1e-6);

    gtd_nav_file_destroy(f);
}

static void test_builder_event_marker(void **state) {
    (void)state;

    GtdFileBuilder *b = gtd_builder_create();
    assert_non_null(b);

    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);

    assert_int_equal(gtd_builder_add_nav_fix(b, t, gtd_ts_none(), 35.6762, 139.6503, GTD_NONE_F64,
                                             GTD_NONE_F64, GTD_NONE_F64),
                     GTD_OK);

    assert_int_equal(gtd_builder_add_event_marker(b, "system/startup", t, "Device started"),
                     GTD_OK);

    assert_int_equal(
        gtd_builder_add_event_marker_style(b, "system/startup", GTD_ICON_GEAR, "#00FF00"), GTD_OK);

    GtdNavFile *f = NULL;
    assert_int_equal(gtd_builder_finish(b, &f), GTD_OK);

    assert_int_equal(gtd_nav_file_event_marker_count(f), 1);

    GtdEventMarkerInfo em;
    assert_int_equal(gtd_nav_file_get_event_marker(f, 0, &em), GTD_OK);
    assert_string_equal(em.variant_path, "system/startup");
    assert_int_equal(em.has_annotation, 1);
    assert_string_equal(em.annotation, "Device started");

    gtd_nav_file_destroy(f);
}

#ifdef GTD_FIXTURE_PATH
static void test_open_fixture(void **state) {
    (void)state;

    GtdNavFile *f = NULL;
    GtdStatus s = gtd_nav_file_open(GTD_FIXTURE_PATH, &f);
    assert_int_equal(s, GTD_OK);
    assert_non_null(f);
    assert_true(gtd_nav_file_nav_point_count(f) >= 1);
    gtd_nav_file_destroy(f);
}
#endif

int main(void) {
    const struct CMUnitTest tests[] = {
        cmocka_unit_test(test_builder_basic_write),
        cmocka_unit_test(test_builder_to_bytes_round_trip),
        cmocka_unit_test(test_builder_no_fixes_error),
        cmocka_unit_test(test_builder_satellite_report),
        cmocka_unit_test(test_builder_event_marker),
#ifdef GTD_FIXTURE_PATH
        cmocka_unit_test(test_open_fixture),
#endif
    };
    return cmocka_run_group_tests(tests, NULL, NULL);
}
