#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <cstddef>
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
using geotrace::FileBuilder;
using geotrace::InvalidPathError;
using geotrace::MarkerIcon;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::NoNavFixesError;
using geotrace::Satellite;
using geotrace::SatelliteReport;
using geotrace::Timestamp;
using geotrace::Velocity;

static Timestamp t0 = Timestamp::from_seconds(1700000000ULL);
static Timestamp t1 = Timestamp::from_seconds(1700000010ULL);

TEST_CASE("FileBuilder: single nav fix produces a valid NavFile") {
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(51.5074);
    fix.lon = Angle::degrees(-0.1278);

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    CHECK(file.nav_point_count() == 1);
    auto p = file.nav_point(0);
    CHECK(p.lat.as_degrees() == doctest::Approx(51.5074));
    CHECK(p.lon.as_degrees() == doctest::Approx(-0.1278));
}

TEST_CASE("FileBuilder: metadata is preserved") {
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(0.0);
    fix.lon = Angle::degrees(0.0);

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
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(48.8566);
    fix.lon = Angle::degrees(2.3522);
    fix.heading = Angle::degrees(180.0);
    fix.speed = Velocity::mps(10.0);
    fix.eph_m = 5.0;

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    auto p = file.nav_point(0);
    REQUIRE(p.heading.has_value());
    CHECK(p.heading.value().as_degrees() == doctest::Approx(180.0));
    REQUIRE(p.speed.has_value());
    CHECK(p.speed.value().as_mps() == doctest::Approx(10.0));
    REQUIRE(p.eph_m.has_value());
    CHECK(p.eph_m.value() == doctest::Approx(5.0));
}

TEST_CASE("FileBuilder: no-optional nav fix has nullopt fields") {
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(0.0);
    fix.lon = Angle::degrees(0.0);

    const NavFile file = FileBuilder{}.add_nav_fix(fix).finish();

    auto p = file.nav_point(0);
    CHECK_FALSE(p.heading.has_value());
    CHECK_FALSE(p.speed.has_value());
    CHECK_FALSE(p.eph_m.has_value());
}

TEST_CASE("FileBuilder: satellite report round-trips") {
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(40.7128);
    fix.lon = Angle::degrees(-74.0060);

    Satellite s1{};
    s1.constellation = Constellation::Gps;
    s1.prn = 7;
    s1.in_fix = true;
    s1.elevation_deg = 55.0;
    s1.azimuth_deg = 120.0;
    s1.snr_dbhz = 40.0;

    Satellite s2{};
    s2.constellation = Constellation::Glonass;
    s2.prn = 2;
    s2.in_fix = false;
    s2.snr_dbhz = 28.0;

    SatelliteReport report{};
    report.gps_time = t0;
    report.tracked.push_back(s1);
    report.tracked.push_back(s2);

    const NavFile file = FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish();

    auto p = file.nav_point(0);
    CHECK(p.satellite_count == 2);

    auto s0 = file.satellite(0, 0);
    CHECK(s0.constellation == Constellation::Gps);
    CHECK(s0.prn == 7);
    CHECK(s0.in_fix);
    REQUIRE(s0.snr_dbhz.has_value());
    CHECK(s0.snr_dbhz.value() == doctest::Approx(40.0));

    auto s1_out = file.satellite(0, 1);
    CHECK(s1_out.constellation == Constellation::Glonass);
    CHECK_FALSE(s1_out.in_fix);
}

TEST_CASE("FileBuilder: event marker round-trips") {
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(35.6762);
    fix.lon = Angle::degrees(139.6503);

    EventMarker m1{};
    m1.variant_path = "system/startup";
    m1.sys_time = t0;
    m1.annotation = "Device started";

    EventMarkerStyle style{};
    style.variant_path = "system/startup";
    style.icon = MarkerIcon::Gear;
    style.color_hex = "#00FF00";

    const NavFile file =
        FileBuilder{}.add_nav_fix(fix).add_event_marker(m1).add_event_marker_style(style).finish();

    REQUIRE(file.event_marker_count() == 1);
    auto m = file.event_marker(0);
    CHECK(m.variant_path == "system/startup");
    CHECK(m.annotation == "Device started");
}

TEST_CASE("FileBuilder: fluent chain works end-to-end") {
    NavFix f1{};
    f1.gps_time = t0;
    f1.lat = Angle::degrees(1.0);
    f1.lon = Angle::degrees(2.0);

    NavFix f2{};
    f2.gps_time = t1;
    f2.lat = Angle::degrees(1.1);
    f2.lon = Angle::degrees(2.1);

    auto file = FileBuilder{}.device("chain test").add_nav_fix(f1).add_nav_fix(f2).finish();

    CHECK(file.nav_point_count() == 2);
    CHECK(file.device() == "chain test");
}

TEST_CASE("FileBuilder: NoNavFixesError thrown when annotations exist but no fixes") {
    FileBuilder b;
    Annotation ann{};
    ann.time = t0;
    ann.label = "unreachable";
    b.add_annotation(ann);
    CHECK_THROWS_AS(b.finish(), NoNavFixesError);
}

TEST_CASE("FileBuilder: InvalidPathError thrown for malformed variant path") {
    FileBuilder b;
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(0.0);
    fix.lon = Angle::degrees(0.0);
    b.add_nav_fix(fix);

    EventMarker marker{};
    marker.variant_path = "bad path with spaces!";
    marker.sys_time = t0;

    CHECK_THROWS_AS(b.add_event_marker(marker), InvalidPathError);
}

TEST_CASE("FileBuilder: move semantics work") {
    FileBuilder b1;
    NavFix fix{};
    fix.gps_time = t0;
    fix.lat = Angle::degrees(0.0);
    fix.lon = Angle::degrees(0.0);
    b1.add_nav_fix(fix);

    FileBuilder b2 = std::move(b1);
    auto file = b2.finish();
    CHECK(file.nav_point_count() == 1);
}

TEST_CASE("FileBuilder: add() dispatches by argument type") {
    // Two fixes bracket the annotation and event marker so both fall in range.
    NavFix f0{};
    f0.gps_time = t0;
    f0.lat = Angle::degrees(51.5074);
    f0.lon = Angle::degrees(-0.1278);

    NavFix f1{};
    f1.gps_time = t1;
    f1.lat = Angle::degrees(51.5080);
    f1.lon = Angle::degrees(-0.1265);

    SatelliteReport report{};
    report.gps_time = t0;
    Satellite sat{};
    sat.constellation = Constellation::Gps;
    sat.prn = 1;
    sat.in_fix = true;
    report.tracked.push_back(sat);

    const Timestamp mid = Timestamp::from_seconds(1700000005ULL);
    Annotation ann{};
    ann.time = mid;
    ann.label = "midpoint";
    ann.icon = MarkerIcon::Pin;

    const EventMarker marker{"power/boot", mid, "cold start"};

    // Each add() resolves, at compile time, to the matching add_* overload.
    const NavFile file = FileBuilder{}.add(f0).add(f1).add(report).add(ann).add(marker).finish();

    CHECK(file.nav_point_count() == 2);

    // The satellite report associated with a fix (add(SatelliteReport) dispatched).
    std::size_t total_sats = 0;
    for (std::size_t i = 0; i < file.nav_point_count(); ++i)
        total_sats += file.nav_point(i).satellite_count;
    CHECK(total_sats >= 1);

    // The event marker landed with its path (add(EventMarker) dispatched).
    REQUIRE(file.event_marker_count() == 1);
    CHECK(file.event_marker(0).variant_path == "power/boot");
}

#if defined(__GNUC__) && !defined(__clang__)
#pragma GCC diagnostic pop
#endif
