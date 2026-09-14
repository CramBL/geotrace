#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <utility>
#include <vector>

using geotrace::Satellite;
using geotrace::SatelliteView;

static_assert(noexcept(geotrace::snr_is_no_data_sentinel(1.0F)));
static_assert(noexcept(Satellite{}.snr_is_no_data_sentinel()));
static_assert(noexcept(SatelliteView{}.snr_is_no_data_sentinel()));

// The cases of `the_band_is_half_a_db_wide_either_side` in the Rust SDK's `snr.rs`.
TEST_CASE("snr_is_no_data_sentinel classifies a reading as the Rust SDK does") {
    const std::vector<std::pair<float, bool>> cases{
        {99.0F, true}, {99.4F, true}, {98.5F, false}, {99.5F, false}, {40.0F, false},
    };

    for (const auto &one : cases) {
        CAPTURE(one.first);
        CHECK(geotrace::snr_is_no_data_sentinel(one.first) == one.second);
    }
}

TEST_CASE_TEMPLATE("a satellite classifies its SNR reading, and returns false without one",
                   SatelliteType, Satellite, SatelliteView) {
    SatelliteType satellite{};
    CHECK_FALSE(satellite.snr_is_no_data_sentinel());

    satellite.snr_dbhz = 99.4F;
    CHECK(satellite.snr_is_no_data_sentinel());

    satellite.snr_dbhz = 40.0F;
    CHECK_FALSE(satellite.snr_is_no_data_sentinel());
}
