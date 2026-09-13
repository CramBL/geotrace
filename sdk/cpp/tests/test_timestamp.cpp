#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <limits>
#include <stdexcept>
#include <string>

using geotrace::Angle;
using geotrace::Annotation;
using geotrace::Channel;
using geotrace::EventMarker;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Result;
using geotrace::SatelliteReport;
using geotrace::Timestamp;

constexpr Timestamp FIX_TIME{1'700'000'000'000'000};
constexpr Timestamp INT64_MIN_MICROS{std::numeric_limits<std::int64_t>::min()};
constexpr const char *INT64_MIN_MICROS_PAST_THE_RANGE_MESSAGE =
    "-9223372036854775808 microseconds since the Unix epoch is past the range a UTC timestamp "
    "covers";

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

TEST_CASE("FileBuilder throws std::out_of_range for a Timestamp of INT64_MIN microseconds") {
    FileBuilder builder;
    std::string argument_name;

    SUBCASE("the receiver time of a nav fix") {
        argument_name = "gps_time";
        const NavFix fix{FixTime::receiver(INT64_MIN_MICROS), Angle::degrees(51.5),
                         Angle::degrees(-0.1)};
        CHECK_THROWS_AS(builder.add_nav_fix(fix), std::out_of_range);
    }
    SUBCASE("the host time of a nav fix beside a valid receiver time") {
        argument_name = "sys_time";
        const NavFix fix{FixTime::both(FIX_TIME, INT64_MIN_MICROS), Angle::degrees(51.5),
                         Angle::degrees(-0.1)};
        CHECK_THROWS_AS(builder.add_nav_fix(fix), std::out_of_range);
    }
    SUBCASE("the receiver time of a satellite report beside a valid host time") {
        argument_name = "gps_time";
        const SatelliteReport report{FixTime::both(INT64_MIN_MICROS, FIX_TIME), {}};
        CHECK_THROWS_AS(builder.add_satellite_report(report), std::out_of_range);
    }
    SUBCASE("the host time of a satellite report beside a valid receiver time") {
        argument_name = "sys_time";
        const SatelliteReport report{FixTime::both(FIX_TIME, INT64_MIN_MICROS), {}};
        CHECK_THROWS_AS(builder.add_satellite_report(report), std::out_of_range);
    }
    SUBCASE("the time of an annotation") {
        argument_name = "time";
        CHECK_THROWS_AS(builder.add_annotation(Annotation{INT64_MIN_MICROS}), std::out_of_range);
    }
    SUBCASE("the time of an event marker") {
        argument_name = "sys_time";
        CHECK_THROWS_AS(builder.add_event_marker(EventMarker{"power/boot", INT64_MIN_MICROS}),
                        std::out_of_range);
    }
    SUBCASE("the time of a channel sample") {
        argument_name = "times[1]";
        Channel channel{};
        channel.name = "temperature";
        channel.times = {FIX_TIME, INT64_MIN_MICROS};
        channel.values = {20.0, 21.0};
        CHECK_THROWS_AS(builder.add_channel(channel), std::out_of_range);
    }

    CHECK(builder.status().code == GTD_ERR_OUT_OF_RANGE);
    CHECK(builder.status().description ==
          argument_name + ": " + INT64_MIN_MICROS_PAST_THE_RANGE_MESSAGE);
}

TEST_CASE("a nav fix before the epoch keeps both its timestamps through a write and a read") {
    const Timestamp gps_time = Timestamp::from_seconds(-1'700'000'000);
    const Timestamp sys_time = Timestamp::from_micros(-1'700'000'000'123'456);
    const NavFix fix{FixTime::both(gps_time, sys_time), Angle::degrees(51.5), Angle::degrees(-0.1)};

    const NavFile file = NavFile::from_bytes(FileBuilder{}.add_nav_fix(fix).finish().to_bytes());

    const auto point = file.nav_point(0);
    REQUIRE(point.gps_time.has_value());
    REQUIRE(point.sys_time.has_value());
    CHECK(point.gps_time->unix_micros == gps_time.unix_micros);
    CHECK(point.sys_time->unix_micros == sys_time.unix_micros);
}
