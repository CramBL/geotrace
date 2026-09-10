/**
 * Number parsing shared by the C examples that read numbers out of CSV text.
 */

#ifndef GEOTRACE_EXAMPLES_PARSE_NUMBER_H
#define GEOTRACE_EXAMPLES_PARSE_NUMBER_H

#include <locale.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

#define DECIMAL_FIELD_BUFSIZE 64

/* Writes the value of `text` to `out` and returns 1. Returns 0 and leaves `out`
   unchanged when `text` holds anything but one decimal number.

   `strtod` reads its decimal separator from `LC_NUMERIC`. Under a locale whose
   separator is a comma it stops at the '.' of "51.5074" and returns 51. The
   parse below writes the locale's separator in place of the field's '.' before
   it calls `strtod`. The field's own separator stays a '.' under every locale. */
static inline int parse_decimal_double(const char *text, double *out) {
    static const char DECIMAL_CHARS[] = "0123456789+-.eE";

    const char *separator = localeconv()->decimal_point;
    size_t separator_len = strlen(separator);
    char buffer[DECIMAL_FIELD_BUFSIZE];
    size_t used = 0;
    for (const char *cursor = text; *cursor != '\0'; cursor++) {
        if (strchr(DECIMAL_CHARS, *cursor) == NULL) {
            return 0;
        }
        const char *replacement = (*cursor == '.') ? separator : cursor;
        size_t replacement_len = (*cursor == '.') ? separator_len : 1;
        if (used + replacement_len >= sizeof buffer) {
            return 0;
        }
        memcpy(buffer + used, replacement, replacement_len);
        used += replacement_len;
    }
    if (used == 0) {
        return 0;
    }
    buffer[used] = '\0';

    char *end;
    double value = strtod(buffer, &end);
    if (end != buffer + used) {
        return 0;
    }
    *out = value;
    return 1;
}

#endif /* GEOTRACE_EXAMPLES_PARSE_NUMBER_H */
