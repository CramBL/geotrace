#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

using geotrace::Angle;
using geotrace::Velocity;

static_assert(noexcept(Velocity::kmh(1.0).as_knots()));
static_assert(noexcept(Velocity::knots(1.0).as_kmh()));
static_assert(noexcept(Angle::radians(1.0).as_radians()));

// `23.2 / 3.6` is 6.444444444444444, `13.0 * 1852.0 / 3600.0` is 6.687777777777778,
// `6.444444444444445 * 3.6` is 23.200000000000003 and `6.687777777777779 * 3600.0 / 1852.0`
// is 13.000000000000002.
TEST_CASE("Velocity converts to the same values as the Rust SDK") {
    CHECK(Velocity::kmh(23.2).as_mps() == 6.444444444444445);
    CHECK(Velocity::knots(13.0).as_mps() == 6.687777777777779);
    CHECK(Velocity::mps(6.444444444444445).as_kmh() == 23.2);
    CHECK(Velocity::mps(6.687777777777779).as_knots() == 13.0);
}

TEST_CASE("Velocity::kMpsPerKmh and kMpsPerKnot are the factors kmh and knots convert with") {
    CHECK(Velocity::kmh(23.2).as_mps() == 23.2 * Velocity::kMpsPerKmh);
    CHECK(Velocity::knots(13.0).as_mps() == 13.0 * Velocity::kMpsPerKnot);
}

// `0.1 * 180.0 / pi` is 5.729577951308232 and `3.0 / (180.0 / pi)` is 0.05235987755982988.
TEST_CASE("Angle converts to the same values as the Rust SDK") {
    CHECK(Angle::radians(0.1).as_degrees() == 5.729577951308233);
    CHECK(Angle::degrees(3.0).as_radians() == 0.05235987755982989);
}
