#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <vector>

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
        NavFix f1{};
        f1.gps_time = T0;
        f1.lat = Angle::degrees(LAT);
        f1.lon = Angle::degrees(LON);
        f1.heading = Angle::degrees(270.0);
        f1.speed = Velocity::mps(5.5);
        f1.eph_m = 3.2;

        NavFix f2{};
        f2.gps_time = T1;
        f2.lat = Angle::degrees(LAT2);
        f2.lon = Angle::degrees(LON2);

        auto file = FileBuilder{}.add_nav_fix(f1).add_nav_fix(f2).finish();
        bytes = file.to_bytes();
    }

    CHECK_FALSE(bytes.empty());

    auto file2 = NavFile::from_bytes(bytes);
    REQUIRE(file2.nav_point_count() == 2);

    auto p0 = file2.nav_point(0);
    CHECK(p0.lat.as_degrees() == doctest::Approx(LAT).epsilon(1e-6));
    CHECK(p0.lon.as_degrees() == doctest::Approx(LON).epsilon(1e-6));
    REQUIRE(p0.heading.has_value());
    CHECK(p0.heading.value().as_degrees() == doctest::Approx(270.0).epsilon(1e-4));
    REQUIRE(p0.speed.has_value());
    CHECK(p0.speed.value().as_mps() == doctest::Approx(5.5).epsilon(1e-4));
    REQUIRE(p0.eph_m.has_value());
    CHECK(p0.eph_m.value() == doctest::Approx(3.2).epsilon(1e-4));
    CHECK_FALSE(p0.gps_time.is_none());
    CHECK(p0.gps_time.unix_micros == T0.unix_micros);

    auto p1 = file2.nav_point(1);
    CHECK(p1.lat.as_degrees() == doctest::Approx(LAT2).epsilon(1e-6));
    CHECK_FALSE(p1.heading.has_value());
}

TEST_CASE("round-trip: satellite report survives write → from_bytes → read") {
    NavFix fix{};
    fix.gps_time = T0;
    fix.lat = Angle::degrees(LAT);
    fix.lon = Angle::degrees(LON);

    Satellite s1{};
    s1.constellation = Constellation::Gps;
    s1.prn = 12;
    s1.in_fix = true;
    s1.elevation_deg = 60.0;
    s1.azimuth_deg = 200.0;
    s1.snr_dbhz = 42.0;

    Satellite s2{};
    s2.constellation = Constellation::Galileo;
    s2.prn = 5;
    s2.in_fix = false;
    s2.snr_dbhz = 25.0;

    SatelliteReport report{};
    report.gps_time = T0;
    report.tracked.push_back(s1);
    report.tracked.push_back(s2);

    auto file = FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    auto p = file2.nav_point(0);
    CHECK(p.satellite_count == 2);

    auto s0_out = file2.satellite(0, 0);
    CHECK(s0_out.constellation == Constellation::Gps);
    CHECK(s0_out.prn == 12);
    CHECK(s0_out.in_fix);
    REQUIRE(s0_out.elevation_deg.has_value());
    CHECK(s0_out.elevation_deg.value() == doctest::Approx(60.0).epsilon(0.5));
    REQUIRE(s0_out.snr_dbhz.has_value());
    CHECK(s0_out.snr_dbhz.value() == doctest::Approx(42.0).epsilon(0.5));

    auto s1_out = file2.satellite(0, 1);
    CHECK(s1_out.constellation == Constellation::Galileo);
    CHECK_FALSE(s1_out.in_fix);
}

TEST_CASE("round-trip: event marker survives write → from_bytes → read") {
    NavFix fix{};
    fix.gps_time = T0;
    fix.lat = Angle::degrees(LAT);
    fix.lon = Angle::degrees(LON);

    EventMarker m_in{};
    m_in.variant_path = "engine/start";
    m_in.sys_time = T0;
    m_in.annotation = "Engine started";

    auto file = FileBuilder{}.add_nav_fix(fix).add_event_marker(m_in).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    REQUIRE(file2.event_marker_count() == 1);
    auto m_out = file2.event_marker(0);
    CHECK(m_out.variant_path == "engine/start");
    CHECK(m_out.annotation == "Engine started");
    CHECK_FALSE(m_out.sys_time.is_none());
    CHECK(m_out.sys_time.unix_micros == T0.unix_micros);
}

TEST_CASE("round-trip: metadata survives write → to_bytes → from_bytes") {
    NavFix fix{};
    fix.gps_time = T0;
    fix.lat = Angle::degrees(0.0);
    fix.lon = Angle::degrees(0.0);

    auto file = FileBuilder{}.title("test title").device("test device").add_nav_fix(fix).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    CHECK(file2.title() == "test title");
    CHECK(file2.device() == "test device");
}

TEST_CASE("round-trip: velocity unit conversions are consistent") {
    NavFix fix{};
    fix.gps_time = T0;
    fix.lat = Angle::degrees(LAT);
    fix.lon = Angle::degrees(LON);
    fix.speed = Velocity::kmh(72.0); // 20 m/s

    auto file = FileBuilder{}.add_nav_fix(fix).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    auto p = file2.nav_point(0);
    REQUIRE(p.speed.has_value());
    const auto speed = p.speed.value();
    CHECK(speed.as_mps() == doctest::Approx(20.0).epsilon(0.01));
    CHECK(speed.as_kmh() == doctest::Approx(72.0).epsilon(0.05));
    CHECK(speed.as_knots() == doctest::Approx(38.88).epsilon(0.1));
}
