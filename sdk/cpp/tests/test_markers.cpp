#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <optional>
#include <stdexcept>

using geotrace::Angle;
using geotrace::Annotation;
using geotrace::EventMarkerStyle;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::MarkerIcon;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Timestamp;

constexpr Timestamp FIRST_FIX_TIME{1'700'000'000'000'000};
constexpr Timestamp MARKER_TIME{1'700'000'005'000'000};
constexpr Timestamp LAST_FIX_TIME{1'700'000'010'000'000};
constexpr std::uint8_t UNRECOGNIZED_ICON_CODE = 200;

namespace {
NavFile make_markers_and_styles() {
    const NavFix first{FixTime::receiver(FIRST_FIX_TIME), Angle::degrees(51.0),
                       Angle::degrees(-1.0)};
    const NavFix last{FixTime::receiver(LAST_FIX_TIME), Angle::degrees(52.0), Angle::degrees(-2.0)};

    return FileBuilder{}
        .add_nav_fix(first)
        .add_nav_fix(last)
        .add_annotation(Annotation{MARKER_TIME, "waypoint", MarkerIcon::Lightning})
        .add_annotation(Annotation{MARKER_TIME, "", MarkerIcon::Pin})
        .add_event_marker_style(EventMarkerStyle{"power/boot", MarkerIcon::Warning, "#FF9900"})
        .add_event_marker_style(EventMarkerStyle{"power/sleep", std::nullopt, ""})
        .finish();
}
} // namespace

TEST_CASE("NavFile: a labelled marker reads back with its icon, time and position") {
    auto file = make_markers_and_styles();
    REQUIRE(file.marker_count() == 2);

    auto marker = file.marker(0);
    CHECK(marker.label == "waypoint");
    REQUIRE(marker.icon.has_value());
    CHECK(marker.icon.value() == MarkerIcon::Lightning);
    CHECK(marker.icon_code == GTD_ICON_LIGHTNING);
    CHECK(marker.time.unix_micros == MARKER_TIME.unix_micros);
    CHECK(marker.lat.as_degrees() == doctest::Approx(51.5).epsilon(1e-9));
    CHECK(marker.lon.as_degrees() == doctest::Approx(-1.5).epsilon(1e-9));
}

TEST_CASE("NavFile: an unlabelled marker reads back with an empty label") {
    auto file = make_markers_and_styles();

    auto marker = file.marker(1);
    CHECK(marker.label.empty());
    REQUIRE(marker.icon.has_value());
    CHECK(marker.icon.value() == MarkerIcon::Pin);
    CHECK(marker.icon_code == GTD_ICON_PIN);
}

TEST_CASE("NavFile: marker out-of-range throws std::out_of_range") {
    auto file = make_markers_and_styles();
    CHECK_THROWS_AS(static_cast<void>(file.marker(2)), std::out_of_range);
    CHECK(file.try_marker(2).error().code == GTD_ERR_OUT_OF_RANGE);
}

TEST_CASE("NavFile: a style with an explicit icon and color reads back") {
    auto file = make_markers_and_styles();
    REQUIRE(file.event_marker_style_count() == 2);

    auto style = file.event_marker_style(0);
    CHECK(style.variant_path == "power/boot");
    REQUIRE(style.icon.has_value());
    CHECK(style.icon.value() == MarkerIcon::Warning);
    CHECK(style.icon_name == "warning");
    CHECK(style.color_hex == "#FF9900");
}

TEST_CASE("NavFile: a style that leaves the icon and color to the application reads back empty") {
    auto file = make_markers_and_styles();

    auto style = file.event_marker_style(1);
    CHECK(style.variant_path == "power/sleep");
    CHECK_FALSE(style.icon.has_value());
    CHECK(style.icon_name.empty());
    CHECK(style.color_hex.empty());
}

TEST_CASE("NavFile: event_marker_style out-of-range throws std::out_of_range") {
    auto file = make_markers_and_styles();
    CHECK_THROWS_AS(static_cast<void>(file.event_marker_style(2)), std::out_of_range);
    CHECK(file.try_event_marker_style(2).error().code == GTD_ERR_OUT_OF_RANGE);
}

#ifdef GTD_UNRECOGNIZED_MARKER_ICON_FIXTURE_PATH
TEST_CASE("NavFile: an icon code outside MarkerIcon reads back as nullopt with its code") {
    auto file = NavFile::open(GTD_UNRECOGNIZED_MARKER_ICON_FIXTURE_PATH);
    REQUIRE(file.marker_count() == 1);

    auto marker = file.marker(0);
    CHECK(marker.label == "hovercraft");
    CHECK_FALSE(marker.icon.has_value());
    CHECK(marker.icon_code == UNRECOGNIZED_ICON_CODE);
}
#endif

#ifdef GTD_UNRECOGNIZED_STYLE_VALUES_FIXTURE_PATH
TEST_CASE("NavFile: an icon name and color outside the known values read back verbatim") {
    auto file = NavFile::open(GTD_UNRECOGNIZED_STYLE_VALUES_FIXTURE_PATH);
    REQUIRE(file.event_marker_style_count() == 1);

    auto style = file.event_marker_style(0);
    CHECK(style.variant_path == "power/boot");
    CHECK_FALSE(style.icon.has_value());
    CHECK(style.icon_name == "hovercraft");
    CHECK(style.color_hex == "FFAA00");
}
#endif
