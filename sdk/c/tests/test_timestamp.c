#include "../geotrace.h"
#include <criterion/criterion.h>
#include <inttypes.h>
#include <stddef.h>
#include <stdint.h>

Test(timestamp, every_unit_constructor_converts_a_count_to_its_microseconds) {
    GtdTimestamp seconds;
    cr_assert_eq(gtd_ts_from_seconds(1700000000, &seconds), GTD_OK);
    cr_assert_eq(seconds.unix_micros, 1700000000000000LL);

    GtdTimestamp millis;
    cr_assert_eq(gtd_ts_from_millis(1700000000123LL, &millis), GTD_OK);
    cr_assert_eq(millis.unix_micros, 1700000000123000LL);

    GtdTimestamp micros;
    cr_assert_eq(gtd_ts_from_micros(1700000000123456LL, &micros), GTD_OK);
    cr_assert_eq(micros.unix_micros, 1700000000123456LL);

    GtdTimestamp nanos;
    cr_assert_eq(gtd_ts_from_nanos(1700000000123456789LL, &nanos), GTD_OK);
    cr_assert_eq(nanos.unix_micros, 1700000000123456LL);
}

Test(timestamp, every_unit_constructor_converts_a_count_before_the_epoch) {
    GtdTimestamp seconds;
    cr_assert_eq(gtd_ts_from_seconds(-1700000000, &seconds), GTD_OK);
    cr_assert_eq(seconds.unix_micros, -1700000000000000LL);

    GtdTimestamp millis;
    cr_assert_eq(gtd_ts_from_millis(-1700000000123LL, &millis), GTD_OK);
    cr_assert_eq(millis.unix_micros, -1700000000123000LL);

    GtdTimestamp micros;
    cr_assert_eq(gtd_ts_from_micros(-1700000000123456LL, &micros), GTD_OK);
    cr_assert_eq(micros.unix_micros, -1700000000123456LL);

    GtdTimestamp nanos;
    cr_assert_eq(gtd_ts_from_nanos(-1700000000123456789LL, &nanos), GTD_OK);
    cr_assert_eq(nanos.unix_micros, -1700000000123456LL);
}

Test(timestamp, nanoseconds_truncate_towards_zero) {
    GtdTimestamp after_the_epoch;
    cr_assert_eq(gtd_ts_from_nanos(999, &after_the_epoch), GTD_OK);
    cr_assert_eq(after_the_epoch.unix_micros, 0);

    GtdTimestamp before_the_epoch;
    cr_assert_eq(gtd_ts_from_nanos(-999, &before_the_epoch), GTD_OK);
    cr_assert_eq(before_the_epoch.unix_micros, 0);
}

Test(timestamp, the_largest_nanosecond_count_converts) {
    GtdTimestamp timestamp;
    cr_assert_eq(gtd_ts_from_nanos(INT64_MAX, &timestamp), GTD_OK);
    cr_assert_eq(timestamp.unix_micros, 9223372036854775LL);
}

Test(timestamp, a_count_past_the_range_is_out_of_range) {
    GtdTimestamp timestamp = gtd_ts_none();

    cr_assert_eq(gtd_ts_from_seconds(INT64_MAX, &timestamp), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_ts_from_millis(INT64_MAX, &timestamp), GTD_ERR_OUT_OF_RANGE);
    cr_assert_eq(gtd_ts_from_micros(INT64_MAX, &timestamp), GTD_ERR_OUT_OF_RANGE);
    cr_assert_not_null(gtd_last_error());

    /* A rejected count leaves the caller's timestamp as it was. */
    cr_assert(gtd_ts_is_none(timestamp));
}

