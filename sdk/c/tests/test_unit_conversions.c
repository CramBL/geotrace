#include "../geotrace.h"
#include <criterion/criterion.h>

/* `23.2 / 3.6` is 6.444444444444444 and `13.0 * 1852.0 / 3600.0` is 6.687777777777778. */
Test(unit_conversions, km_per_h_and_knots_convert_to_the_same_m_per_s_as_in_the_rust_sdk) {
    cr_assert_float_eq(gtd_mps_from_kmh(23.2), 6.444444444444445, 0.0);
    cr_assert_float_eq(gtd_mps_from_knots(13.0), 6.687777777777779, 0.0);
}

/* `6.444444444444445 * 3.6` is 23.200000000000003 and `6.687777777777779 * 3600.0 / 1852.0`
   is 13.000000000000002. */
Test(unit_conversions, converting_back_from_m_per_s_restores_the_km_per_h_and_the_knots) {
    cr_assert_float_eq(gtd_kmh_from_mps(6.444444444444445), 23.2, 0.0);
    cr_assert_float_eq(gtd_knots_from_mps(6.687777777777779), 13.0, 0.0);
}

/* `0.1 * 180.0 / pi` is 5.729577951308232 and `3.0 / (180.0 / pi)` is 0.05235987755982988. */
Test(unit_conversions, radians_and_degrees_convert_to_the_same_value_as_in_the_rust_sdk) {
    cr_assert_float_eq(gtd_degrees_from_radians(0.1), 5.729577951308233, 0.0);
    cr_assert_float_eq(gtd_radians_from_degrees(3.0), 0.05235987755982989, 0.0);
}
