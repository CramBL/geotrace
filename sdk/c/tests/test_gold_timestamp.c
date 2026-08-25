#include "../examples/gold_timestamp.h"

#include <criterion/criterion.h>
#include <stddef.h>

Test(gold_timestamp, parses_fraction_and_offset) {
    cr_assert_eq(gold_parse_timestamp("2026-02-01T15:00:00+00:00").unix_micros, 1769958000000000);
    cr_assert_eq(gold_parse_timestamp("2026-02-01T15:00:00.123456+02:00").unix_micros,
                 1769950800123456);
}

Test(gold_timestamp, truncates_digits_past_microseconds) {
    cr_assert_eq(gold_parse_timestamp("2026-02-01T15:00:00.1234567890-05:30").unix_micros,
                 1769977800123456);
}

Test(gold_timestamp, rejects_malformed_input) {
    static const char *const malformed[] = {
        "",                    /* empty */
        "2026-02",             /* truncated */
        "2026-02-01 15:00:00", /* space instead of 'T' */
        "2026/02/01T15:00:00", /* wrong separator */
        "x",
    };
    for (size_t i = 0; i < sizeof(malformed) / sizeof(malformed[0]); i++)
        cr_assert(gtd_ts_is_none(gold_parse_timestamp(malformed[i])), "accepted %s", malformed[i]);
}

/* One past each end of int, which strtoll itself still converts. The negative
   case sits on the seconds field: a year that far below INT_MIN wraps to
   INT_MAX, which walks the day-count loop through two billion iterations. */
Test(gold_timestamp, rejects_component_wider_than_int) {
    cr_assert(gtd_ts_is_none(gold_parse_timestamp("2147483648-02-01T15:00:00")));
    cr_assert(gtd_ts_is_none(gold_parse_timestamp("2026-02-01T15:00:-2147483649")));
}

Test(gold_timestamp, rejects_component_wider_than_long_long) {
    cr_assert(gtd_ts_is_none(gold_parse_timestamp("99999999999999999999-02-01T15:00:00")));
    cr_assert(gtd_ts_is_none(gold_parse_timestamp("2026-99999999999999999999-01T15:00:00")));
}

Test(gold_timestamp, leaves_unparsable_offset_unapplied) {
    cr_assert_eq(gold_parse_timestamp("2026-02-01T15:00:00+2147483648:00").unix_micros,
                 gold_parse_timestamp("2026-02-01T15:00:00").unix_micros);
}
