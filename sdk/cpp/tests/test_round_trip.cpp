#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <optional>
#include <type_traits>
#include <vector>

using geotrace::Angle;
using geotrace::Constellation;
using geotrace::EventMarker;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Satellite;
using geotrace::SatelliteReport;
using geotrace::SatelliteView;
using geotrace::Timestamp;
using geotrace::travel_mode_from_name;
using geotrace::travel_mode_name;
using geotrace::TravelMode;
using geotrace::Velocity;

static_assert(std::is_same_v<decltype(Satellite::elevation_deg), std::optional<float>>);
static_assert(std::is_same_v<decltype(Satellite::azimuth_deg), std::optional<float>>);
static_assert(std::is_same_v<decltype(Satellite::snr_dbhz), std::optional<float>>);
static_assert(std::is_same_v<decltype(SatelliteView::elevation_deg), std::optional<float>>);
static_assert(std::is_same_v<decltype(SatelliteView::azimuth_deg), std::optional<float>>);
static_assert(std::is_same_v<decltype(SatelliteView::snr_dbhz), std::optional<float>>);

static constexpr double LAT = 51.5074;
static constexpr double LON = -0.1278;
static constexpr double LAT2 = 51.5080;
static constexpr double LON2 = -0.1265;
static const Timestamp FIRST_TIME = Timestamp::from_seconds(1700000000ULL);
static const Timestamp SECOND_TIME = Timestamp::from_seconds(1700000010ULL);

TEST_CASE("round-trip: nav fix fields survive write → from_bytes → read") {
    std::vector<std::uint8_t> bytes;
    {
        NavFix first_fix{FixTime::receiver(FIRST_TIME), Angle::degrees(LAT), Angle::degrees(LON)};
        first_fix.heading = Angle::degrees(270.0);
        first_fix.speed = Velocity::mps(5.5);
        first_fix.eph_m = 3.2;

        const NavFix second_fix{FixTime::receiver(SECOND_TIME), Angle::degrees(LAT2),
                                Angle::degrees(LON2)};

        auto file = FileBuilder{}.add_nav_fix(first_fix).add_nav_fix(second_fix).finish();
        bytes = file.to_bytes();
    }

    CHECK_FALSE(bytes.empty());

    auto file2 = NavFile::from_bytes(bytes);
    REQUIRE(file2.nav_point_count() == 2);

    auto first_point = file2.nav_point(0);
    CHECK(first_point.lat.as_degrees() == doctest::Approx(LAT).epsilon(1e-6));
    CHECK(first_point.lon.as_degrees() == doctest::Approx(LON).epsilon(1e-6));
    REQUIRE(first_point.heading.has_value());
    CHECK(first_point.heading.value().as_degrees() == doctest::Approx(270.0).epsilon(1e-4));
    REQUIRE(first_point.speed.has_value());
    CHECK(first_point.speed.value().as_mps() == doctest::Approx(5.5).epsilon(1e-4));
    REQUIRE(first_point.eph_m.has_value());
    CHECK(first_point.eph_m.value() == doctest::Approx(3.2).epsilon(1e-4));
    REQUIRE(first_point.gps_time.has_value());
    CHECK(first_point.gps_time->unix_micros == FIRST_TIME.unix_micros);
    CHECK_FALSE(first_point.sys_time.has_value());

    auto second_point = file2.nav_point(1);
    CHECK(second_point.lat.as_degrees() == doctest::Approx(LAT2).epsilon(1e-6));
    CHECK_FALSE(second_point.heading.has_value());
}

TEST_CASE("round-trip: satellite report survives write → from_bytes → read") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(LAT), Angle::degrees(LON)};

    Satellite gps_satellite{};
    gps_satellite.constellation = Constellation::Gps;
    gps_satellite.prn = 12;
    gps_satellite.in_fix = true;
    gps_satellite.elevation_deg = 60.0F;
    gps_satellite.azimuth_deg = 200.0F;
    gps_satellite.snr_dbhz = 42.0F;

    Satellite galileo_satellite{};
    galileo_satellite.constellation = Constellation::Galileo;
    galileo_satellite.prn = 5;
    galileo_satellite.in_fix = false;
    galileo_satellite.snr_dbhz = 25.0F;

    const SatelliteReport report{FixTime::receiver(FIRST_TIME), {gps_satellite, galileo_satellite}};

    auto file = FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    auto point = file2.nav_point(0);
    CHECK(point.satellite_count == 2);

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

