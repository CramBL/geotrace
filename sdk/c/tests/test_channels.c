#include "../geotrace.h"
#include <criterion/criterion.h>
#include <math.h>
#include <string.h>

#define assert_near(a, b, eps) cr_assert(fabs((a) - (b)) < (eps))

/* Write a scalar and a vector channel, then read them back from a byte buffer. */
Test(channels, round_trip) {
    GtdFileBuilder *b = gtd_builder_create();
    cr_assert_not_null(b);

    GtdTimestamp t0 = gtd_ts_from_seconds(1700000000ULL);
    cr_assert_eq(gtd_builder_add_nav_fix(b, t0, gtd_ts_none(), 51.5, -0.1, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);

    GtdTimestamp times[2] = {t0, gtd_ts_from_seconds(1700000001ULL)};

    /* A scalar channel carrying a wrap period. */
    double incline_vals[2] = {1.5, 2.0};
    GtdChannel incline = {0};
    incline.name = "incline";
    incline.unit = "deg";
    incline.period_deg = GTD_SOME_F64(360.0);
    incline.times = times;
    incline.n_times = 2;
    incline.values = incline_vals;
    incline.n_values = 2;
    cr_assert_eq(gtd_builder_add_channel(b, &incline), GTD_OK);

    /* A vector channel, values row-major: [x0, y0, z0, x1, y1, z1]. */
    const char *comps[3] = {"x", "y", "z"};
    double accel_vals[6] = {0.1, 0.2, 0.98, -0.1, 0.3, 1.02};
    GtdChannel accel = {0};
    accel.name = "accel";
    accel.unit = "g";
    accel.period_deg = GTD_NONE_F64;
    accel.components = comps;
    accel.n_components = 3;
    accel.times = times;
    accel.n_times = 2;
    accel.values = accel_vals;
    accel.n_values = 6;
    cr_assert_eq(gtd_builder_add_channel(b, &accel), GTD_OK);

    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_OK);
    uint8_t *buf = NULL;
    size_t len = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(f, &buf, &len), GTD_OK);
    gtd_nav_file_destroy(f);

    GtdNavFile *f2 = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(buf, len, &f2), GTD_OK);
    cr_assert_eq(gtd_nav_file_channel_count(f2), 2);

    /* Channels sort by name: accel (vector) at 0, incline (scalar) at 1. */
    GtdChannelInfo ci;
    cr_assert_eq(gtd_nav_file_get_channel(f2, 0, &ci), GTD_OK);
    cr_assert_str_eq(ci.name, "accel");
    cr_assert_eq(ci.has_unit, 1);
    cr_assert_str_eq(ci.unit, "g");
    cr_assert_eq(ci.component_count, 3);
    cr_assert_eq(ci.sample_count, 2);
    cr_assert_eq(ci.period_deg.present, 0);

    char label[16];
    cr_assert_eq(gtd_nav_file_get_channel_component(f2, 0, 2, label, sizeof(label)), GTD_OK);
    cr_assert_str_eq(label, "z");

    GtdTimestamp got_times[2];
    cr_assert_eq(gtd_nav_file_channel_times(f2, 0, got_times, 2), 2);
    cr_assert_eq(got_times[0].unix_micros, times[0].unix_micros);
    cr_assert_eq(got_times[1].unix_micros, times[1].unix_micros);

    double got_vals[6];
    cr_assert_eq(gtd_nav_file_channel_values(f2, 0, got_vals, 6), 6);
    assert_near(got_vals[0], 0.1, 1e-12);
    assert_near(got_vals[5], 1.02, 1e-12);

    cr_assert_eq(gtd_nav_file_get_channel(f2, 1, &ci), GTD_OK);
    cr_assert_str_eq(ci.name, "incline");
    cr_assert_eq(ci.component_count, 0);
    cr_assert_eq(ci.period_deg.present, 1);
    assert_near(ci.period_deg.value, 360.0, 1e-9);

    gtd_nav_file_destroy(f2);
    gtd_free_bytes(buf, len);
}

Test(channels, invalid_name_is_rejected) {
    GtdFileBuilder *b = gtd_builder_create();
    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    double v = 1.0;
    GtdChannel ch = {0};
    ch.name = "Bad Name";
    ch.period_deg = GTD_NONE_F64;
    ch.times = &t;
    ch.n_times = 1;
    ch.values = &v;
    ch.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(b, &ch), GTD_ERR_INVALID_CHANNEL);
    gtd_builder_destroy(b);
}

Test(channels, length_mismatch_is_rejected) {
    GtdFileBuilder *b = gtd_builder_create();
    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    double v[2] = {1.0, 2.0};
    GtdChannel ch = {0};
    ch.name = "accel";
    ch.period_deg = GTD_NONE_F64;
    ch.times = &t;
    ch.n_times = 1; /* one sample */
    ch.values = v;
    ch.n_values = 2; /* but two scalar values */
    cr_assert_eq(gtd_builder_add_channel(b, &ch), GTD_ERR_INVALID_CHANNEL);
    gtd_builder_destroy(b);
}

Test(channels, duplicate_name_fails_at_finish) {
    GtdFileBuilder *b = gtd_builder_create();
    GtdTimestamp t = gtd_ts_from_seconds(1700000000ULL);
    double v = 1.0;
    GtdChannel ch = {0};
    ch.name = "accel";
    ch.period_deg = GTD_NONE_F64;
    ch.times = &t;
    ch.n_times = 1;
    ch.values = &v;
    ch.n_values = 1;
    cr_assert_eq(gtd_builder_add_channel(b, &ch), GTD_OK);
    cr_assert_eq(gtd_builder_add_channel(b, &ch), GTD_OK);
    GtdNavFile *f = NULL;
    cr_assert_eq(gtd_builder_finish(b, &f), GTD_ERR_INVALID_CHANNEL);
    cr_assert_null(f);
}
