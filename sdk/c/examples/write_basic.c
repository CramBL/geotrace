#include "../geotrace.h"

#include <stdint.h>
#include <stdio.h>
#include <time.h>

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();

    gtd_builder_set_title(builder, "Quick tour");
    gtd_builder_set_device(builder, "Example GPS v1.0");

    GtdTimestamp first_fix_time = gtd_ts_from_seconds((uint64_t)time(NULL));

    GtdSatellite sats[] = {
        {GTD_CONSTELLATION_GPS, 1, 1, GTD_SOME_F32(45.0F), GTD_SOME_F32(90.0F),
         GTD_SOME_F32(38.0F)},
        {GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F32(30.0F), GTD_SOME_F32(180.0F),
         GTD_SOME_F32(35.5F)},
        {GTD_CONSTELLATION_GALILEO, 3, 0, GTD_NONE_F32, GTD_NONE_F32, GTD_SOME_F32(22.0F)},
    };

    GtdStatus status;

    status = gtd_builder_add_nav_fix(builder, first_fix_time, gtd_ts_none(), 51.5074, -0.1278,
                                     GTD_SOME_F64(90.0), GTD_SOME_F64(5.5), GTD_SOME_F64(3.2));
    if (status != GTD_OK) {
        fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
        goto fail;
    }

    status = gtd_builder_add_satellite_report(builder, first_fix_time, gtd_ts_none(), sats,
                                              sizeof sats / sizeof sats[0]);
    if (status != GTD_OK) {
        fprintf(stderr, "add_satellite_report: %s\n", gtd_last_error());
        goto fail;
    }

    status = gtd_builder_add_nav_fix(builder, gtd_ts_from_seconds((uint64_t)time(NULL) + 10),
                                     gtd_ts_none(), 51.5080, -0.1265, GTD_SOME_F64(85.0),
                                     GTD_SOME_F64(5.8), GTD_SOME_F64(2.9));
    if (status != GTD_OK) {
        fprintf(stderr, "add_nav_fix: %s\n", gtd_last_error());
        goto fail;
    }

    status = gtd_builder_add_annotation(builder, first_fix_time, "Start point", GTD_ICON_PIN);
    if (status != GTD_OK) {
        fprintf(stderr, "add_annotation: %s\n", gtd_last_error());
        goto fail;
    }

    GtdNavFile *file = NULL;
    status = gtd_builder_finish(builder, &file);
    builder = NULL;
    if (status != GTD_OK) {
        fprintf(stderr, "finish: %s\n", gtd_last_error());
        return 1;
    }

    status = gtd_nav_file_write_to_path(file, "output.gtd");
    if (status != GTD_OK) {
        fprintf(stderr, "write: %s\n", gtd_last_error());
    }

    gtd_nav_file_destroy(file);
    return status == GTD_OK ? 0 : 1;

fail:
    gtd_builder_destroy(builder);
    return 1;
}
