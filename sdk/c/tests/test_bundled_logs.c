#include "../geotrace.h"
#include "test_helpers.h"
#include <criterion/criterion.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

Test(bundled_logs, multiple_logs_round_trip_sorted_by_name) {
    GtdTimestamp timestamp;
    GtdFileBuilder *builder = builder_with_a_nav_fix(&timestamp);

    const char *zeta_text = "May 29 18:48:24 host proc: started\n";
    const char *alpha_text = "2026-05-29T18:48:24Z boot complete\n";
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "zeta.log", zeta_text, strlen(zeta_text)),
                 GTD_OK);
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "alpha.log", alpha_text, strlen(alpha_text)),
                 GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    GtdNavFile *reloaded = reload_through_bytes(file);

    cr_assert_eq(gtd_nav_file_bundled_log_count(reloaded), 2);

    GtdBundledLogInfo info;
    cr_assert_eq(gtd_nav_file_get_bundled_log(reloaded, 0, &info), GTD_OK);
    cr_assert_str_eq(info.name, "alpha.log");
    cr_assert_eq(info.text_length, strlen(alpha_text));
    cr_assert_str_eq(info.text, alpha_text);

    cr_assert_eq(gtd_nav_file_get_bundled_log(reloaded, 1, &info), GTD_OK);
    cr_assert_str_eq(info.name, "zeta.log");
    cr_assert_eq(info.text_length, strlen(zeta_text));
    cr_assert_str_eq(info.text, zeta_text);

    cr_assert_eq(gtd_nav_file_get_bundled_log(reloaded, 2, &info), GTD_ERR_OUT_OF_RANGE);
    cr_assert_not_null(gtd_last_error());

    gtd_nav_file_destroy(reloaded);
}

Test(bundled_logs, validation_errors_populate_last_error) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    const char *valid_text = "2026-05-29T18:48:24Z boot complete\n";

    /* Empty log text. */
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "empty.log", "", 0),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    /* Unrecognised timestamp format. */
    const char *plain_text = "no timestamp on this line\n";
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "plain.log", plain_text, strlen(plain_text)),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    /* Invalid log names. */
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "", valid_text, strlen(valid_text)),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    cr_assert_eq(
        gtd_builder_add_bundled_log(builder, "sub/name.log", valid_text, strlen(valid_text)),
        GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    cr_assert_eq(gtd_builder_add_bundled_log(builder, "..", valid_text, strlen(valid_text)),
                 GTD_ERR_INVALID_ARGUMENT);
    cr_assert_not_null(gtd_last_error());

    /* Non-UTF-8 content. */
    const char bad_utf8[2] = {(char)0xff, (char)0xfe};
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "bad.log", bad_utf8, sizeof bad_utf8),
                 GTD_ERR_UTF8);
    cr_assert_not_null(gtd_last_error());

    gtd_builder_destroy(builder);
}

Test(bundled_logs, add_from_file_reads_log_and_reports_missing_path) {
    const char *path = "c_bundled_log_from_file_test.log";
    const char *content = "2026-05-29 18:48:24 daemon started\n";

    FILE *handle = fopen(path, "wb");
    cr_assert_not_null(handle);
    cr_assert_eq(fwrite(content, 1, strlen(content), handle), strlen(content));
    cr_assert_eq(fclose(handle), 0);

    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    cr_assert_eq(gtd_builder_add_bundled_log_from_file(builder, "does_not_exist_bundled.log"),
                 GTD_ERR_IO);
    cr_assert_not_null(gtd_last_error());

    cr_assert_eq(gtd_builder_add_bundled_log_from_file(builder, path), GTD_OK);
    (void)remove(path);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_OK);
    GtdNavFile *reloaded = reload_through_bytes(file);

    cr_assert_eq(gtd_nav_file_bundled_log_count(reloaded), 1);
    GtdBundledLogInfo info;
    cr_assert_eq(gtd_nav_file_get_bundled_log(reloaded, 0, &info), GTD_OK);
    cr_assert_str_eq(info.name, path);
    cr_assert_eq(info.text_length, strlen(content));
    cr_assert_str_eq(info.text, content);

    gtd_nav_file_destroy(reloaded);
}

Test(bundled_logs, duplicate_name_is_rejected_on_finish) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);

    const char *text = "2026-05-29T18:48:24Z entry\n";
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "dup.log", text, strlen(text)), GTD_OK);
    cr_assert_eq(gtd_builder_add_bundled_log(builder, "dup.log", text, strlen(text)), GTD_OK);

    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &file), GTD_ERR_INVALID_ARGUMENT);
    cr_assert_null(file);
    cr_assert_not_null(gtd_last_error());
    cr_assert_not_null(strstr(gtd_last_error(), "dup.log"));
}
