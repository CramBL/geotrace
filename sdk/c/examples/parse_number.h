/**
 * Number parsing shared by the C examples that read numbers out of CSV text.
 */

#ifndef GEOTRACE_EXAMPLES_PARSE_NUMBER_H
#define GEOTRACE_EXAMPLES_PARSE_NUMBER_H

#include <stdlib.h>

/* Writes the value of `text` to `out` and returns 1, or returns 0 and leaves
   `out` unchanged when `text` is not a decimal number. */
static inline int parse_decimal_double(const char *text, double *out) {
    char *end;
    double value = strtod(text, &end);
    if (end == text) {
        return 0;
    }
    *out = value;
    return 1;
}

#endif /* GEOTRACE_EXAMPLES_PARSE_NUMBER_H */
