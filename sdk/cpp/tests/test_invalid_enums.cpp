#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <optional>
#include <stdexcept>

#include "test_undeclared_enums.hpp"

using geotrace::Angle;
using geotrace::Annotation;
using geotrace::Constellation;
using geotrace::EventMarkerStyle;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::MarkerIcon;
using geotrace::NavFix;
using geotrace::Satellite;
using geotrace::SatelliteReport;
using geotrace::Timestamp;
using geotrace::TravelMode;

constexpr Timestamp FIX_TIME{1'700'000'000'000'000};

constexpr std::uint8_t FIRST_UNDECLARED_CONSTELLATION_CODE =
    static_cast<std::uint8_t>(Constellation::Qzss) + 1;
constexpr std::uint8_t FIRST_UNDECLARED_ICON_CODE =
    static_cast<std::uint8_t>(MarkerIcon::Wrench) + 1;
constexpr std::uint8_t FIRST_UNDECLARED_TRAVEL_MODE_CODE =
    static_cast<std::uint8_t>(TravelMode::Aircraft) + 1;

namespace {

NavFix one_fix() {
    return NavFix{FixTime::receiver(FIX_TIME), Angle::degrees(51.5), Angle::degrees(-0.1)};
}

} // namespace

TEST_CASE("an undeclared travel mode throws std::invalid_argument") {
    FileBuilder builder;
    CHECK_THROWS_AS(builder.travel_mode(undeclared_enum_value<TravelMode>()),
                    std::invalid_argument);
    CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
    CHECK(builder.status().description == "TravelMode has no enumerator with the value 200");
}

TEST_CASE("an undeclared constellation throws and states the satellite's index") {
    SatelliteReport report{FixTime::receiver(FIX_TIME), {}};
    report.tracked.push_back(
        Satellite{Constellation::Galileo, 1, true, std::nullopt, std::nullopt, std::nullopt});
    report.tracked.push_back(Satellite{undeclared_enum_value<Constellation>(), 2, true,
                                       std::nullopt, std::nullopt, std::nullopt});

    FileBuilder builder;
    builder.add(one_fix());
    CHECK_THROWS_AS(builder.add_satellite_report(report), std::invalid_argument);
    CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
    CHECK(builder.status().description ==
          "tracked[1].constellation: Constellation has no enumerator with the value 200");
}

TEST_CASE("an annotation with an undeclared icon throws std::invalid_argument") {
    FileBuilder builder;
    builder.add(one_fix());
    const Annotation annotation{FIX_TIME, "waypoint", undeclared_enum_value<MarkerIcon>()};
    CHECK_THROWS_AS(builder.add_annotation(annotation), std::invalid_argument);
    CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
    CHECK(builder.status().description == "MarkerIcon has no enumerator with the value 200");
}

TEST_CASE("an event marker style with an undeclared icon throws std::invalid_argument") {
    FileBuilder builder;
    const EventMarkerStyle style{"power/boot", undeclared_enum_value<MarkerIcon>(), ""};
    CHECK_THROWS_AS(builder.add_event_marker_style(style), std::invalid_argument);
    CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
    CHECK(builder.status().description == "MarkerIcon has no enumerator with the value 200");
}

TEST_CASE("an undeclared travel mode has no wire name") {
    CHECK(geotrace::travel_mode_name(undeclared_enum_value<TravelMode>()) == std::nullopt);
}

TEST_CASE("the code of an enumerator converts back to it") {
    CHECK(geotrace::constellation_from_code(static_cast<std::uint8_t>(Constellation::Qzss)) ==
          Constellation::Qzss);
    CHECK(geotrace::marker_icon_from_code(static_cast<std::uint8_t>(MarkerIcon::Wrench)) ==
          MarkerIcon::Wrench);
    CHECK(geotrace::travel_mode_from_code(static_cast<std::uint8_t>(TravelMode::Aircraft)) ==
          TravelMode::Aircraft);
}

TEST_CASE("a code no enumerator declares converts to std::nullopt") {
    CHECK(geotrace::constellation_from_code(FIRST_UNDECLARED_CONSTELLATION_CODE) == std::nullopt);
    CHECK(geotrace::marker_icon_from_code(FIRST_UNDECLARED_ICON_CODE) == std::nullopt);
    CHECK(geotrace::marker_icon_from_code(static_cast<std::uint8_t>(GTD_ICON_AUTO)) ==
          std::nullopt);
    CHECK(geotrace::travel_mode_from_code(FIRST_UNDECLARED_TRAVEL_MODE_CODE) == std::nullopt);
}
