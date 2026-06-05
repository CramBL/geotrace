#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

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
    NavFile file = FileBuilder{}
                       .add_nav_fix(NavFix{
                           .gps_time = t0,
                           .lat = Angle::degrees(51.5074),
                           .lon = Angle::degrees(-0.1278),
                       })
                       .finish();

    CHECK(file.nav_point_count() == 1);
    auto p = file.nav_point(0);
    CHECK(p.lat.as_degrees() == doctest::Approx(51.5074));
    CHECK(p.lon.as_degrees() == doctest::Approx(-0.1278));
}

TEST_CASE("FileBuilder: metadata is preserved") {
    NavFile file = FileBuilder{}
                       .title("my track")
                       .device("test device")
                       .notes("some notes")
                       .identity("unit-test")
                       .add_nav_fix(NavFix{
                           .gps_time = t0,
                           .lat = Angle::degrees(0.0),
                           .lon = Angle::degrees(0.0),
                       })
                       .finish();

    CHECK(file.title() == "my track");
    CHECK(file.device() == "test device");
    CHECK(file.notes() == "some notes");
    CHECK(file.identity() == "unit-test");
}

TEST_CASE("FileBuilder: optional fields round-trip") {
    NavFile file = FileBuilder{}
                       .add_nav_fix(NavFix{
                           .gps_time = t0,
                           .lat = Angle::degrees(48.8566),
                           .lon = Angle::degrees(2.3522),
                           .heading = Angle::degrees(180.0),
                           .speed = Velocity::mps(10.0),
                           .eph_m = 5.0,
                       })
                       .finish();

    auto p = file.nav_point(0);
    REQUIRE(p.heading.has_value());
    CHECK(p.heading->as_degrees() == doctest::Approx(180.0));
    REQUIRE(p.speed.has_value());
    CHECK(p.speed->as_mps() == doctest::Approx(10.0));
    REQUIRE(p.eph_m.has_value());
    CHECK(*p.eph_m == doctest::Approx(5.0));
}

TEST_CASE("FileBuilder: no-optional nav fix has nullopt fields") {
    NavFile file = FileBuilder{}
                       .add_nav_fix(NavFix{
                           .gps_time = t0,
                           .lat = Angle::degrees(0.0),
                           .lon = Angle::degrees(0.0),
                       })
                       .finish();

    auto p = file.nav_point(0);
    CHECK_FALSE(p.heading.has_value());
    CHECK_FALSE(p.speed.has_value());
    CHECK_FALSE(p.eph_m.has_value());
}

TEST_CASE("FileBuilder: satellite report round-trips") {
    NavFile file = FileBuilder{}
                       .add_nav_fix(NavFix{
                           .gps_time = t0,
                           .lat = Angle::degrees(40.7128),
                           .lon = Angle::degrees(-74.0060),
                       })
                       .add_satellite_report(SatelliteReport{
                           .gps_time = t0,
                           .tracked =
                               {
                                   Satellite{
                                       .constellation = Constellation::Gps,
                                       .prn = 7,
                                       .in_fix = true,
                                       .elevation_deg = 55.0,
                                       .azimuth_deg = 120.0,
                                       .snr_dbhz = 40.0,
                                   },
                                   Satellite{
                                       .constellation = Constellation::Glonass,
                                       .prn = 2,
                                       .in_fix = false,
                                       .snr_dbhz = 28.0,
                                   },
                               },
                       })
                       .finish();

    auto p = file.nav_point(0);
    CHECK(p.satellite_count == 2);

    auto s0 = file.satellite(0, 0);
    CHECK(s0.constellation == Constellation::Gps);
    CHECK(s0.prn == 7);
    CHECK(s0.in_fix);
    REQUIRE(s0.snr_dbhz.has_value());
    CHECK(*s0.snr_dbhz == doctest::Approx(40.0));

    auto s1 = file.satellite(0, 1);
    CHECK(s1.constellation == Constellation::Glonass);
    CHECK_FALSE(s1.in_fix);
}

TEST_CASE("FileBuilder: event marker round-trips") {
    NavFile file = FileBuilder{}
                       .add_nav_fix(NavFix{
                           .gps_time = t0,
                           .lat = Angle::degrees(35.6762),
                           .lon = Angle::degrees(139.6503),
                       })
                       .add_event_marker(EventMarker{
                           .variant_path = "system/startup",
                           .sys_time = t0,
                           .annotation = "Device started",
                       })
                       .add_event_marker_style(EventMarkerStyle{
                           .variant_path = "system/startup",
                           .icon = MarkerIcon::Gear,
                           .color_hex = "#00FF00",
                       })
                       .finish();

    REQUIRE(file.event_marker_count() == 1);
    auto m = file.event_marker(0);
    CHECK(m.variant_path == "system/startup");
    CHECK(m.annotation == "Device started");
}

TEST_CASE("FileBuilder: fluent chain works end-to-end") {
    auto file =
        FileBuilder{}
            .device("chain test")
            .add_nav_fix(
                NavFix{.gps_time = t0, .lat = Angle::degrees(1.0), .lon = Angle::degrees(2.0)})
            .add_nav_fix(
                NavFix{.gps_time = t1, .lat = Angle::degrees(1.1), .lon = Angle::degrees(2.1)})
            .finish();

    CHECK(file.nav_point_count() == 2);
    CHECK(file.device() == "chain test");
}

TEST_CASE("FileBuilder: NoNavFixesError thrown when annotations exist but no fixes") {
    FileBuilder b;
    b.add_annotation(Annotation{.time = t0, .label = "unreachable"});
    CHECK_THROWS_AS(std::move(b).finish(), NoNavFixesError);
}

TEST_CASE("FileBuilder: InvalidPathError thrown for malformed variant path") {
    FileBuilder b;
    b.add_nav_fix(NavFix{.gps_time = t0, .lat = Angle::degrees(0.0), .lon = Angle::degrees(0.0)});
    CHECK_THROWS_AS(b.add_event_marker(EventMarker{
                        .variant_path = "bad path with spaces!",
                        .sys_time = t0,
                    }),
                    InvalidPathError);
}

TEST_CASE("FileBuilder: move semantics work") {
    FileBuilder b1;
    b1.add_nav_fix(NavFix{.gps_time = t0, .lat = Angle::degrees(0.0), .lon = Angle::degrees(0.0)});
    FileBuilder b2 = std::move(b1);
    auto file = std::move(b2).finish();
    CHECK(file.nav_point_count() == 1);
}
