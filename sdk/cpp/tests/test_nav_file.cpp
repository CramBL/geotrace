#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <stdexcept>
#include <utility>
#include <vector>

using geotrace::Angle;
using geotrace::Error;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::IoError;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Timestamp;

static NavFile make_minimal() {
    const NavFix fix{FixTime::receiver(Timestamp::from_seconds(1700000000)),
                     Angle::degrees(51.5074), Angle::degrees(-0.1278)};
    return FileBuilder{}.add_nav_fix(fix).finish();
}

TEST_CASE("NavFile: open on non-existent path throws IoError") {
    CHECK_THROWS_AS(static_cast<void>(NavFile::open("/nonexistent/path/that/does/not/exist.gtd")),
                    IoError);
}

TEST_CASE("NavFile: nav_point out-of-range throws std::out_of_range") {
    auto file = make_minimal();
    CHECK_THROWS_AS(static_cast<void>(file.nav_point(9999)), std::out_of_range);
}

TEST_CASE("NavFile: satellite out-of-range throws std::out_of_range") {
    auto file = make_minimal();
    CHECK_THROWS_AS(static_cast<void>(file.satellite(0, 0)),
                    std::out_of_range); // no satellite report
    CHECK_THROWS_AS(static_cast<void>(file.satellite(9999, 0)),
                    std::out_of_range); // nav index out of range
}

TEST_CASE("NavFile: event_marker out-of-range throws std::out_of_range") {
    auto file = make_minimal();
    CHECK_THROWS_AS(static_cast<void>(file.event_marker(0)), std::out_of_range);
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
    const NavFix first_fix{FixTime::receiver(Timestamp::from_seconds(1700000000)),
                           Angle::degrees(0.0), Angle::degrees(0.0)};
    const NavFix second_fix{FixTime::receiver(Timestamp::from_seconds(1700000010)),
                            Angle::degrees(0.1), Angle::degrees(0.1)};
    const NavFix third_fix{FixTime::receiver(Timestamp::from_seconds(1700000020)),
                           Angle::degrees(0.2), Angle::degrees(0.2)};

    auto file = FileBuilder{}
                    .add_nav_fix(first_fix)
                    .add_nav_fix(second_fix)
                    .add_nav_fix(third_fix)
                    .finish();

    CHECK(file.nav_point_count() == 3);
}

TEST_CASE("NavFile: move semantics work") {
    auto file = make_minimal();
    auto moved = std::move(file);
    CHECK(moved.nav_point_count() == 1);
}

TEST_CASE("NavFile: from_bytes with invalid data throws") {
    const std::vector<std::uint8_t> garbage = {0x00, 0xFF, 0xAB, 0xCD};
    CHECK_THROWS_AS(static_cast<void>(NavFile::from_bytes(garbage)), Error);
}

#ifdef GTD_FIXTURE_PATH
TEST_CASE("NavFile: open fixture file succeeds") {
    auto file = NavFile::open(GTD_FIXTURE_PATH);
    CHECK(file.nav_point_count() >= 1);
    CHECK(file.title() == "minimal fixture");
    CHECK(file.device() == "gen_fixture");

    auto point = file.nav_point(0);
    CHECK(point.lat.as_degrees() == doctest::Approx(51.5074).epsilon(1e-4));
    CHECK(point.satellite_count == 2);
}
#endif
