#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <limits>
#include <stdexcept>

using geotrace::Result;
using geotrace::Timestamp;

TEST_CASE("every unit constructor converts a count to its microseconds") {
    CHECK(Timestamp::from_seconds(1'700'000'000).unix_micros == 1'700'000'000'000'000);
    CHECK(Timestamp::from_millis(1'700'000'000'123).unix_micros == 1'700'000'000'123'000);
    CHECK(Timestamp::from_micros(1'700'000'000'123'456).unix_micros == 1'700'000'000'123'456);
    CHECK(Timestamp::from_nanos(1'700'000'000'123'456'789).unix_micros == 1'700'000'000'123'456);
}

TEST_CASE("a count before the epoch converts") {
    CHECK(Timestamp::from_seconds(-1'700'000'000).unix_micros == -1'700'000'000'000'000);
    CHECK(Timestamp::from_nanos(-1'700'000'000'123'456'789).unix_micros == -1'700'000'000'123'456);
}

TEST_CASE("try_from_seconds returns an error for a count past the range") {
    const Result<Timestamp> result =
        Timestamp::try_from_seconds(std::numeric_limits<std::int64_t>::max());
    REQUIRE(result.is_err());
    CHECK(result.error().code == GTD_ERR_OUT_OF_RANGE);
    CHECK_FALSE(result.error().description.empty());
}

TEST_CASE("from_seconds throws for a count past the range") {
    CHECK_THROWS_AS(
        static_cast<void>(Timestamp::from_seconds(std::numeric_limits<std::int64_t>::max())),
        std::out_of_range);
}
