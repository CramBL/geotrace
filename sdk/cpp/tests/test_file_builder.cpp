#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <chrono>
#include <cstddef>
#include <functional>
#include <optional>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

#include "test_timestamps.hpp"

#if defined(__GNUC__) && !defined(__clang__)
// False positive: once add_nav_fix() and detail::to_c() get inlined across
// this file's many FileBuilder chains, GCC's -Wmaybe-uninitialized loses
// track of std::optional's engaged/payload invariant for `heading`/`speed`/
// `eph_m` and flags reads of NavFix's default-constructed (empty) optionals.
// File-scoped because the false positive recurs at nearly every call site.
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wmaybe-uninitialized"
#endif

using geotrace::Angle;
using geotrace::Annotation;
using geotrace::Channel;
using geotrace::Constellation;
using geotrace::EventMarker;
using geotrace::EventMarkerStyle;
using geotrace::FieldTooLongError;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::InvalidPathError;
using geotrace::MarkerIcon;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::NoNavFixesError;
using geotrace::Satellite;
using geotrace::SatelliteReport;
using geotrace::Timestamp;
using geotrace::Velocity;

namespace {

struct StringArgumentWithANulByte {
    GtdStatus status;
    std::string message;
    std::function<void(FileBuilder &)> call;
};

Channel one_sample_channel(std::string name) {
    Channel channel{};
    channel.name = std::move(name);
    channel.times = {fix_timestamp()};
    channel.values = {1.0};
    return channel;
}

} // namespace

TEST_CASE("FileBuilder: single nav fix produces a valid NavFile") {
    const NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(51.5074),
                     Angle::degrees(-0.1278)};

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    CHECK(file.nav_point_count() == 1);
    auto point = file.nav_point(0);
    CHECK(point.lat.as_degrees() == doctest::Approx(51.5074));
    CHECK(point.lon.as_degrees() == doctest::Approx(-0.1278));
}

TEST_CASE("FileBuilder: a satellite window wider than the default associates a late report") {
    const NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(40.7128),
                     Angle::degrees(-74.0060)};
    const SatelliteReport report{
        FixTime::receiver(after_fix_timestamp(std::chrono::milliseconds{1500})),
        {Satellite{Constellation::Gps, 7, true, 55.0F, 120.0F, 40.0F}},
    };

    const NavFile with_the_default_window =
        FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish();
    CHECK(with_the_default_window.nav_point_count() == 2);

    const NavFile with_a_two_second_window = FileBuilder{}
                                                 .satellite_window(std::chrono::seconds{2})
                                                 .add_nav_fix(fix)
                                                 .add_satellite_report(report)
                                                 .finish();
    CHECK(with_a_two_second_window.nav_point_count() == 1);
}

TEST_CASE("FileBuilder: a negative satellite window is an invalid argument") {
    FileBuilder builder;
    CHECK_THROWS_AS(builder.satellite_window(std::chrono::seconds{-1}), std::invalid_argument);
    CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
}

TEST_CASE("FileBuilder: metadata is preserved") {
    const NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(0.0), Angle::degrees(0.0)};

    const NavFile file = FileBuilder{}
                             .title("my track")
                             .device("test device")
                             .notes("some notes")
                             .identity("unit-test")
                             .add_nav_fix(fix)
                             .finish();

    CHECK(file.title() == "my track");
    CHECK(file.device() == "test device");
    CHECK(file.notes() == "some notes");
    CHECK(file.identity() == "unit-test");
}

TEST_CASE("FileBuilder: optional fields round-trip") {
    NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(48.8566), Angle::degrees(2.3522)};
    fix.heading = Angle::degrees(180.0);
    fix.speed = Velocity::mps(10.0);
    fix.eph_m = 5.0;

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    auto point = file.nav_point(0);
    REQUIRE(point.heading.has_value());
    CHECK(point.heading.value().as_degrees() == doctest::Approx(180.0));
    REQUIRE(point.speed.has_value());
    CHECK(point.speed.value().as_mps() == doctest::Approx(10.0));
    REQUIRE(point.eph_m.has_value());
    CHECK(point.eph_m.value() == doctest::Approx(5.0));
}

TEST_CASE("FileBuilder: no-optional nav fix has nullopt fields") {
    const NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(0.0), Angle::degrees(0.0)};

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    auto point = file.nav_point(0);
    CHECK_FALSE(point.heading.has_value());
    CHECK_FALSE(point.speed.has_value());
    CHECK_FALSE(point.eph_m.has_value());
}

