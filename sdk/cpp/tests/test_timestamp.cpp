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

TEST_CASE("from_iso8601 parses a timestamp to its microseconds") {
    CHECK(Timestamp::from_iso8601("2026-02-01T15:00:00+00:00").unix_micros ==
          1'769'958'000'000'000);
    CHECK(Timestamp::from_iso8601("2026-02-01T15:00:00.123456Z").unix_micros ==
          1'769'958'000'123'456);
}

TEST_CASE("from_iso8601 parses a date before the epoch") {
    CHECK(Timestamp::from_iso8601("1969-12-31T23:59:59Z").unix_micros == -1'000'000);
    // A leap day, and 1900, which is no leap year.
    CHECK(Timestamp::from_iso8601("1968-02-29T12:00:00Z").unix_micros == -58'017'600'000'000);
    CHECK(Timestamp::from_iso8601("1900-01-01T00:00:00Z").unix_micros == -2'208'988'800'000'000);
}

TEST_CASE("try_from_iso8601 returns a parse error for a date that does not exist") {
    const Result<Timestamp> result = Timestamp::try_from_iso8601("2024-06-99T00:00:00Z");
    REQUIRE(result.is_err());
    CHECK(result.error().code == GTD_ERR_PARSE);
    CHECK_FALSE(result.error().description.empty());
}

TEST_CASE("try_from_iso8601 returns a parse error for a timestamp with no timezone designator") {
    CHECK(Timestamp::try_from_iso8601("2026-02-01T15:00:00").is_err());
}

TEST_CASE("from_iso8601 throws for a string that is not a timestamp") {
    CHECK_THROWS_AS(static_cast<void>(Timestamp::from_iso8601("x")), geotrace::ParseError);
}
