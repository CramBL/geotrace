#include "test_helpers.h"
#include <criterion/criterion.h>
#include <stdio.h>
#include <string.h>

#define MAX_RECORDS 16
#define MAX_TEXT    512

/* `level` stores the `GtdLogLevel` values as the integers they are: there is no
   `GtdLogLevel` of 0 for this struct to be initialized with. */
typedef struct {
    size_t count;
    int32_t level[MAX_RECORDS];
    char target[MAX_RECORDS][MAX_TEXT];
    char message[MAX_RECORDS][MAX_TEXT];
} RecordedLog;

/* NOLINTBEGIN(bugprone-easily-swappable-parameters): the C SDK fixes the order
   of a log callback's parameters. */
static void record_into(GtdLogLevel level, const char *target, const char *message,
                        void *user_data) {
    /* NOLINTEND(bugprone-easily-swappable-parameters) */
    RecordedLog *recorded = (RecordedLog *)user_data;
    if (recorded->count >= MAX_RECORDS) {
        return;
    }
    recorded->level[recorded->count] = (int32_t)level;
    (void)snprintf(recorded->target[recorded->count], MAX_TEXT, "%s", target);
    (void)snprintf(recorded->message[recorded->count], MAX_TEXT, "%s", message);
    recorded->count++;
}

static int has_a_message_containing(const RecordedLog *recorded, const char *needle) {
    for (size_t i = 0; i < recorded->count; i++) {
        if (strstr(recorded->message[i], needle) != NULL) {
            return 1;
        }
    }
    return 0;
}

/* One fix and one satellite report 1.5 s later, past the association window:
   the builder reports the ghost nav fix it creates for the report, at
   GTD_LOG_DEBUG. */
static void build_a_file_with_a_ghost_fix(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    GtdTimestamp fix_time;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &fix_time), GTD_OK);
    GtdTimestamp report_time;
    cr_assert_eq(gtd_ts_from_millis(1700000001500, &report_time), GTD_OK);

    cr_assert_eq(gtd_builder_add_nav_fix(builder, fix_time, gtd_ts_none(), 51.5, -0.1, GTD_NONE_F64,
                                         GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    GtdSatellite satellites[1] = {
        {GTD_CONSTELLATION_GPS, 5, 1, GTD_SOME_F32(30.0F), GTD_SOME_F32(120.0F),
         GTD_SOME_F32(40.0F)},
    };
    cr_assert_eq(
        gtd_builder_add_satellite_report(builder, report_time, gtd_ts_none(), satellites, 1),
        GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    gtd_nav_file_destroy(file);
}

Test(log_callback, a_callback_receives_the_builder_warnings) {
    RecordedLog recorded = {0};
    cr_assert_eq(gtd_set_log_callback(record_into, &recorded), GTD_OK);

    gtd_nav_file_destroy(build_file_with_satellite_issues());
    gtd_clear_log_callback();

    cr_assert_eq(recorded.count, 2);
    cr_assert_eq(recorded.level[0], GTD_LOG_WARN);
    cr_assert(has_a_message_containing(&recorded, "PRN 0"));
    cr_assert(has_a_message_containing(&recorded, "99 dB-Hz"));
    cr_assert_str_eq(recorded.target[0], "geotrace_sdk::builder");
}

Test(log_callback, a_cleared_callback_receives_nothing) {
    RecordedLog recorded = {0};
    cr_assert_eq(gtd_set_log_callback(record_into, &recorded), GTD_OK);
    gtd_clear_log_callback();

    gtd_nav_file_destroy(build_file_with_satellite_issues());

    cr_assert_eq(recorded.count, 0);
}

Test(log_callback, a_null_callback_clears_the_callback) {
    RecordedLog recorded = {0};
    cr_assert_eq(gtd_set_log_callback(record_into, &recorded), GTD_OK);
    cr_assert_eq(gtd_set_log_callback(NULL, NULL), GTD_OK);

    gtd_nav_file_destroy(build_file_with_satellite_issues());

    cr_assert_eq(recorded.count, 0);
}

Test(log_callback, a_second_callback_replaces_the_first) {
    RecordedLog first = {0};
    RecordedLog second = {0};
    cr_assert_eq(gtd_set_log_callback(record_into, &first), GTD_OK);
    cr_assert_eq(gtd_set_log_callback(record_into, &second), GTD_OK);

    gtd_nav_file_destroy(build_file_with_satellite_issues());
    gtd_clear_log_callback();

    cr_assert_eq(first.count, 0);
    cr_assert_eq(second.count, 2);
}

Test(log_callback, the_default_level_drops_a_debug_record) {
    RecordedLog recorded = {0};
    cr_assert_eq(gtd_set_log_callback(record_into, &recorded), GTD_OK);

    build_a_file_with_a_ghost_fix();
    gtd_clear_log_callback();

    cr_assert_eq(recorded.count, 0);
}

Test(log_callback, the_debug_level_forwards_a_debug_record) {
    RecordedLog recorded = {0};
    cr_assert_eq(gtd_set_log_callback(record_into, &recorded), GTD_OK);
    gtd_set_log_level(GTD_LOG_DEBUG);

    build_a_file_with_a_ghost_fix();
    gtd_clear_log_callback();

    cr_assert_eq(recorded.count, 1);
    cr_assert_eq(recorded.level[0], GTD_LOG_DEBUG);
    cr_assert(has_a_message_containing(&recorded, "ghost nav fix"));
}

Test(log_callback, the_error_level_drops_a_warning) {
    RecordedLog recorded = {0};
    cr_assert_eq(gtd_set_log_callback(record_into, &recorded), GTD_OK);
    gtd_set_log_level(GTD_LOG_ERROR);

    gtd_nav_file_destroy(build_file_with_satellite_issues());
    gtd_clear_log_callback();

    cr_assert_eq(recorded.count, 0);
}
