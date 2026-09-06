#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cmath>

using geotrace::Angle;
using geotrace::Constellation;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Satellite;
using geotrace::SatelliteReport;
using geotrace::Timestamp;

#ifdef GTD_OUT_OF_RANGE_FIXTURE_PATH
TEST_CASE("NavFile: out-of-range coordinates read verbatim") {
    auto file = NavFile::open(GTD_OUT_OF_RANGE_FIXTURE_PATH);
    REQUIRE(file.nav_point_count() == 4);

    CHECK(std::isnan(file.nav_point(0).lat.as_degrees()));
    CHECK(file.nav_point(1).lat.as_degrees() == doctest::Approx(91.0));
    CHECK(file.nav_point(2).lon.as_degrees() == doctest::Approx(-181.0));

    const auto heading = file.nav_point(3).heading;
    REQUIRE(heading.has_value());
    CHECK(heading->as_degrees() == doctest::Approx(675.0));
}
#endif

TEST_CASE("NavFile: satellite warnings report the PRN and the SNR sentinel") {
    constexpr Timestamp FIX_TIME{1'700'000'000'000'000};
    const NavFix fix{FixTime::receiver(FIX_TIME), Angle::degrees(51.5), Angle::degrees(-0.1)};
    const SatelliteReport report{
        FixTime::receiver(FIX_TIME),
        {
            Satellite{Constellation::Gps, 0, true, 45.0F, 90.0F, 40.0F},
            Satellite{Constellation::Gps, 5, true, 30.0F, 120.0F, 99.0F},
        },
    };

    const NavFile file = FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish();

    REQUIRE(file.satellite_warning_count() == 2);

    const auto prn_zero = file.satellite_warning(0);
    CHECK(prn_zero.count == 1);
    CHECK(prn_zero.issue == "satellite(s) with PRN 0");
    CHECK(prn_zero.description == "PRN 0 is reserved and undefined in NMEA");

    const auto snr_sentinel = file.satellite_warning(1);
    CHECK(snr_sentinel.count == 1);
    // The issue text contains "\xe2\x89\x88 99 dB-Hz". Writing that
    // character as its UTF-8 bytes keeps the comparison independent of the
    // compiler's source and execution character sets.
    CHECK(snr_sentinel.issue == "satellite(s) with SNR \xe2\x89\x88 99 dB-Hz");
    CHECK(snr_sentinel.description ==
          "common firmware sentinel for unavailable signal strength; omit the SNR field when no "
          "measurement is available");

    CHECK(file.try_satellite_warning(2).error().code == GTD_ERR_OUT_OF_RANGE);
}
