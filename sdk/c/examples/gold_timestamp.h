/**
 * Timestamp parsing for the gold dataset CSV fixtures, shared by the C gold
 * dataset example and sdk/c/tests/test_gold_timestamp.c.
 */

#ifndef GEOTRACE_GOLD_TIMESTAMP_H
#define GEOTRACE_GOLD_TIMESTAMP_H

#include "../geotrace.h"

#include <errno.h>
#include <inttypes.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    int32_t year;
    int32_t month;
} GoldYearMonth;

static inline int gold_is_leap(int32_t year) {
    return (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
}

static inline int32_t gold_days_in_month(GoldYearMonth date) {
    static const int32_t days_per_month[12] = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return (date.month == 2 && gold_is_leap(date.year)) ? 29 : days_per_month[date.month - 1];
}

/* Reads a decimal integer at *cursor and advances it past the digits, rejecting
   a missing number and one outside int32_t range. */
static inline int gold_read_int32(const char **cursor, int32_t *out) {
    char *end = NULL;

    errno = 0;
    const intmax_t value = strtoimax(*cursor, &end, 10);
    if (end == *cursor || errno == ERANGE || value < INT32_MIN || value > INT32_MAX) {
        return 0;
    }
    *cursor = end;
    *out = (int32_t)value;
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
    int32_t year = 0;
    int32_t month = 0;
    int32_t day = 0;
    int32_t hour = 0;
    int32_t minute = 0;
    int32_t second = 0;
    const char *cursor = text;

    if (!text || *text == '\0') {
        return gtd_ts_none();
    }
    if (!gold_read_int32(&cursor, &year) || !gold_read_separator(&cursor, '-') ||
        !gold_read_int32(&cursor, &month) || !gold_read_separator(&cursor, '-') ||
        !gold_read_int32(&cursor, &day) || !gold_read_separator(&cursor, 'T') ||
        !gold_read_int32(&cursor, &hour) || !gold_read_separator(&cursor, ':') ||
        !gold_read_int32(&cursor, &minute) || !gold_read_separator(&cursor, ':') ||
        !gold_read_int32(&cursor, &second)) {
        return gtd_ts_none();
    }
    if (month < 1 || month > 12) {
        return gtd_ts_none();
    }

    /* Optional fractional seconds (".ffffff"), kept as microseconds. */
    int32_t frac_us = 0;
    if (*cursor == '.') {
        cursor++;
        char digits[7] = "000000";
        int32_t digit_count = 0;
        while (digit_count < 6 && *cursor >= '0' && *cursor <= '9') {
            digits[digit_count++] = *cursor++;
        }
        while (*cursor >= '0' && *cursor <= '9') { /* skip sub-microsecond digits */
            cursor++;
        }
        frac_us = (int32_t)strtoimax(digits, NULL, 10);
    }

    /* Optional timezone offset ("+HH:MM" / "-HH:MM"). */
    char sign = '+';
    int64_t offset_secs = 0;
    if (*cursor == '+' || *cursor == '-') {
        int32_t offset_hours = 0;
        int32_t offset_minutes = 0;
        sign = *cursor;
        cursor++;
        if (gold_read_int32(&cursor, &offset_hours) && gold_read_separator(&cursor, ':') &&
            gold_read_int32(&cursor, &offset_minutes)) {
            offset_secs = (((int64_t)offset_hours * INT64_C(60)) + offset_minutes) * INT64_C(60);
        }
    }

    int64_t days = 0;
    for (int32_t y = 1970; y < year; y++) {
        days += gold_is_leap(y) ? 366 : 365;
    }
    for (int32_t m = 1; m < month; m++) {
        days += gold_days_in_month((GoldYearMonth){.year = year, .month = m});
    }
    days += day - 1;

    int64_t secs =
        (days * INT64_C(86400)) + (hour * INT64_C(3600)) + (minute * INT64_C(60)) + second;
    secs += (sign == '-') ? offset_secs : -offset_secs;

    /* `gtd_ts_from_seconds` rejects a year past the range a timestamp covers,
       whose microsecond count would overflow int64_t. */
    GtdTimestamp whole_seconds;
    if (gtd_ts_from_seconds(secs, &whole_seconds) != GTD_OK) {
        return gtd_ts_none();
    }
    GtdTimestamp timestamp;
    if (gtd_ts_from_micros(whole_seconds.unix_micros + frac_us, &timestamp) != GTD_OK) {
        return gtd_ts_none();
    }
    return timestamp;
}

#endif /* GEOTRACE_GOLD_TIMESTAMP_H */
