#include "../geotrace.h"
#include <criterion/criterion.h>
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
