#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <type_traits>

using geotrace::Angle;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::RecordedFixTimestamps;
using geotrace::SatelliteReport;
using geotrace::Timestamp;

constexpr Timestamp GPS{1'700'000'000'000'000};
constexpr Timestamp SYS{1'700'000'002'000'000};

static_assert(!std::is_default_constructible_v<Timestamp>);
static_assert(!std::is_default_constructible_v<FixTime>);
static_assert(!std::is_default_constructible_v<NavFix>);
static_assert(!std::is_default_constructible_v<SatelliteReport>);

constexpr FixTime BOTH_CLOCKS = FixTime::both(GPS, SYS);
static_assert(BOTH_CLOCKS.gps_time()->unix_micros == GPS.unix_micros);
static_assert(BOTH_CLOCKS.sys_time()->unix_micros == SYS.unix_micros);

TEST_CASE("FixTime reports the clock it was built from") {
    SUBCASE("receiver") {
        const FixTime time = FixTime::receiver(GPS);
        REQUIRE(time.gps_time().has_value());
        CHECK(time.gps_time()->unix_micros == GPS.unix_micros);
        CHECK_FALSE(time.sys_time().has_value());
    }
    SUBCASE("host") {
        const FixTime time = FixTime::host(SYS);
        CHECK_FALSE(time.gps_time().has_value());
        REQUIRE(time.sys_time().has_value());
        CHECK(time.sys_time()->unix_micros == SYS.unix_micros);
    }
    SUBCASE("both") {
        const FixTime time = FixTime::both(GPS, SYS);
        REQUIRE(time.gps_time().has_value());
        REQUIRE(time.sys_time().has_value());
        CHECK(time.gps_time()->unix_micros == GPS.unix_micros);
        CHECK(time.sys_time()->unix_micros == SYS.unix_micros);
    }
}

TEST_CASE("FixTime::from_recorded takes the clocks the recorder holds") {
    RecordedFixTimestamps recorded{};

    SUBCASE("both") {
        recorded.gps_time = GPS;
        recorded.sys_time = SYS;
        const auto time = FixTime::from_recorded(recorded);
        REQUIRE(time.has_value());
        CHECK(time->gps_time()->unix_micros == GPS.unix_micros);
        CHECK(time->sys_time()->unix_micros == SYS.unix_micros);
    }
    SUBCASE("receiver only") {
        recorded.gps_time = GPS;
        const auto time = FixTime::from_recorded(recorded);
        REQUIRE(time.has_value());
        CHECK(time->gps_time()->unix_micros == GPS.unix_micros);
        CHECK_FALSE(time->sys_time().has_value());
    }
    SUBCASE("host only") {
        recorded.sys_time = SYS;
        const auto time = FixTime::from_recorded(recorded);
        REQUIRE(time.has_value());
        CHECK_FALSE(time->gps_time().has_value());
        CHECK(time->sys_time()->unix_micros == SYS.unix_micros);
    }
    SUBCASE("neither") {
        CHECK_FALSE(FixTime::from_recorded(recorded).has_value());
    }
}

TEST_CASE("a nav point keeps its two clocks apart through a write and a read") {
    SUBCASE("host only") {
        const NavFix fix{FixTime::host(SYS), Angle::degrees(51.5), Angle::degrees(-0.1)};
        const NavFile file =
            NavFile::from_bytes(FileBuilder{}.add_nav_fix(fix).finish().to_bytes());

        const auto point = file.nav_point(0);
        CHECK_FALSE(point.gps_time.has_value());
        REQUIRE(point.sys_time.has_value());
        CHECK(point.sys_time->unix_micros == SYS.unix_micros);
    }
    SUBCASE("both") {
        const NavFix fix{FixTime::both(GPS, SYS), Angle::degrees(51.5), Angle::degrees(-0.1)};
        const NavFile file =
            NavFile::from_bytes(FileBuilder{}.add_nav_fix(fix).finish().to_bytes());

        const auto point = file.nav_point(0);
        REQUIRE(point.gps_time.has_value());
        REQUIRE(point.sys_time.has_value());
        CHECK(point.gps_time->unix_micros == GPS.unix_micros);
        CHECK(point.sys_time->unix_micros == SYS.unix_micros);
    }
}
