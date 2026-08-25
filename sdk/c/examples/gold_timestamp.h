/**
 * Timestamp parsing for the gold dataset CSV fixtures, shared by the C gold
 * dataset example and sdk/c/tests/test_gold_timestamp.c.
 */

#ifndef GEOTRACE_GOLD_TIMESTAMP_H
#define GEOTRACE_GOLD_TIMESTAMP_H

#include "../geotrace.h"

#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdlib.h>

static inline int gold_is_leap(int y) {
    return (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
}

static inline int gold_month_days(int m, int y) {
    static const int dom[12] = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return (m == 2 && gold_is_leap(y)) ? 29 : dom[m - 1];
}

/* Reads a decimal integer at *cursor and advances it past the digits, rejecting
   a missing number and one outside int range. Parsing into long long keeps the
   range check meaningful where long is as narrow as int. */
static inline int gold_read_int(const char **cursor, int *out) {
    char *end = NULL;
    long long value;

    errno = 0;
    value = strtoll(*cursor, &end, 10);
    if (end == *cursor || errno == ERANGE || value < INT_MIN || value > INT_MAX)
        return 0;
    *cursor = end;
    *out = (int)value;
    return 1;
}

static inline int gold_read_separator(const char **cursor, char expected) {
    if (**cursor != expected)
        return 0;
    (*cursor)++;
    return 1;
}

/* Parse "YYYY-MM-DDTHH:MM:SS[.ffffff][+HH:MM]" -> GtdTimestamp.
   Returns gtd_ts_none() on failure. An offset that fails to parse is left
   unapplied, as are fractional digits past the sixth. */
static inline GtdTimestamp gold_parse_timestamp(const char *s) {
    int Y = 0, Mo = 0, D = 0, H = 0, Mi = 0, S = 0;
    const char *p = s;

    if (!s || *s == '\0')
        return gtd_ts_none();
    if (!gold_read_int(&p, &Y) || !gold_read_separator(&p, '-') || !gold_read_int(&p, &Mo) ||
        !gold_read_separator(&p, '-') || !gold_read_int(&p, &D) || !gold_read_separator(&p, 'T') ||
        !gold_read_int(&p, &H) || !gold_read_separator(&p, ':') || !gold_read_int(&p, &Mi) ||
        !gold_read_separator(&p, ':') || !gold_read_int(&p, &S))
        return gtd_ts_none();

    /* Optional fractional seconds (".ffffff"), kept as microseconds. */
    long frac_us = 0;
    if (*p == '.') {
        p++;
        char digits[7] = "000000";
        int n = 0;
        while (n < 6 && *p >= '0' && *p <= '9')
            digits[n++] = *p++;
        while (*p >= '0' && *p <= '9') /* skip sub-microsecond digits */
            p++;
        frac_us = strtol(digits, NULL, 10);
    }

    /* Optional timezone offset ("+HH:MM" / "-HH:MM"). */
    char sign = '+';
    long tz = 0;
    if (*p == '+' || *p == '-') {
        int tz_h = 0, tz_m = 0;
        sign = *p;
        p++;
        if (gold_read_int(&p, &tz_h) && gold_read_separator(&p, ':') && gold_read_int(&p, &tz_m))
            tz = (((long)tz_h * 60L) + tz_m) * 60L;
    }

    long days = 0;
    for (int y = 1970; y < Y; y++)
        days += gold_is_leap(y) ? 366 : 365;
    for (int m = 1; m < Mo; m++)
        days += gold_month_days(m, Y);
    days += D - 1;

    long secs = (days * 86400L) + (H * 3600L) + (Mi * 60L) + S;
    secs += (sign == '-') ? tz : -tz;

    return gtd_ts_from_micros(((uint64_t)secs * 1000000ULL) + (uint64_t)frac_us);
}

#endif /* GEOTRACE_GOLD_TIMESTAMP_H */