TEST_CASE("FileBuilder: satellite report round-trips") {
    const NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(40.7128),
                     Angle::degrees(-74.0060)};

    Satellite gps_satellite{};
    gps_satellite.constellation = Constellation::Gps;
    gps_satellite.prn = 7;
    gps_satellite.in_fix = true;
    gps_satellite.elevation_deg = 55.0F;
    gps_satellite.azimuth_deg = 120.0F;
    gps_satellite.snr_dbhz = 40.0F;

    Satellite glonass_satellite{};
    glonass_satellite.constellation = Constellation::Glonass;
    glonass_satellite.prn = 2;
    glonass_satellite.in_fix = false;
    glonass_satellite.snr_dbhz = 28.0F;

    const SatelliteReport report{FixTime::receiver(fix_timestamp()),
                                 {gps_satellite, glonass_satellite}};

    const NavFile file = FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish();

    auto point = file.nav_point(0);
    CHECK(point.satellite_count == 2);

    auto first_satellite = file.satellite(0, 0);
    CHECK(first_satellite.constellation == Constellation::Gps);
    CHECK(first_satellite.prn == 7);
    CHECK(first_satellite.in_fix);
    REQUIRE(first_satellite.snr_dbhz.has_value());
    CHECK(first_satellite.snr_dbhz.value() == doctest::Approx(40.0));

    auto s1_out = file.satellite(0, 1);
    CHECK(s1_out.constellation == Constellation::Glonass);
    CHECK_FALSE(s1_out.in_fix);
}

TEST_CASE("FileBuilder: event marker round-trips") {
    const NavFix fix{FixTime::receiver(fix_timestamp()), Angle::degrees(35.6762),
                     Angle::degrees(139.6503)};

    const EventMarker marker{"system/startup", fix_timestamp(), "Device started"};

    EventMarkerStyle style{};
    style.variant_path = "system/startup";
    style.icon = MarkerIcon::Gear;
    style.color_hex = "#00FF00";

    const NavFile file = FileBuilder{}
                             .add_nav_fix(fix)
                             .add_event_marker(marker)
                             .add_event_marker_style(style)
                             .finish();

    REQUIRE(file.event_marker_count() == 1);
    auto read_marker = file.event_marker(0);
    CHECK(read_marker.variant_path == "system/startup");
    CHECK(read_marker.annotation == "Device started");
}

TEST_CASE("FileBuilder: fluent chain works end-to-end") {
    const NavFix first_fix{FixTime::receiver(fix_timestamp()), Angle::degrees(1.0),
                           Angle::degrees(2.0)};
    const NavFix second_fix{FixTime::receiver(after_fix_timestamp(std::chrono::seconds{10})),
                            Angle::degrees(1.1), Angle::degrees(2.1)};

    auto file =
        FileBuilder{}.device("chain test").add_nav_fix(first_fix).add_nav_fix(second_fix).finish();

    CHECK(file.nav_point_count() == 2);
    CHECK(file.device() == "chain test");
}

// An annotation with no icon set must reach the C boundary as `GTD_ICON_PIN`,
// which `gtd_builder_add_annotation` accepts.
static_assert(geotrace::detail::to_c(MarkerIcon::Pin) == GTD_ICON_PIN);

TEST_CASE("FileBuilder: an annotation with no icon set is written as Pin") {
    const NavFix first_fix{FixTime::receiver(fix_timestamp()), Angle::degrees(51.5074),
                           Angle::degrees(-0.1278)};
    const NavFix second_fix{FixTime::receiver(after_fix_timestamp(std::chrono::seconds{10})),
                            Angle::degrees(51.5080), Angle::degrees(-0.1265)};

    const Annotation ann{Timestamp::from_seconds(1700000005)};
    CHECK(ann.icon == MarkerIcon::Pin);

    const NavFile file = FileBuilder{}.add(first_fix).add(second_fix).add(ann).finish();
    CHECK(file.nav_point_count() == 2);
}

TEST_CASE("FileBuilder: NoNavFixesError thrown when annotations exist but no fixes") {
    FileBuilder builder;
    Annotation ann{fix_timestamp()};
    ann.label = "unreachable";
    builder.add_annotation(ann);
    CHECK_THROWS_AS(static_cast<void>(builder.finish()), NoNavFixesError);
}

