/**
 * Write a .gtd file with ad-hoc sensor channels, then read them back.
 *
 * A channel is a named time series sampled at its own rate, correlated with the
 * nav track by timestamp.  It can be scalar (an inclinometer angle) or a vector
 * whose components share one sample clock (an accelerometer's x/y/z axes).  This
 * example writes one of each, reads the file back, and prints their metadata.
 */

#include "../geotrace.h"

#include <stdint.h>
#include <stdio.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

int main(void) {
    GtdFileBuilder *b = gtd_builder_create();
    gtd_builder_set_title(b, "Channel tour");

    GtdTimestamp t0 = gtd_ts_from_seconds(BASE_EPOCH);
    if (gtd_builder_add_nav_fix(b, t0, gtd_ts_none(), 51.5074, -0.1278, GTD_NONE_F64, GTD_NONE_F64,
                                GTD_NONE_F64) != GTD_OK) {
        fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
        gtd_builder_destroy(b);
        return 1;
    }

    GtdTimestamp times[3];
    double incline_vals[3];
    double accel_vals[9]; /* 3 samples x 3 components, row-major */
    for (size_t i = 0; i < 3; i++) {
        times[i] = gtd_ts_from_seconds(BASE_EPOCH + i);
        incline_vals[i] = 1.0 + (double)i * 0.5;
        accel_vals[i * 3 + 0] = 0.1 * (double)i;
        accel_vals[i * 3 + 1] = 0.2;
        accel_vals[i * 3 + 2] = 0.98;
    }

    GtdChannel incline = {0};
    incline.name = "incline";
    incline.unit = "deg";
    incline.period_deg = GTD_NONE_F64;
    incline.description = "boom inclinometer";
    incline.times = times;
    incline.n_times = 3;
    incline.values = incline_vals;
    incline.n_values = 3;
    if (gtd_builder_add_channel(b, &incline) != GTD_OK) {
        fprintf(stderr, "add_channel(incline): %s\n", gtd_last_error());
        gtd_builder_destroy(b);
        return 1;
    }

    const char *comps[3] = {"x", "y", "z"};
    GtdChannel accel = {0};
    accel.name = "accel";
    accel.unit = "g";
    accel.period_deg = GTD_NONE_F64;
    accel.components = comps;
    accel.n_components = 3;
    accel.times = times;
    accel.n_times = 3;
    accel.values = accel_vals;
    accel.n_values = 9;
    if (gtd_builder_add_channel(b, &accel) != GTD_OK) {
        fprintf(stderr, "add_channel(accel): %s\n", gtd_last_error());
        gtd_builder_destroy(b);
        return 1;
    }

    GtdNavFile *f = NULL;
    if (gtd_builder_finish(b, &f) != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    size_t n = gtd_nav_file_channel_count(f);
    printf("%zu channels:\n", n);
    for (size_t i = 0; i < n; i++) {
        GtdChannelInfo ci;
        if (gtd_nav_file_get_channel(f, i, &ci) != GTD_OK) {
            continue;
        }
        printf("  %-10s %zu samples", ci.name, ci.sample_count);
        if (ci.has_unit) {
            printf(" [%s]", ci.unit);
        }
        if (ci.component_count > 0) {
            printf(" components:");
            for (size_t c = 0; c < ci.component_count; c++) {
                char label[32];
                if (gtd_nav_file_get_channel_component(f, i, c, label, sizeof(label)) == GTD_OK) {
                    printf(" %s", label);
                }
            }
        }
        printf("\n");
    }

    gtd_nav_file_destroy(f);
    return 0;
}
