// The parsers for the lower-case wire names of the `.gtd` format. Each case
// covers every name of one set, so a mapping that swaps two of them fails.
#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <string>
#include <utility>
#include <vector>

using geotrace::Constellation;
using geotrace::MarkerIcon;

TEST_CASE("every constellation name parses to its constellation") {
    const std::vector<std::pair<std::string, Constellation>> cases{
        {"gps", Constellation::Gps},         {"glonass", Constellation::Glonass},
        {"galileo", Constellation::Galileo}, {"beidou", Constellation::Beidou},
        {"navic", Constellation::Navic},     {"qzss", Constellation::Qzss},
    };

    for (const auto &one : cases) {
        CAPTURE(one.first);
        CHECK(geotrace::constellation_from_name(one.first) == one.second);
        CHECK(geotrace::try_constellation_from_name(one.first).value() == one.second);
    }
}

TEST_CASE("a constellation name outside the set is a parse error") {
    const geotrace::Result<Constellation> parsed = geotrace::try_constellation_from_name("pulsar");
    REQUIRE(parsed.is_err());
    CHECK(parsed.error().code == GTD_ERR_PARSE);
    CHECK(parsed.error().description.find("pulsar") != std::string::npos);
    CHECK_THROWS_AS(static_cast<void>(geotrace::constellation_from_name("pulsar")),
                    geotrace::ParseError);
}

TEST_CASE("every marker icon name parses to its icon") {
    const std::vector<std::pair<std::string, MarkerIcon>> cases{
        {"pin", MarkerIcon::Pin},
        {"cross", MarkerIcon::Cross},
        {"circle", MarkerIcon::Circle},
        {"lightning", MarkerIcon::Lightning},
        {"warning", MarkerIcon::Warning},
        {"error", MarkerIcon::Error},
        {"check", MarkerIcon::Check},
        {"satellite", MarkerIcon::Satellite},
        {"satellite_lost", MarkerIcon::SatelliteLost},
        {"gear", MarkerIcon::Gear},
        {"refresh", MarkerIcon::Refresh},
        {"download", MarkerIcon::Download},
        {"upload", MarkerIcon::Upload},
        {"wrench", MarkerIcon::Wrench},
    };

    for (const auto &one : cases) {
        CAPTURE(one.first);
        CHECK(geotrace::marker_icon_from_name(one.first) == one.second);
        CHECK(geotrace::try_marker_icon_from_name(one.first).value() == one.second);
    }
}

// `GTD_ICON_AUTO` is the one `GtdMarkerIcon` value with no wire name.
TEST_CASE("a marker icon name outside the set is a parse error") {
    const geotrace::Result<MarkerIcon> parsed = geotrace::try_marker_icon_from_name("compass");
    REQUIRE(parsed.is_err());
    CHECK(parsed.error().code == GTD_ERR_PARSE);
    CHECK(parsed.error().description.find("compass") != std::string::npos);
    CHECK_THROWS_AS(static_cast<void>(geotrace::marker_icon_from_name("compass")),
                    geotrace::ParseError);
    CHECK(geotrace::try_marker_icon_from_name("auto").is_err());
}
