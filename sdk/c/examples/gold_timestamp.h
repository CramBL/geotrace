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

static inline int gold_is_leap(int year) {
    return (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
}

static inline int gold_month_days(int month, int year) {
    static const int dom[12] = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return (month == 2 && gold_is_leap(year)) ? 29 : dom[month - 1];
}

/* Reads a decimal integer at *cursor and advances it past the digits, rejecting
   a missing number and one outside int range. Parsing into long long keeps the
   range check meaningful where long is as narrow as int. */
static inline int gold_read_int(const char **cursor, int *out) {
    char *end = NULL;
    long long value;

    errno = 0;
    value = strtoll(*cursor, &end, 10);
    if (end == *cursor || errno == ERANGE || value < INT_MIN || value > INT_MAX) {
        return 0;
    }
    *cursor = end;
    *out = (int)value;
    return 1;
}

static inline int gold_read_separator(const char **cursor, char expected) {
    if (**cursor != expected) {
        return 0;
    }
    (*cursor)++;
    return 1;
}

/* Parse "YYYY-MM-DDTHH:MM:SS[.ffffff][+HH:MM]" -> GtdTimestamp.
   Returns gtd_ts_none() on failure. An offset that fails to parse is left
   unapplied, as are fractional digits past the sixth. */
static inline GtdTimestamp gold_parse_timestamp(const char *text) {
    int year = 0;
    int month = 0;
    int day = 0;
    int hour = 0;
    int minute = 0;
    int second = 0;
    const char *cursor = text;

    if (!text || *text == '\0') {
        return gtd_ts_none();
    }
    if (!gold_read_int(&cursor, &year) || !gold_read_separator(&cursor, '-') ||
        !gold_read_int(&cursor, &month) || !gold_read_separator(&cursor, '-') ||
        !gold_read_int(&cursor, &day) || !gold_read_separator(&cursor, 'T') ||
        !gold_read_int(&cursor, &hour) || !gold_read_separator(&cursor, ':') ||
        !gold_read_int(&cursor, &minute) || !gold_read_separator(&cursor, ':') ||
        !gold_read_int(&cursor, &second)) {
        return gtd_ts_none();
    }

    /* Optional fractional seconds (".ffffff"), kept as microseconds. */
    long frac_us = 0;
    if (*cursor == '.') {
        cursor++;
        char digits[7] = "000000";
        int digit_count = 0;
        while (digit_count < 6 && *cursor >= '0' && *cursor <= '9') {
            digits[digit_count++] = *cursor++;
        }
        while (*cursor >= '0' && *cursor <= '9') { /* skip sub-microsecond digits */
            cursor++;
        }
        frac_us = strtol(digits, NULL, 10);
    }

    /* Optional timezone offset ("+HH:MM" / "-HH:MM"). */
    char sign = '+';
    long offset_secs = 0;
    if (*cursor == '+' || *cursor == '-') {
        int offset_hours = 0;
        int offset_minutes = 0;
        sign = *cursor;
        cursor++;
        if (gold_read_int(&cursor, &offset_hours) && gold_read_separator(&cursor, ':') &&
            gold_read_int(&cursor, &offset_minutes)) {
            offset_secs = (((long)offset_hours * 60L) + offset_minutes) * 60L;
        }
    }

    long days = 0;
    for (int y = 1970; y < year; y++) {
        days += gold_is_leap(y) ? 366 : 365;
    }
    for (int m = 1; m < month; m++) {
        days += gold_month_days(m, year);
    }
    days += day - 1;

    long secs = (days * 86400L) + (hour * 3600L) + (minute * 60L) + second;
    secs += (sign == '-') ? offset_secs : -offset_secs;

    return gtd_ts_from_micros(((uint64_t)secs * 1000000ULL) + (uint64_t)frac_us);
}

#endif /* GEOTRACE_GOLD_TIMESTAMP_H */