TEST_CASE("FileBuilder: NoNavFixesError thrown when an event marker exists but no fixes") {
    FileBuilder builder;
    builder.add_event_marker(EventMarker{"power/boot", fix_timestamp()});
    CHECK_THROWS_AS(static_cast<void>(builder.finish()), NoNavFixesError);
}

TEST_CASE("FileBuilder: NoNavFixesError thrown when satellite reports exist but no fixes") {
    const Satellite satellite_out_of_fix{Constellation::Gps, 7, false, 55.0F, 120.0F, 40.0F};
    FileBuilder builder;
    builder.add_satellite_report(
        SatelliteReport{FixTime::receiver(fix_timestamp()), {satellite_out_of_fix}});
    builder.add_satellite_report(SatelliteReport{
        FixTime::receiver(after_fix_timestamp(std::chrono::seconds{10})), {satellite_out_of_fix}});
    CHECK_THROWS_WITH_AS(static_cast<void>(builder.finish()),
                         "2 satellite report(s) have no nav fix to take a position from: at least "
                         "one nav fix is required",
                         NoNavFixesError);
}

TEST_CASE("FileBuilder: FieldTooLongError thrown for a label past the field capacity") {
    FileBuilder builder;
    Annotation ann{fix_timestamp()};
    ann.label = std::string(256, 'l');

    CHECK_THROWS_AS(builder.add_annotation(ann), FieldTooLongError);
}

TEST_CASE("FileBuilder: InvalidPathError thrown for malformed variant path") {
    FileBuilder builder;
    builder.add_nav_fix(
        NavFix{FixTime::receiver(fix_timestamp()), Angle::degrees(0.0), Angle::degrees(0.0)});

    const EventMarker marker{"bad path with spaces!", fix_timestamp()};

    CHECK_THROWS_AS(builder.add_event_marker(marker), InvalidPathError);
}

TEST_CASE("FileBuilder: a style color outside the #RRGGBB form throws std::invalid_argument") {
    for (const std::string color : {"red", "FF9900", "   "}) {
        CAPTURE(color);
        const std::string message =
            "invalid event marker color \"" + color + "\": expected the #RRGGBB form";
        FileBuilder builder;
        CHECK_THROWS_WITH_AS(
            builder.add_event_marker_style(EventMarkerStyle{"power/boot", std::nullopt, color}),
            message.c_str(), std::invalid_argument);
    }
}

TEST_CASE("FileBuilder: a style variant path the event marker rules reject throws") {
    FileBuilder builder;
    SUBCASE("an empty path") {
        CHECK_THROWS_WITH_AS(builder.add_event_marker_style(EventMarkerStyle{""}),
                             "invalid event marker variant path \"\": path is empty",
                             InvalidPathError);
    }
    SUBCASE("a non-ASCII path") {
        CHECK_THROWS_WITH_AS(builder.add_event_marker_style(EventMarkerStyle{"über_lang"}),
                             "invalid event marker variant path \"über_lang\": contains "
                             "characters outside ASCII alphanumeric, hyphen, underscore, and slash",
                             InvalidPathError);
    }
    SUBCASE("a path past 255 bytes") {
        CHECK_THROWS_AS(builder.add_event_marker_style(EventMarkerStyle{std::string(256, 'p')}),
                        FieldTooLongError);
    }
}

