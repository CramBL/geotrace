#include "../geotrace.h"
#include <criterion/criterion.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

typedef struct {
    const char *(*c_string_getter)(const GtdNavFile *file);
    const char *(*getter_with_length)(const GtdNavFile *file, size_t *length);
    const char *value_with_a_nul_byte;
    size_t value_with_a_nul_byte_length;
    const char *ordinary_value;
} MetadataField;

#define STRING_WITH_LENGTH(literal) (literal), (sizeof(literal) - 1)

/* `value_with_a_nul_byte` is the value in `metadata_with_a_nul_byte.gtd`.
   `file_with_ordinary_metadata_read_back()` sets `ordinary_value`. */
static const MetadataField METADATA_FIELDS[] = {
    {gtd_nav_file_title, gtd_nav_file_title_with_length, STRING_WITH_LENGTH("title\0after"),
     "Ride home"},
    {gtd_nav_file_device, gtd_nav_file_device_with_length, STRING_WITH_LENGTH("device\0after"),
     "u-blox F9P"},
    {gtd_nav_file_notes, gtd_nav_file_notes_with_length, STRING_WITH_LENGTH("notes\0after"),
     "Light rain"},
    {gtd_nav_file_identity, gtd_nav_file_identity_with_length,
     STRING_WITH_LENGTH("identity\0after"), "rover-7"},
    {gtd_nav_file_travel_mode, gtd_nav_file_travel_mode_with_length,
     STRING_WITH_LENGTH("car\0after"), "bicycle"},
};

#define METADATA_FIELD_COUNT (sizeof(METADATA_FIELDS) / sizeof(METADATA_FIELDS[0]))

static GtdNavFile *finish_with_a_nav_fix_and_read_back(GtdFileBuilder *builder) {
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &timestamp), GTD_OK);
    cr_assert_eq(gtd_builder_add_nav_fix(builder, timestamp, gtd_ts_none(), 51.5, -0.1,
                                         GTD_NONE_F64, GTD_NONE_F64, GTD_NONE_F64),
                 GTD_OK);
    GtdNavFile *built = NULL;
    cr_assert_eq(gtd_builder_finish(builder, &built), GTD_OK);

    uint8_t *bytes = NULL;
    size_t byte_count = 0;
    cr_assert_eq(gtd_nav_file_to_bytes(built, &bytes, &byte_count), GTD_OK);
    gtd_nav_file_destroy(built);

    GtdNavFile *read_back = NULL;
    cr_assert_eq(gtd_nav_file_from_bytes(bytes, byte_count, &read_back), GTD_OK);
    gtd_free_bytes(bytes, byte_count);
    return read_back;
}

static GtdNavFile *file_with_ordinary_metadata_read_back(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);
    cr_assert_eq(gtd_builder_set_title(builder, "Ride home"), GTD_OK);
    cr_assert_eq(gtd_builder_set_device(builder, "u-blox F9P"), GTD_OK);
    cr_assert_eq(gtd_builder_set_notes(builder, "Light rain"), GTD_OK);
    cr_assert_eq(gtd_builder_set_identity(builder, "rover-7"), GTD_OK);
    cr_assert_eq(gtd_builder_set_travel_mode(builder, GTD_TRAVEL_MODE_BICYCLE), GTD_OK);
    return finish_with_a_nav_fix_and_read_back(builder);
}

#ifdef GTD_METADATA_WITH_A_NUL_BYTE_FIXTURE_PATH
Test(metadata, getters_with_length_return_a_value_with_a_nul_byte_whole) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(GTD_METADATA_WITH_A_NUL_BYTE_FIXTURE_PATH, &file), GTD_OK);
    for (size_t i = 0; i < METADATA_FIELD_COUNT; i++) {
        const MetadataField *field = &METADATA_FIELDS[i];
        size_t length = SIZE_MAX;
        const char *value = field->getter_with_length(file, &length);
        cr_assert_not_null(value);
        cr_assert_eq(length, field->value_with_a_nul_byte_length);
        cr_assert_arr_eq(value, field->value_with_a_nul_byte, length);
        cr_assert_eq(value[length], '\0');
    }
    gtd_nav_file_destroy(file);
}

Test(metadata, c_string_getters_return_null_for_a_value_with_a_nul_byte) {
    GtdNavFile *file = NULL;
    cr_assert_eq(gtd_nav_file_open(GTD_METADATA_WITH_A_NUL_BYTE_FIXTURE_PATH, &file), GTD_OK);
    for (size_t i = 0; i < METADATA_FIELD_COUNT; i++) {
        cr_assert_null(METADATA_FIELDS[i].c_string_getter(file));
    }
    gtd_nav_file_destroy(file);
}
#endif

Test(metadata, both_getters_return_an_ordinary_value) {
    GtdNavFile *file = file_with_ordinary_metadata_read_back();
    for (size_t i = 0; i < METADATA_FIELD_COUNT; i++) {
        const MetadataField *field = &METADATA_FIELDS[i];
        cr_assert_str_eq(field->c_string_getter(file), field->ordinary_value);
        size_t length = SIZE_MAX;
        cr_assert_str_eq(field->getter_with_length(file, &length), field->ordinary_value);
        cr_assert_eq(length, strlen(field->ordinary_value));
    }
    gtd_nav_file_destroy(file);
}

Test(metadata, getters_with_length_accept_a_null_length) {
    GtdNavFile *file = file_with_ordinary_metadata_read_back();
    for (size_t i = 0; i < METADATA_FIELD_COUNT; i++) {
        const MetadataField *field = &METADATA_FIELDS[i];
        cr_assert_str_eq(field->getter_with_length(file, NULL), field->ordinary_value);
    }
    gtd_nav_file_destroy(file);
}

Test(metadata, both_getters_return_null_for_a_value_that_is_not_set) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);
    GtdNavFile *file = finish_with_a_nav_fix_and_read_back(builder);
    for (size_t i = 0; i < METADATA_FIELD_COUNT; i++) {
        const MetadataField *field = &METADATA_FIELDS[i];
        cr_assert_null(field->c_string_getter(file));
        size_t length = SIZE_MAX;
        cr_assert_null(field->getter_with_length(file, &length));
        cr_assert_eq(length, 0);
    }
    gtd_nav_file_destroy(file);
}

Test(metadata, the_getter_with_length_returns_a_title_set_to_the_empty_string) {
    GtdFileBuilder *builder = gtd_builder_create();
    cr_assert_not_null(builder);
    cr_assert_eq(gtd_builder_set_title(builder, ""), GTD_OK);
    GtdNavFile *file = finish_with_a_nav_fix_and_read_back(builder);

    size_t length = SIZE_MAX;
    const char *title = gtd_nav_file_title_with_length(file, &length);
    cr_assert_not_null(title);
    cr_assert_eq(length, 0);
    cr_assert_eq(title[0], '\0');
    cr_assert_str_eq(gtd_nav_file_title(file), "");
    gtd_nav_file_destroy(file);
}
