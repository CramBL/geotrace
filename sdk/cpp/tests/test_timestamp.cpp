#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <limits>
#include <stdexcept>
#include <type_traits>
#include <utility>

using geotrace::Angle;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Result;
using geotrace::Timestamp;

namespace {

template <typename T, typename = void> struct has_assignable_unix_micros : std::false_type {};

template <typename T>
struct has_assignable_unix_micros<
    T, std::void_t<decltype(std::declval<T &>().unix_micros = std::int64_t{})>> : std::true_type {};

} // namespace

constexpr const char *INT64_MIN_MICROS_PAST_THE_RANGE_MESSAGE =
    "-9223372036854775808 microseconds since the Unix epoch is past the range a UTC timestamp "
    "covers";

static_assert(!std::is_constructible_v<Timestamp, std::int64_t>);
// `GtdTimestamp` has a public `unix_micros`, the field the trait detects.
static_assert(has_assignable_unix_micros<GtdTimestamp>::value);
static_assert(!has_assignable_unix_micros<Timestamp>::value);

TEST_CASE("every unit constructor converts a count to its microseconds") {
    CHECK(Timestamp::from_seconds(1'700'000'000).as_unix_micros() == 1'700'000'000'000'000);
    CHECK(Timestamp::from_millis(1'700'000'000'123).as_unix_micros() == 1'700'000'000'123'000);
    CHECK(Timestamp::from_micros(1'700'000'000'123'456).as_unix_micros() == 1'700'000'000'123'456);
    CHECK(Timestamp::from_nanos(1'700'000'000'123'456'789).as_unix_micros() ==
          1'700'000'000'123'456);
}

TEST_CASE("a count before the epoch converts") {
    CHECK(Timestamp::from_seconds(-1'700'000'000).as_unix_micros() == -1'700'000'000'000'000);
    CHECK(Timestamp::from_nanos(-1'700'000'000'123'456'789).as_unix_micros() ==
          -1'700'000'000'123'456);
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

TEST_CASE("try_from_micros returns an out-of-range error for INT64_MIN microseconds") {
    const Result<Timestamp> result =
        Timestamp::try_from_micros(std::numeric_limits<std::int64_t>::min());
    REQUIRE(result.is_err());
    CHECK(result.error().code == GTD_ERR_OUT_OF_RANGE);
    CHECK(result.error().description == INT64_MIN_MICROS_PAST_THE_RANGE_MESSAGE);
}

TEST_CASE("from_micros throws std::out_of_range for INT64_MIN microseconds") {
    CHECK_THROWS_AS(
        static_cast<void>(Timestamp::from_micros(std::numeric_limits<std::int64_t>::min())),
        std::out_of_range);
}

TEST_CASE("from_iso8601 parses a timestamp to its microseconds") {
    CHECK(Timestamp::from_iso8601("2026-02-01T15:00:00+00:00").as_unix_micros() ==
          1'769'958'000'000'000);
    CHECK(Timestamp::from_iso8601("2026-02-01T15:00:00.123456Z").as_unix_micros() ==
          1'769'958'000'123'456);
}

TEST_CASE("from_iso8601 parses a date before the epoch") {
    CHECK(Timestamp::from_iso8601("1969-12-31T23:59:59Z").as_unix_micros() == -1'000'000);
    // A leap day, and 1900, which is no leap year.
    CHECK(Timestamp::from_iso8601("1968-02-29T12:00:00Z").as_unix_micros() == -58'017'600'000'000);
    CHECK(Timestamp::from_iso8601("1900-01-01T00:00:00Z").as_unix_micros() ==
          -2'208'988'800'000'000);
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

TEST_CASE("a nav fix before the epoch keeps both its timestamps through a write and a read") {
    const Timestamp gps_time = Timestamp::from_seconds(-1'700'000'000);
    const Timestamp sys_time = Timestamp::from_micros(-1'700'000'000'123'456);
    const NavFix fix{FixTime::both(gps_time, sys_time), Angle::degrees(51.5), Angle::degrees(-0.1)};

    const NavFile file = NavFile::from_bytes(FileBuilder{}.add_nav_fix(fix).finish().to_bytes());

    const auto point = file.nav_point(0);
    REQUIRE(point.gps_time.has_value());
    REQUIRE(point.sys_time.has_value());
    CHECK(point.gps_time->as_unix_micros() == gps_time.as_unix_micros());
    CHECK(point.sys_time->as_unix_micros() == sys_time.as_unix_micros());
}
