#include "../geotrace.h"
#include <criterion/criterion.h>
#include <stddef.h>
#include <stdint.h>

/* The cases of `the_band_is_half_a_db_wide_either_side` in the Rust SDK's `snr.rs`. */
Test(snr, a_reading_classifies_as_in_the_rust_sdk) {
    static const struct {
        float snr_dbhz;
        uint8_t no_data;
    } cases[] = {
        {99.0F, 1}, {99.4F, 1}, {98.5F, 0}, {99.5F, 0}, {40.0F, 0},
    };
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        cr_assert_eq(gtd_snr_is_no_data_sentinel(cases[i].snr_dbhz), cases[i].no_data, "%.1f dB-Hz",
                     (double)cases[i].snr_dbhz);
    }
}
