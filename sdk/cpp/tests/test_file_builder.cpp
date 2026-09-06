#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <optional>
#include <string>
#include <utility>

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

static const Timestamp FIRST_TIME = Timestamp::from_seconds(1700000000ULL);
static const Timestamp SECOND_TIME = Timestamp::from_seconds(1700000010ULL);

TEST_CASE("FileBuilder: single nav fix produces a valid NavFile") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(51.5074),
                     Angle::degrees(-0.1278)};

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    CHECK(file.nav_point_count() == 1);
    auto point = file.nav_point(0);
    CHECK(point.lat.as_degrees() == doctest::Approx(51.5074));
    CHECK(point.lon.as_degrees() == doctest::Approx(-0.1278));
}

TEST_CASE("FileBuilder: metadata is preserved") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(0.0), Angle::degrees(0.0)};

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
    NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(48.8566), Angle::degrees(2.3522)};
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
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(0.0), Angle::degrees(0.0)};

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    auto point = file.nav_point(0);
    CHECK_FALSE(point.heading.has_value());
    CHECK_FALSE(point.speed.has_value());
    CHECK_FALSE(point.eph_m.has_value());
}

TEST_CASE("FileBuilder: satellite report round-trips") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(40.7128),
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

    const SatelliteReport report{FixTime::receiver(FIRST_TIME), {gps_satellite, glonass_satellite}};

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
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(35.6762),
                     Angle::degrees(139.6503)};

    const EventMarker marker{"system/startup", FIRST_TIME, "Device started"};

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
    const NavFix first_fix{FixTime::receiver(FIRST_TIME), Angle::degrees(1.0), Angle::degrees(2.0)};
    const NavFix second_fix{FixTime::receiver(SECOND_TIME), Angle::degrees(1.1),
                            Angle::degrees(2.1)};

    auto file =
        FileBuilder{}.device("chain test").add_nav_fix(first_fix).add_nav_fix(second_fix).finish();

    CHECK(file.nav_point_count() == 2);
    CHECK(file.device() == "chain test");
}

// An annotation with no icon set must reach the C boundary as `GTD_ICON_PIN`,
// which `gtd_builder_add_annotation` accepts.
static_assert(geotrace::detail::to_c(MarkerIcon::Pin) == GTD_ICON_PIN);

static_assert(geotrace::detail::to_c(std::optional<MarkerIcon>{}) == GTD_ICON_AUTO);
static_assert(geotrace::detail::to_c(std::optional<MarkerIcon>{MarkerIcon::Gear}) == GTD_ICON_GEAR);

TEST_CASE("FileBuilder: an annotation with no icon set is written as Pin") {
    const NavFix first_fix{FixTime::receiver(FIRST_TIME), Angle::degrees(51.5074),
                           Angle::degrees(-0.1278)};
    const NavFix second_fix{FixTime::receiver(SECOND_TIME), Angle::degrees(51.5080),
                            Angle::degrees(-0.1265)};

    const Annotation ann{Timestamp::from_seconds(1700000005ULL)};
    CHECK(ann.icon == MarkerIcon::Pin);

    const NavFile file = FileBuilder{}.add(first_fix).add(second_fix).add(ann).finish();
    CHECK(file.nav_point_count() == 2);
}

TEST_CASE("FileBuilder: NoNavFixesError thrown when annotations exist but no fixes") {
    FileBuilder builder;
    Annotation ann{FIRST_TIME};
    ann.label = "unreachable";
    builder.add_annotation(ann);
    CHECK_THROWS_AS(static_cast<void>(builder.finish()), NoNavFixesError);
}

TEST_CASE("FileBuilder: FieldTooLongError thrown for a label past the field capacity") {
    FileBuilder builder;
    Annotation ann{FIRST_TIME};
    ann.label = std::string(256, 'l');

    CHECK_THROWS_AS(builder.add_annotation(ann), FieldTooLongError);
}

TEST_CASE("FileBuilder: InvalidPathError thrown for malformed variant path") {
    FileBuilder builder;
    builder.add_nav_fix(
        NavFix{FixTime::receiver(FIRST_TIME), Angle::degrees(0.0), Angle::degrees(0.0)});

    const EventMarker marker{"bad path with spaces!", FIRST_TIME};

    CHECK_THROWS_AS(builder.add_event_marker(marker), InvalidPathError);
}

TEST_CASE("FileBuilder: move semantics work") {
    FileBuilder builder;
    builder.add_nav_fix(
        NavFix{FixTime::receiver(FIRST_TIME), Angle::degrees(0.0), Angle::degrees(0.0)});

    FileBuilder moved_builder = std::move(builder);
    auto file = moved_builder.finish();
    CHECK(file.nav_point_count() == 1);
}

TEST_CASE("FileBuilder: add() dispatches by argument type") {
    // Two fixes bracket the annotation and event marker so both fall in range.
    const NavFix first_fix{FixTime::receiver(FIRST_TIME), Angle::degrees(51.5074),
                           Angle::degrees(-0.1278)};
    const NavFix second_fix{FixTime::receiver(SECOND_TIME), Angle::degrees(51.5080),
                            Angle::degrees(-0.1265)};

    Satellite sat{};
    sat.constellation = Constellation::Gps;
    sat.prn = 1;
    sat.in_fix = true;

    const SatelliteReport report{FixTime::receiver(FIRST_TIME), {sat}};

    const Timestamp mid = Timestamp::from_seconds(1700000005ULL);
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