Test(timestamp, a_null_out_is_a_null_argument) {
    cr_assert_eq(gtd_ts_from_seconds(0, NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_ts_from_millis(0, NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_ts_from_micros(0, NULL), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_ts_from_nanos(0, NULL), GTD_ERR_NULL_ARGUMENT);
}

Test(timestamp, iso8601_parses_a_timestamp_to_its_microseconds) {
    static const struct {
        const char *text;
        int64_t unix_micros;
    } cases[] = {
        {"2026-02-01T15:00:00+00:00", 1769958000000000LL},
        {"2026-02-01T15:00:00Z", 1769958000000000LL},
        {"2026-02-01T15:00:00.123456+02:00", 1769950800123456LL},
        /* Digits past the sixth are truncated. */
        {"2026-02-01T15:00:00.1234567890-05:30", 1769977800123456LL},
        /* Past the range a 32-bit second count covers. */
        {"2039-01-01T00:00:00+00:00", 2177452800000000LL},
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        GtdTimestamp timestamp = gtd_ts_none();
        cr_assert_eq(gtd_ts_from_iso8601(cases[i].text, &timestamp), GTD_OK, "rejected %s",
                     cases[i].text);
        cr_assert_eq(timestamp.unix_micros, cases[i].unix_micros, "%s parsed to %" PRId64,
                     cases[i].text, timestamp.unix_micros);
    }
}

Test(timestamp, iso8601_parses_a_date_before_the_epoch) {
    static const struct {
        const char *text;
        int64_t unix_micros;
    } cases[] = {
        {"1969-12-31T23:59:59Z", -1000000LL},
        /* The fraction runs forwards from the second. */
        {"1969-12-31T23:59:59.123456Z", -876544LL},
        {"1965-03-04T05:06:07Z", -152391233000000LL},
        {"1968-02-29T12:00:00Z", -58017600000000LL},   /* a leap day */
        {"1900-01-01T00:00:00Z", -2208988800000000LL}, /* 1900 is no leap year */
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        GtdTimestamp timestamp = gtd_ts_none();
        cr_assert_eq(gtd_ts_from_iso8601(cases[i].text, &timestamp), GTD_OK, "rejected %s",
                     cases[i].text);
        cr_assert_eq(timestamp.unix_micros, cases[i].unix_micros, "%s parsed to %" PRId64,
                     cases[i].text, timestamp.unix_micros);
    }
}

Test(timestamp, iso8601_converts_a_leap_second_to_the_following_second) {
    GtdTimestamp leap_second = gtd_ts_none();
    cr_assert_eq(gtd_ts_from_iso8601("2024-06-01T12:00:60Z", &leap_second), GTD_OK);

    GtdTimestamp following_second = gtd_ts_none();
    cr_assert_eq(gtd_ts_from_iso8601("2024-06-01T12:01:00Z", &following_second), GTD_OK);

    cr_assert_eq(leap_second.unix_micros, following_second.unix_micros);
}

Test(timestamp, iso8601_rejects_a_string_that_is_not_a_timestamp) {
    static const char *const malformed[] = {
        "",
        "x",
        "2026-02",
        "2026/02/01T15:00:00Z",
        "2026-02-01T15:00:00+00:00extra",
        "2026-02-01T15:00:00", /* no timezone designator */
    };
    for (size_t i = 0; i < sizeof malformed / sizeof malformed[0]; i++) {
        GtdTimestamp timestamp = gtd_ts_none();
        cr_assert_eq(gtd_ts_from_iso8601(malformed[i], &timestamp), GTD_ERR_PARSE, "accepted %s",
                     malformed[i]);
        /* A rejected string leaves the caller's timestamp as it was. */
        cr_assert(gtd_ts_is_none(timestamp));
        cr_assert_not_null(gtd_last_error());
    }
}

Test(timestamp, iso8601_rejects_a_date_that_does_not_exist) {
    static const char *const nonexistent[] = {
        "2024-06-99T00:00:00Z", "2024-06-00T00:00:00Z",
        "2024-02-30T00:00:00Z", "2023-02-29T00:00:00Z", /* February 29 of a common year */
        "2024-13-01T00:00:00Z", "2024-00-01T00:00:00Z",
    };
    for (size_t i = 0; i < sizeof nonexistent / sizeof nonexistent[0]; i++) {
        GtdTimestamp timestamp = gtd_ts_none();
        cr_assert_eq(gtd_ts_from_iso8601(nonexistent[i], &timestamp), GTD_ERR_PARSE, "accepted %s",
                     nonexistent[i]);
    }
}

Test(timestamp, iso8601_rejects_a_time_of_day_outside_its_range) {
    static const char *const out_of_range[] = {
        "2024-06-01T24:00:00Z", /* the end-of-day hour ISO 8601 writes as 24 */
        "2024-06-01T12:60:00Z",
        "2024-06-01T12:00:61Z",
    };
    for (size_t i = 0; i < sizeof out_of_range / sizeof out_of_range[0]; i++) {
        GtdTimestamp timestamp = gtd_ts_none();
        cr_assert_eq(gtd_ts_from_iso8601(out_of_range[i], &timestamp), GTD_ERR_PARSE, "accepted %s",
                     out_of_range[i]);
    }
}

Test(timestamp, iso8601_rejects_a_year_past_the_range_a_timestamp_covers) {
    GtdTimestamp timestamp = gtd_ts_none();
    cr_assert_eq(gtd_ts_from_iso8601("+300000-01-01T00:00:00Z", &timestamp), GTD_ERR_PARSE);
}

Test(timestamp, iso8601_rejects_a_null_argument) {
    GtdTimestamp timestamp = gtd_ts_none();
    cr_assert_eq(gtd_ts_from_iso8601(NULL, &timestamp), GTD_ERR_NULL_ARGUMENT);
    cr_assert_eq(gtd_ts_from_iso8601("2026-02-01T15:00:00Z", NULL), GTD_ERR_NULL_ARGUMENT);
}
