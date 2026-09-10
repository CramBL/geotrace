/**
 * Field splitting shared by the C examples that read CSV text.
 */

#ifndef GEOTRACE_EXAMPLES_CSV_FIELDS_H
#define GEOTRACE_EXAMPLES_CSV_FIELDS_H

#include <string.h>

/* Writes a pointer to each `delim`-separated field of `line` to `fields`, at
   most `max` of them, and returns how many. Overwrites each delimiter with a
   null byte. */
static inline int split_delim(char *line, char delim, char *fields[], int max) {
    int count = 0;
    char *cursor = line;
    while (count < max) {
        fields[count++] = cursor;
        cursor = strchr(cursor, delim);
        if (!cursor) {
            break;
        }
        *cursor++ = '\0';
    }
    return count;
}

static inline int split_csv(char *line, char *fields[], int max) {
    return split_delim(line, ',', fields, max);
}

#endif /* GEOTRACE_EXAMPLES_CSV_FIELDS_H */
