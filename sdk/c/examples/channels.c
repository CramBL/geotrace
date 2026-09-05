/**
 * Write a .gtd file with ad-hoc sensor channels, then read them back.
 *
 * A channel is a named time series sampled at its own rate, correlated with the
 * nav track by timestamp.  It can be scalar (an inclinometer angle) or a vector
 * whose components share one sample clock (an accelerometer's x/y/z axes).  This
 * example also shows recognized milli-g values and a custom display-only unit.
 */

#include "../geotrace.h"

#include <stdint.h>
#include <stdio.h>

/* A fixed epoch keeps the output deterministic: 2024-06-01T08:00:00Z. */
#define BASE_EPOCH 1717228800U

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    gtd_builder_set_title(builder, "Channel tour");

    GtdTimestamp first_fix_time = gtd_ts_from_seconds(BASE_EPOCH);
    if (gtd_builder_add_nav_fix(builder, first_fix_time, gtd_ts_none(), 51.5074, -0.1278,
                                GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64) != GTD_OK) {
        fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
        gtd_builder_destroy(builder);
        return 1;
    }

    GtdTimestamp times[3];
    double incline_vals[3];
    double accel_vals[9]; /* 3 samples x 3 components, row-major */
    double quality_vals[3];
    for (size_t i = 0; i < 3; i++) {
        times[i] = gtd_ts_from_seconds(BASE_EPOCH + i);
        incline_vals[i] = 1.0 + ((double)i * 0.5);
        accel_vals[(i * 3) + 0] = 100.0 * (double)i;
        accel_vals[(i * 3) + 1] = 200.0;
        accel_vals[(i * 3) + 2] = 980.0;
        quality_vals[i] = 80.0 + (double)i;
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
    if (gtd_builder_add_channel(builder, &incline) != GTD_OK) {
        fprintf(stderr, "add_channel(incline): %s\n", gtd_last_error());
        gtd_builder_destroy(builder);
        return 1;
    }

    const char *comps[3] = {"x", "y", "z"};
    GtdChannel accel = {0};
    accel.name = "accel";
    accel.unit = "mg";
    accel.period_deg = GTD_NONE_F64;
    accel.components = comps;
    accel.n_components = 3;
    accel.times = times;
    accel.n_times = 3;
    accel.values = accel_vals;
    accel.n_values = 9;
    if (gtd_builder_add_channel(builder, &accel) != GTD_OK) {
        fprintf(stderr, "add_channel(accel): %s\n", gtd_last_error());
        gtd_builder_destroy(builder);
        return 1;
    }

    GtdChannel quality = {0};
    quality.name = "quality";
    quality.unit = "vendor score";
    quality.period_deg = GTD_NONE_F64;
    quality.times = times;
    quality.n_times = 3;
    quality.values = quality_vals;
    quality.n_values = 3;
    if (gtd_builder_add_channel_with_unit_mode(builder, &quality, GTD_CHANNEL_UNIT_CUSTOM) !=
        GTD_OK) {
        fprintf(stderr, "add_channel(quality): %s\n", gtd_last_error());
        gtd_builder_destroy(builder);
        return 1;
    }

    GtdNavFile *file = NULL;
    if (gtd_builder_finish(builder, &file) != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    size_t channel_count = gtd_nav_file_channel_count(file);
    printf("%zu channels:\n", channel_count);
    for (size_t i = 0; i < channel_count; i++) {
        GtdChannelInfo info;
        if (gtd_nav_file_get_channel(file, i, &info) != GTD_OK) {
            continue;
        }
        printf("  %-10s %zu samples", info.name, info.sample_count);
        if (info.has_unit) {
            printf(" [%s]", info.unit);
        }
        if (info.component_count > 0) {
            printf(" components:");
            for (size_t c = 0; c < info.component_count; c++) {
                char label[32];
                if (gtd_nav_file_get_channel_component(file, i, c, label, sizeof(label)) ==
                    GTD_OK) {
                    printf(" %s", label);
                }
            }
        }
        printf("\n");
    }

    gtd_nav_file_destroy(file);
    return 0;
}