TEST_CASE("round-trip: satellite metrics survive write → from_bytes → read bit-exact") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(LAT), Angle::degrees(LON)};

    Satellite satellite{};
    satellite.constellation = Constellation::Gps;
    satellite.prn = 7;
    satellite.in_fix = true;
    satellite.elevation_deg = 38.5F;
    satellite.azimuth_deg = 359.9999F;
    satellite.snr_dbhz = 38.123456789F;

    const SatelliteReport report{FixTime::receiver(FIRST_TIME), {satellite}};

    auto file = FileBuilder{}.add_nav_fix(fix).add_satellite_report(report).finish();

    auto bytes = file.to_bytes();
    auto reread = NavFile::from_bytes(bytes);

    const auto view = reread.satellite(0, 0);
    REQUIRE(view.elevation_deg.has_value());
    REQUIRE(view.azimuth_deg.has_value());
    REQUIRE(view.snr_dbhz.has_value());
    CHECK(view.elevation_deg.value() == 38.5F);
    CHECK(view.azimuth_deg.value() == 359.9999F);
    CHECK(view.snr_dbhz.value() == 38.123456789F);
}

TEST_CASE("round-trip: event marker survives write → from_bytes → read") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(LAT), Angle::degrees(LON)};

    const EventMarker m_in{"engine/start", FIRST_TIME, "Engine started"};

    auto file = FileBuilder{}.add_nav_fix(fix).add_event_marker(m_in).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    REQUIRE(file2.event_marker_count() == 1);
    auto m_out = file2.event_marker(0);
    CHECK(m_out.variant_path == "engine/start");
    CHECK(m_out.annotation == "Engine started");
    CHECK(m_out.sys_time.unix_micros == FIRST_TIME.unix_micros);
}

TEST_CASE("round-trip: metadata survives write → to_bytes → from_bytes") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(0.0), Angle::degrees(0.0)};

    auto file = FileBuilder{}.title("test title").device("test device").add_nav_fix(fix).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    CHECK(file2.title() == "test title");
    CHECK(file2.device() == "test device");
}

TEST_CASE("round-trip: travel mode survives write → to_bytes → from_bytes") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(0.0), Angle::degrees(0.0)};

    auto file = FileBuilder{}.travel_mode(TravelMode::Rail).add_nav_fix(fix).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    CHECK(file2.travel_mode() == "rail");
    CHECK(travel_mode_from_name(std::string{file2.travel_mode()}) == TravelMode::Rail);
}

TEST_CASE("a build without provenance writes only the sdk version") {
    const NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(LAT), Angle::degrees(LON)};

    auto file = NavFile::from_bytes(FileBuilder{}.add_nav_fix(fix).finish().to_bytes());

    CHECK(file.sdk_version() == GEOTRACE_CPP_VERSION);
    CHECK(file.sdk_git_commit().empty());
    CHECK_FALSE(file.sdk_commit_time().has_value());
}

TEST_CASE("travel mode names round-trip through travel_mode_from_name") {
    for (auto mode :
         {TravelMode::Car, TravelMode::Motorcycle, TravelMode::Bicycle, TravelMode::Pedestrian,
          TravelMode::Boat, TravelMode::Rail, TravelMode::Aircraft}) {
        auto name = travel_mode_name(mode);
        CHECK_FALSE(name.empty());
        CHECK(travel_mode_from_name(std::string{name}) == mode);
    }
    CHECK(travel_mode_from_name("hovercraft") == std::nullopt);
}

TEST_CASE("round-trip: velocity unit conversions are consistent") {
    NavFix fix{FixTime::receiver(FIRST_TIME), Angle::degrees(LAT), Angle::degrees(LON)};
    fix.speed = Velocity::kmh(72.0); // 20 m/s

    auto file = FileBuilder{}.add_nav_fix(fix).finish();

    auto bytes = file.to_bytes();
    auto file2 = NavFile::from_bytes(bytes);

    auto point = file2.nav_point(0);
    REQUIRE(point.speed.has_value());
    const auto speed = point.speed.value();
    CHECK(speed.as_mps() == doctest::Approx(20.0).epsilon(0.01));
    CHECK(speed.as_kmh() == doctest::Approx(72.0).epsilon(0.05));
    CHECK(speed.as_knots() == doctest::Approx(38.88).epsilon(0.1));
}
