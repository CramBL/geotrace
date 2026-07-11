#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <stdexcept>
#include <utility>
#include <vector>

using geotrace::Angle;
using geotrace::Error;
using geotrace::FileBuilder;
using geotrace::IoError;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Timestamp;

static NavFile make_minimal() {
    NavFix fix{};
    fix.gps_time = Timestamp::from_seconds(1700000000ULL);
    fix.lat = Angle::degrees(51.5074);
    fix.lon = Angle::degrees(-0.1278);
    return FileBuilder{}.add_nav_fix(fix).finish();
}

TEST_CASE("NavFile: open on non-existent path throws IoError") {
    CHECK_THROWS_AS(NavFile::open("/nonexistent/path/that/does/not/exist.gtd"), IoError);
}

TEST_CASE("NavFile: nav_point out-of-range throws std::out_of_range") {
    auto file = make_minimal();
    CHECK_THROWS_AS(file.nav_point(9999), std::out_of_range);
}

TEST_CASE("NavFile: satellite out-of-range throws std::out_of_range") {
    auto file = make_minimal();
    CHECK_THROWS_AS(file.satellite(0, 0), std::out_of_range);    // no satellite report
    CHECK_THROWS_AS(file.satellite(9999, 0), std::out_of_range); // nav idx out of range
}

TEST_CASE("NavFile: event_marker out-of-range throws std::out_of_range") {
    auto file = make_minimal();
    CHECK_THROWS_AS(file.event_marker(0), std::out_of_range);
}

TEST_CASE("NavFile: absent metadata returns empty string_view") {
    auto file = make_minimal();
    CHECK(file.title() == "");
    CHECK(file.device() == "");
    CHECK(file.notes() == "");
    CHECK(file.identity() == "");
    CHECK(file.travel_mode() == "");
    CHECK(file.title().empty());
}

TEST_CASE("NavFile: nav_point_count returns correct value") {
    NavFix f1{};
    f1.gps_time = Timestamp::from_seconds(1700000000ULL);
    f1.lat = Angle::degrees(0.0);
    f1.lon = Angle::degrees(0.0);

    NavFix f2{};
    f2.gps_time = Timestamp::from_seconds(1700000010ULL);
    f2.lat = Angle::degrees(0.1);
    f2.lon = Angle::degrees(0.1);

    NavFix f3{};
    f3.gps_time = Timestamp::from_seconds(1700000020ULL);
    f3.lat = Angle::degrees(0.2);
    f3.lon = Angle::degrees(0.2);

    auto file = FileBuilder{}.add_nav_fix(f1).add_nav_fix(f2).add_nav_fix(f3).finish();

    CHECK(file.nav_point_count() == 3);
}

TEST_CASE("NavFile: move semantics work") {
    auto f1 = make_minimal();
    auto f2 = std::move(f1);
    CHECK(f2.nav_point_count() == 1);
}

TEST_CASE("NavFile: from_bytes with invalid data throws") {
    const std::vector<std::uint8_t> garbage = {0x00, 0xFF, 0xAB, 0xCD};
    CHECK_THROWS_AS(NavFile::from_bytes(garbage), Error);
}

#ifdef GTD_FIXTURE_PATH
TEST_CASE("NavFile: open fixture file succeeds") {
    auto file = NavFile::open(GTD_FIXTURE_PATH);
    CHECK(file.nav_point_count() >= 1);
    CHECK(file.title() == "minimal fixture");
    CHECK(file.device() == "gen_fixture");

    auto p0 = file.nav_point(0);
    CHECK(p0.lat.as_degrees() == doctest::Approx(51.5074).epsilon(1e-4));
    CHECK(p0.satellite_count == 2);
}
#endif
