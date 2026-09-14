#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <chrono>
#include <type_traits>

#include "test_timestamps.hpp"

using geotrace::Angle;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::RecordedFixTimestamps;
using geotrace::SatelliteReport;
using geotrace::Timestamp;

static_assert(!std::is_default_constructible_v<Timestamp>);
static_assert(!std::is_default_constructible_v<FixTime>);
static_assert(!std::is_default_constructible_v<NavFix>);
static_assert(!std::is_default_constructible_v<SatelliteReport>);

TEST_CASE("FixTime reports the clock it was built from") {
    SUBCASE("receiver") {
        const FixTime time = FixTime::receiver(fix_timestamp());
        REQUIRE(time.gps_time().has_value());
        CHECK(time.gps_time()->as_unix_micros() == fix_timestamp().as_unix_micros());
        CHECK_FALSE(time.sys_time().has_value());
    }
    SUBCASE("host") {
        const FixTime time = FixTime::host(after_fix_timestamp(std::chrono::seconds{2}));
        CHECK_FALSE(time.gps_time().has_value());
        REQUIRE(time.sys_time().has_value());
        CHECK(time.sys_time()->as_unix_micros() ==
              after_fix_timestamp(std::chrono::seconds{2}).as_unix_micros());
    }
    SUBCASE("both") {
        const FixTime time =
            FixTime::both(fix_timestamp(), after_fix_timestamp(std::chrono::seconds{2}));
        REQUIRE(time.gps_time().has_value());
        REQUIRE(time.sys_time().has_value());
        CHECK(time.gps_time()->as_unix_micros() == fix_timestamp().as_unix_micros());
        CHECK(time.sys_time()->as_unix_micros() ==
              after_fix_timestamp(std::chrono::seconds{2}).as_unix_micros());
    }
}

TEST_CASE("FixTime::from_recorded takes the clocks the recorder holds") {
    RecordedFixTimestamps recorded{};

    SUBCASE("both") {
        recorded.gps_time = fix_timestamp();
        recorded.sys_time = after_fix_timestamp(std::chrono::seconds{2});
        const auto time = FixTime::from_recorded(recorded);
        REQUIRE(time.has_value());
        CHECK(time->gps_time()->as_unix_micros() == fix_timestamp().as_unix_micros());
        CHECK(time->sys_time()->as_unix_micros() ==
              after_fix_timestamp(std::chrono::seconds{2}).as_unix_micros());
    }
    SUBCASE("receiver only") {
        recorded.gps_time = fix_timestamp();
        const auto time = FixTime::from_recorded(recorded);
        REQUIRE(time.has_value());
        CHECK(time->gps_time()->as_unix_micros() == fix_timestamp().as_unix_micros());
        CHECK_FALSE(time->sys_time().has_value());
    }
    SUBCASE("host only") {
        recorded.sys_time = after_fix_timestamp(std::chrono::seconds{2});
        const auto time = FixTime::from_recorded(recorded);
        REQUIRE(time.has_value());
        CHECK_FALSE(time->gps_time().has_value());
        CHECK(time->sys_time()->as_unix_micros() ==
              after_fix_timestamp(std::chrono::seconds{2}).as_unix_micros());
    }
    SUBCASE("neither") {
        CHECK_FALSE(FixTime::from_recorded(recorded).has_value());
    }
}

TEST_CASE("a nav point keeps its two clocks apart through a write and a read") {
    SUBCASE("host only") {
        const NavFix fix{FixTime::host(after_fix_timestamp(std::chrono::seconds{2})),
                         Angle::degrees(51.5), Angle::degrees(-0.1)};
        const NavFile file =
            NavFile::from_bytes(FileBuilder{}.add_nav_fix(fix).finish().to_bytes());

        const auto point = file.nav_point(0);
        CHECK_FALSE(point.gps_time.has_value());
        REQUIRE(point.sys_time.has_value());
        CHECK(point.sys_time->as_unix_micros() ==
              after_fix_timestamp(std::chrono::seconds{2}).as_unix_micros());
    }
    SUBCASE("both") {
        const NavFix fix{
            FixTime::both(fix_timestamp(), after_fix_timestamp(std::chrono::seconds{2})),
            Angle::degrees(51.5), Angle::degrees(-0.1)};
        const NavFile file =
            NavFile::from_bytes(FileBuilder{}.add_nav_fix(fix).finish().to_bytes());

        const auto point = file.nav_point(0);
        REQUIRE(point.gps_time.has_value());
        REQUIRE(point.sys_time.has_value());
        CHECK(point.gps_time->as_unix_micros() == fix_timestamp().as_unix_micros());
        CHECK(point.sys_time->as_unix_micros() ==
              after_fix_timestamp(std::chrono::seconds{2}).as_unix_micros());
    }
}
