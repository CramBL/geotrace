#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

using geotrace::Angle;
using geotrace::Constellation;
using geotrace::EventMarker;
using geotrace::FileBuilder;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Satellite;
using geotrace::SatelliteReport;
using geotrace::Timestamp;
using geotrace::Velocity;

static constexpr double LAT = 51.5074;
static constexpr double LON = -0.1278;
static constexpr double LAT2 = 51.5080;
static constexpr double LON2 = -0.1265;
static const Timestamp T0 = Timestamp::from_seconds(1700000000ULL);
static const Timestamp T1 = Timestamp::from_seconds(1700000010ULL);

TEST_CASE("round-trip: nav fix fields survive write → from_bytes → read") {
    std::vector<std::uint8_t> bytes;
    {
        auto file = FileBuilder{}
                        .add_nav_fix(NavFix{
                            .gps_time = T0,
                            .lat = Angle::degrees(LAT),
                            .lon = Angle::degrees(LON),
                            .heading = Angle::degrees(270.0),
                            .speed = Velocity::mps(5.5),
                            .eph_m = 3.2,
                        })
                        .add_nav_fix(NavFix{
                            .gps_time = T1,
                            .lat = Angle::degrees(LAT2),
                            .lon = Angle::degrees(LON2),
                        })
                        .finish();
        bytes = file.to_bytes();
    }

    CHECK_FALSE(bytes.empty());

    auto file2 = NavFile::from_bytes(bytes);
    REQUIRE(file2.nav_point_count() == 2);

    auto p0 = file2.nav_point(0);
    CHECK(p0.lat.as_degrees() == doctest::Approx(LAT).epsilon(1e-6));
    CHECK(p0.lon.as_degrees() == doctest::Approx(LON).epsilon(1e-6));
    REQUIRE(p0.heading.has_value());
    CHECK(p0.heading->as_degrees() == doctest::Approx(270.0).epsilon(1e-4));
    REQUIRE(p0.speed.has_value());
    CHECK(p0.speed->as_mps() == doctest::Approx(5.5).epsilon(1e-4));
    REQUIRE(p0.eph_m.has_value());
    CHECK(*p0.eph_m == doctest::Approx(3.2).epsilon(1e-4));
    CHECK_FALSE(p0.gps_time.is_none());
    CHECK(p0.gps_time.unix_micros == T0.unix_micros);

    auto p1 = file2.nav_point(1);
    CHECK(p1.lat.as_degrees() == doctest::Approx(LAT2).epsilon(1e-6));
    CHECK_FALSE(p1.heading.has_value());
}

TEST_CASE("round-trip: satellite report survives write → from_bytes → read") {
    auto file = FileBuilder{}
                    .add_nav_fix(NavFix{
                        .gps_time = T0,
                        .lat = Angle::degrees(LAT),
                        .lon = Angle::degrees(LON),
                    })
                    .add_satellite_report(SatelliteReport{
                        .gps_time = T0,
                        .tracked =
                            {
                                Satellite{
                                    .constellation = Constellation::Gps,
                                    .prn = 12,
                                    .in_fix = true,
                                    .elevation_deg = 60.0,
                                    .azimuth_deg = 200.0,
                                    .snr_dbhz = 42.0,
                                },
                                Satellite{
                                    .constellation = Constellation::Galileo,
                                    .prn = 5,
                                    .in_fix = false,
                                    .snr_dbhz = 25.0,
                                },
                            },
                    })
                    .finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    auto p = file2.nav_point(0);
    CHECK(p.satellite_count == 2);

    auto s0 = file2.satellite(0, 0);
    CHECK(s0.constellation == Constellation::Gps);
    CHECK(s0.prn == 12);
    CHECK(s0.in_fix);
    REQUIRE(s0.elevation_deg.has_value());
    CHECK(*s0.elevation_deg == doctest::Approx(60.0).epsilon(0.5));
    REQUIRE(s0.snr_dbhz.has_value());
    CHECK(*s0.snr_dbhz == doctest::Approx(42.0).epsilon(0.5));

    auto s1 = file2.satellite(0, 1);
    CHECK(s1.constellation == Constellation::Galileo);
    CHECK_FALSE(s1.in_fix);
}

TEST_CASE("round-trip: event marker survives write → from_bytes → read") {
    auto file = FileBuilder{}
                    .add_nav_fix(NavFix{
                        .gps_time = T0,
                        .lat = Angle::degrees(LAT),
                        .lon = Angle::degrees(LON),
                    })
                    .add_event_marker(EventMarker{
                        .variant_path = "engine/start",
                        .sys_time = T0,
                        .annotation = "Engine started",
                    })
                    .finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    REQUIRE(file2.event_marker_count() == 1);
    auto m = file2.event_marker(0);
    CHECK(m.variant_path == "engine/start");
    CHECK(m.annotation == "Engine started");
    CHECK_FALSE(m.sys_time.is_none());
    CHECK(m.sys_time.unix_micros == T0.unix_micros);
}

TEST_CASE("round-trip: metadata survives write → to_bytes → from_bytes") {
    auto file = FileBuilder{}
                    .title("test title")
                    .device("test device")
                    .add_nav_fix(NavFix{
                        .gps_time = T0,
                        .lat = Angle::degrees(0.0),
                        .lon = Angle::degrees(0.0),
                    })
                    .finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    CHECK(file2.title() == "test title");
    CHECK(file2.device() == "test device");
}

TEST_CASE("round-trip: velocity unit conversions are consistent") {
    auto file = FileBuilder{}
                    .add_nav_fix(NavFix{
                        .gps_time = T0,
                        .lat = Angle::degrees(LAT),
                        .lon = Angle::degrees(LON),
                        .speed = Velocity::kmh(72.0), // 20 m/s
                    })
                    .finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    auto p = file2.nav_point(0);
    REQUIRE(p.speed.has_value());
    CHECK(p.speed->as_mps() == doctest::Approx(20.0).epsilon(0.01));
    CHECK(p.speed->as_kmh() == doctest::Approx(72.0).epsilon(0.05));
    CHECK(p.speed->as_knots() == doctest::Approx(38.88).epsilon(0.1));
}