TEST_CASE("FileBuilder: a string argument with a nul byte throws and states the string") {
    const std::string with_a_nul_byte = std::string{"before"} + '\0' + "after";

    Channel described_with_a_nul_byte = one_sample_channel("speed");
    described_with_a_nul_byte.description = with_a_nul_byte;
    Channel labelled_with_a_nul_byte = one_sample_channel("accel");
    labelled_with_a_nul_byte.components = {"x", with_a_nul_byte};
    labelled_with_a_nul_byte.values = {1.0, 2.0};

    const std::vector<StringArgumentWithANulByte> arguments{
        {GTD_ERR_INVALID_ARGUMENT, "the title value has a nul byte at offset 6",
         [&](FileBuilder &builder) { builder.title(with_a_nul_byte); }},
        {GTD_ERR_INVALID_ARGUMENT, "the device value has a nul byte at offset 6",
         [&](FileBuilder &builder) { builder.device(with_a_nul_byte); }},
        {GTD_ERR_INVALID_ARGUMENT, "the notes value has a nul byte at offset 6",
         [&](FileBuilder &builder) { builder.notes(with_a_nul_byte); }},
        {GTD_ERR_INVALID_ARGUMENT, "the identity value has a nul byte at offset 6",
         [&](FileBuilder &builder) { builder.identity(with_a_nul_byte); }},
        {GTD_ERR_INVALID_CHANNEL, "the channel name has a nul byte at offset 6",
         [&](FileBuilder &builder) { builder.add_channel(one_sample_channel(with_a_nul_byte)); }},
        {GTD_ERR_INVALID_CHANNEL, "channel \"speed\": the description has a nul byte at offset 6",
         [&](FileBuilder &builder) { builder.add_channel(described_with_a_nul_byte); }},
        {GTD_ERR_INVALID_CHANNEL,
         "channel \"accel\": the label of component 1 has a nul byte at offset 6",
         [&](FileBuilder &builder) { builder.add_channel(labelled_with_a_nul_byte); }},
        {GTD_ERR_INVALID_ARGUMENT, "the annotation label has a nul byte at offset 6",
         [&](FileBuilder &builder) {
             builder.add_annotation(Annotation{fix_timestamp(), with_a_nul_byte});
         }},
        {GTD_ERR_INVALID_PATH, "the event marker variant path has a nul byte at offset 6",
         [&](FileBuilder &builder) {
             builder.add_event_marker(EventMarker{with_a_nul_byte, fix_timestamp()});
         }},
        {GTD_ERR_INVALID_ARGUMENT, "the event marker annotation has a nul byte at offset 6",
         [&](FileBuilder &builder) {
             builder.add_event_marker(EventMarker{"power/boot", fix_timestamp(), with_a_nul_byte});
         }},
        {GTD_ERR_INVALID_PATH, "the event marker style variant path has a nul byte at offset 6",
         [&](FileBuilder &builder) {
             builder.add_event_marker_style(EventMarkerStyle{with_a_nul_byte});
         }},
        {GTD_ERR_INVALID_ARGUMENT, "the event marker style color has a nul byte at offset 6",
         [&](FileBuilder &builder) {
             builder.add_event_marker_style(
                 EventMarkerStyle{"power/boot", std::nullopt, with_a_nul_byte});
         }},
    };

    for (const StringArgumentWithANulByte &argument : arguments) {
        CAPTURE(argument.message);
        FileBuilder builder;
        CHECK_THROWS(argument.call(builder));
        CHECK(builder.status().code == argument.status);
        CHECK(builder.status().description == argument.message);
    }
}

TEST_CASE("FileBuilder: move semantics work") {
    FileBuilder builder;
    builder.add_nav_fix(
        NavFix{FixTime::receiver(fix_timestamp()), Angle::degrees(0.0), Angle::degrees(0.0)});

    FileBuilder moved_builder = std::move(builder);
    auto file = moved_builder.finish();
    CHECK(file.nav_point_count() == 1);
}

TEST_CASE("FileBuilder: add() dispatches by argument type") {
    // Two fixes bracket the annotation and event marker so both fall in range.
    const NavFix first_fix{FixTime::receiver(fix_timestamp()), Angle::degrees(51.5074),
                           Angle::degrees(-0.1278)};
    const NavFix second_fix{FixTime::receiver(after_fix_timestamp(std::chrono::seconds{10})),
                            Angle::degrees(51.5080), Angle::degrees(-0.1265)};

    Satellite sat{};
    sat.constellation = Constellation::Gps;
    sat.prn = 1;
    sat.in_fix = true;

    const SatelliteReport report{FixTime::receiver(fix_timestamp()), {sat}};

    const Timestamp mid = Timestamp::from_seconds(1700000005);
    Annotation ann{mid};
    ann.label = "midpoint";
    ann.icon = MarkerIcon::Pin;

    const EventMarker marker{"power/boot", mid, "cold start"};

    // Each add() resolves, at compile time, to the matching add_* overload.
    const NavFile file =
        FileBuilder{}.add(first_fix).add(second_fix).add(report).add(ann).add(marker).finish();

    CHECK(file.nav_point_count() == 2);

    // The satellite report associated with a fix (add(SatelliteReport) dispatched).
    std::size_t total_sats = 0;
    for (std::size_t i = 0; i < file.nav_point_count(); ++i) {
        total_sats += file.nav_point(i).satellite_count;
    }
    CHECK(total_sats >= 1);

    // The event marker landed with its path (add(EventMarker) dispatched).
    REQUIRE(file.event_marker_count() == 1);
    CHECK(file.event_marker(0).variant_path == "power/boot");
}

#if defined(__GNUC__) && !defined(__clang__)
#pragma GCC diagnostic pop
#endif
