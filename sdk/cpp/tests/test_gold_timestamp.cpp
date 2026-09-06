#include "../examples/gold_timestamp.hpp"

#include <doctest/doctest.h>

TEST_CASE("a date past 2038 parses to its microsecond count") {
    const auto timestamp = gold::parse_timestamp("2039-01-01T00:00:00+00:00");
    REQUIRE(timestamp.has_value());
    CHECK(timestamp->unix_micros == 2'177'452'800'000'000);
}
