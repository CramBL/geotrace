#include <doctest/doctest.h>
#include <geotrace.h>
#include <geotrace/geotrace.hpp>

#include "test_undeclared_enums.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#if GEOTRACE_CPP_EXCEPTIONS
#include <stdexcept>
#endif
#include <string>
#include <vector>
#include <limits>
#include <utility>

using geotrace::Angle;
using geotrace::Constellation;
using geotrace::FileBuilder;
using geotrace::FixTime;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::PositionProvenance;
using geotrace::SatelliteCoverage;
using geotrace::SatelliteMembership;
using geotrace::SatelliteObservationReport;
using geotrace::SatelliteScope;
using geotrace::SatelliteUsageReport;
using geotrace::SolutionDimension;
using geotrace::SolutionMethod;
using geotrace::SolutionState;
using geotrace::Timestamp;

namespace {
enum class InvalidField : std::uint8_t {
    State,
    Method,
    Dimension,
    Provenance,
    Coverage,
    Membership,
    Constellation
};

NavFix source(std::int64_t seconds) {
    return NavFix{FixTime::host(Timestamp::from_seconds(1700000000 + seconds)), Angle::degrees(0),
                  Angle::degrees(static_cast<double>(seconds))};
}
} // namespace

TEST_CASE("navigation facts preserve absence, explicit state and contradictory counts") {
    auto fix = source(0);
    fix.state = SolutionState::NoFix;
    fix.method = SolutionMethod::RtkFixed;
    fix.dimension = SolutionDimension::ThreeD;
    fix.used_count = 12;
    fix.hdop = 0.9;
    fix.vdop = 1.1;
    fix.pdop = 1.2;
    FileBuilder builder;
    builder.add(fix);
    fix.time = source(1).time;
    fix.state = SolutionState::Unknown;
    fix.position_provenance = std::nullopt;
    fix.used_count = 0;
    builder.add(fix);
    builder.add(source(2));
    fix.time = source(3).time;
    fix.state = SolutionState::Fix;
    fix.used_count = std::numeric_limits<std::uint32_t>::max();
    builder.add(fix);
    auto result = builder.try_finish();
    REQUIRE(result.is_ok());
    auto decoded = NavFile::try_from_bytes(result.value().to_bytes());
    REQUIRE(decoded.is_ok());
    const auto first = decoded.value().try_nav_point(0).value();
    CHECK(first.state == SolutionState::NoFix);
    CHECK(first.method == SolutionMethod::RtkFixed);
    CHECK(first.dimension == SolutionDimension::ThreeD);
    CHECK(first.position_provenance == PositionProvenance::SourceProvided);
    CHECK(first.used_count == 12);
    CHECK(first.hdop == 0.9);
    CHECK(first.vdop == 1.1);
    CHECK(first.pdop == 1.2);
    CHECK_FALSE(first.heading);
    CHECK_FALSE(first.eph_m);
    CHECK(first.observation_report_count == 0);
    const auto unknown = decoded.value().try_nav_point(1).value();
    CHECK(unknown.state == SolutionState::Unknown);
    CHECK_FALSE(unknown.position_provenance);
    CHECK(unknown.used_count == 0);
    const auto absent = decoded.value().try_nav_point(2).value();
    CHECK_FALSE(absent.state);
    CHECK_FALSE(absent.used_count);
    CHECK_FALSE(absent.hdop);
    CHECK(decoded.value().try_nav_point(3).value().used_count ==
          std::numeric_limits<std::uint32_t>::max());
}

TEST_CASE("scope construction normalizes declaration order and preserves global distinction") {
    const std::array values{Constellation::Qzss,    Constellation::Gps,    Constellation::Glonass,
                            Constellation::Galileo, Constellation::Beidou, Constellation::Navic,
                            Constellation::Gps};
    const auto scope = SatelliteScope::try_constellations(values);
    REQUIRE(scope.is_ok());
    CHECK_FALSE(scope.value().is_global());
    CHECK(scope.value() != SatelliteScope::global());
    CHECK(scope.value().constellations() ==
          std::vector<Constellation>{Constellation::Gps, Constellation::Glonass,
                                     Constellation::Galileo, Constellation::Beidou,
                                     Constellation::Navic, Constellation::Qzss});
    const std::array single{Constellation::Gps};
    const std::array scopes{SatelliteScope::global(), SatelliteScope::constellations(single),
                            scope.value()};
    FileBuilder builder;
    builder.add(source(0));
    for (const auto &declared_scope : scopes) {
        builder.add(
            SatelliteUsageReport{source(0).time, declared_scope, SatelliteCoverage::Complete});
    }
    auto file = builder.try_finish();
    REQUIRE(file.is_ok());
    const auto decoded = NavFile::try_from_bytes(file.value().to_bytes());
    REQUIRE(decoded.is_ok());
    const auto reports = decoded.value().try_satellite_usage_reports(0).value();
    REQUIRE(reports.size() == scopes.size());
    for (std::size_t index = 0; index < scopes.size(); ++index) {
        CHECK(reports.at(index).scope == scopes.at(index));
    }
    CHECK(SatelliteScope::try_constellations({}).is_err());
    const std::array invalid{undeclared_enum_value<Constellation>()};
    CHECK(SatelliteScope::try_constellations(invalid).error().description.find(
              "scope.constellations") != std::string::npos);
}

TEST_CASE("independent reports retain ordered rows, clocks, scopes and owned readback") {
    const std::array gps{Constellation::Gps};
    SatelliteUsageReport usage{source(0).time,
                               SatelliteScope::constellations(gps),
                               SatelliteCoverage::Partial,
                               8,
                               {{{Constellation::Gps, 1}, SatelliteMembership::Unused},
                                {{Constellation::Gps, 1}, SatelliteMembership::Used},
                                {{Constellation::Gps, 2}, SatelliteMembership::Unknown}}};
    SatelliteObservationReport observations{source(0).time,
                                            SatelliteScope::global(),
                                            SatelliteCoverage::Complete,
                                            12,
                                            {{{Constellation::Gps, 1}, 0.0F, std::nullopt, 0.0F}}};
    std::vector<SatelliteUsageReport> retained;
    {
        FileBuilder builder;
        builder.add(source(0)).add(usage).add_satellite_observation_report(observations);
        usage.time = FixTime::both(Timestamp::from_millis(1700000000100),
                                   Timestamp::from_millis(1700000000200));
        usage.reported_count = 0;
        usage.coverage = SatelliteCoverage::Complete;
        usage.satellites.clear();
        builder.add_satellite_usage_report(usage);
        observations.satellites.clear();
        observations.reported_count = std::nullopt;
        observations.coverage = SatelliteCoverage::Partial;
        builder.add(observations);
        auto moved_builder = std::move(builder);
        auto built = moved_builder.try_finish();
        REQUIRE(built.is_ok());
        auto file = std::move(built.value());
        auto decoded = NavFile::try_from_bytes(file.to_bytes());
        REQUIRE(decoded.is_ok());
        auto moved_file = std::move(decoded.value());
        const auto point = moved_file.try_nav_point(0).value();
        CHECK(point.usage_report_count == 2);
        CHECK(point.observation_report_count == 2);
        CHECK(point.satellite_count == 0);
        CHECK_FALSE(point.satellite_report_sys_time);
        retained = moved_file.try_satellite_usage_reports(0).value();
        REQUIRE(retained.size() == 2);
        CHECK(retained.at(0).satellites.size() == 3);
        CHECK(retained.at(0).satellites.at(0).membership == SatelliteMembership::Unused);
        CHECK(retained.at(0).satellites.at(1).membership == SatelliteMembership::Used);
        CHECK(retained.at(0).satellites.at(2).membership == SatelliteMembership::Unknown);
        CHECK(retained.at(0).reported_count == 8);
        CHECK(retained.at(0).scope == SatelliteScope::constellations(gps));
        CHECK(retained.at(1).satellites.empty());
        CHECK(retained.at(1).coverage == SatelliteCoverage::Complete);
        CHECK(retained.at(1).reported_count == 0);
        CHECK(retained.at(1).time.gps_time() == usage.time.gps_time());
        CHECK(retained.at(1).time.sys_time() == usage.time.sys_time());
        CHECK(retained.at(0).placement->input_order == 0);
        CHECK(retained.at(1).placement->input_order == 2);
        const auto seen = moved_file.try_satellite_observation_reports(0).value();
        CHECK(seen.at(0).scope.is_global());
        CHECK(seen.at(0).reported_count == 12);
        CHECK(seen.at(0).satellites.at(0).elevation_deg == 0.0F);
        CHECK_FALSE(seen.at(0).satellites.at(0).azimuth_deg);
        CHECK(seen.at(0).satellites.at(0).snr_dbhz == 0.0F);
        CHECK_FALSE(seen.at(1).reported_count);
        CHECK(seen.at(1).coverage == SatelliteCoverage::Partial);
        CHECK(seen.at(1).satellites.empty());
        CHECK(moved_file.try_satellite_usage({0, 0, 2}).value() == retained.at(0).satellites.at(2));
        CHECK(moved_file.try_satellite_observation({0, 0, 0}).value() ==
              seen.at(0).satellites.at(0));
#if GEOTRACE_CPP_EXCEPTIONS
        CHECK(moved_file.satellite_usage_reports(0) == retained);
        CHECK(moved_file.satellite_usage_report({0, 0}) == retained.at(0));
        CHECK(moved_file.satellite_usage({0, 0, 0}) == retained.at(0).satellites.at(0));
        CHECK(moved_file.satellite_observation_reports(0) == seen);
        CHECK(moved_file.satellite_observation_report({0, 0}) == seen.at(0));
        CHECK(moved_file.satellite_observation({0, 0, 0}) == seen.at(0).satellites.at(0));
#endif
        CHECK(moved_file.try_satellite_usage_report({0, 9}).error().code == GTD_ERR_OUT_OF_RANGE);
        CHECK(moved_file.try_satellite_observation_report({9, 0}).error().code ==
              GTD_ERR_OUT_OF_RANGE);
        CHECK(moved_file.try_satellite_usage({0, 0, 9}).error().code == GTD_ERR_OUT_OF_RANGE);
        CHECK(moved_file.try_satellite_observation({0, 9, 0}).error().code == GTD_ERR_OUT_OF_RANGE);
        CHECK(moved_file.try_satellite_usage_reports(9).error().code == GTD_ERR_OUT_OF_RANGE);
        CHECK(moved_file.try_satellite_observation_reports(9).error().code == GTD_ERR_OUT_OF_RANGE);
#if GEOTRACE_CPP_EXCEPTIONS
        CHECK_THROWS_AS(static_cast<void>(moved_file.satellite_usage_report({0, 9})),
                        std::out_of_range);
        CHECK_THROWS_AS(static_cast<void>(moved_file.satellite_observation({0, 0, 9})),
                        std::out_of_range);
#endif
    }
    CHECK(retained.at(0).satellites.at(1).membership == SatelliteMembership::Used);
    auto changed = retained.at(0);
    CHECK(changed == retained.at(0));
    changed.placement->input_order += 1;
    CHECK(changed != retained.at(0));
    changed = retained.at(0);
    changed.reported_count = 9;
    CHECK(changed != retained.at(0));
}

TEST_CASE("host polling preserves the orphan timestamp and interpolated position") {
    SatelliteObservationReport report{
        source(2).time,
        SatelliteScope::global(),
        SatelliteCoverage::Partial,
        12,
        {{{Constellation::Gps, 1}, std::nullopt, std::nullopt, 40.0F}}};
    auto built = FileBuilder{}.add(source(0)).add(source(10)).add(report).try_finish();
    REQUIRE(built.is_ok());
    const auto file = NavFile::try_from_bytes(built.value().to_bytes());
    REQUIRE(file.is_ok());
    REQUIRE(file.value().nav_point_count() == 3);
    const auto carrier = file.value().try_nav_point(1).value();
    CHECK(carrier.gps_time == source(2).time.sys_time());
    CHECK_FALSE(carrier.sys_time);
    CHECK(carrier.position_provenance == PositionProvenance::SdkSynthesized);
    CHECK(carrier.lon.as_degrees() == doctest::Approx(2));
    const auto poll = file.value().try_satellite_observation_report({1, 0}).value();
    CHECK(poll.time.sys_time() == source(2).time.sys_time());
    CHECK_FALSE(poll.time.gps_time());
    CHECK(poll.placement->normalized_time == *source(2).time.sys_time());
    CHECK(poll.placement->input_order == 0);
    CHECK_FALSE(carrier.satellite_report_sys_time);
}

TEST_CASE("invalid navigation and report enum values preserve the first builder error") {
    for (const auto field : {InvalidField::State, InvalidField::Method, InvalidField::Dimension,
                             InvalidField::Provenance, InvalidField::Coverage,
                             InvalidField::Membership, InvalidField::Constellation}) {
        FileBuilder builder;
        builder.add(source(0));
        auto fix = source(1);
        SatelliteUsageReport usage{source(0).time, SatelliteScope::global(),
                                   SatelliteCoverage::Complete};
        std::string expected;
        switch (field) {
        case InvalidField::State:
            fix.state = undeclared_enum_value<SolutionState>();
            expected = "nav_fix.state";
            break;
        case InvalidField::Method:
            fix.method = undeclared_enum_value<SolutionMethod>();
            expected = "nav_fix.method";
            break;
        case InvalidField::Dimension:
            fix.dimension = undeclared_enum_value<SolutionDimension>();
            expected = "nav_fix.dimension";
            break;
        case InvalidField::Provenance:
            fix.position_provenance = undeclared_enum_value<PositionProvenance>();
            expected = "nav_fix.position_provenance";
            break;
        case InvalidField::Coverage:
            usage.coverage = undeclared_enum_value<SatelliteCoverage>();
            expected = "report.coverage";
            break;
        case InvalidField::Membership:
            usage.satellites = {
                {{Constellation::Gps, 1}, SatelliteMembership::Used},
                {{Constellation::Gps, 2}, undeclared_enum_value<SatelliteMembership>()}};
            expected = "usage report row 1";
            break;
        case InvalidField::Constellation:
            usage.satellites = {
                {{Constellation::Gps, 1}, SatelliteMembership::Used},
                {{undeclared_enum_value<Constellation>(), 2}, SatelliteMembership::Used}};
            expected = "usage report row 1";
            break;
        }
#if GEOTRACE_CPP_EXCEPTIONS
        if (field < InvalidField::Coverage) {
            CHECK_THROWS_AS(builder.add(fix), std::invalid_argument);
        } else {
            CHECK_THROWS_AS(builder.add(usage), std::invalid_argument);
        }
#else
        if (field < InvalidField::Coverage) {
            builder.add(fix);
        } else {
            builder.add(usage);
        }
#endif
        CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
        CHECK(builder.status().description.find(expected) != std::string::npos);
        const auto first_error = builder.status().description;
        builder.add(fix);
        CHECK(builder.status().description == first_error);
        CHECK(builder.try_finish().error().description == first_error);
    }
}

TEST_CASE("a late observation identity error records its row index") {
    SatelliteObservationReport report{
        source(0).time,
        SatelliteScope::global(),
        SatelliteCoverage::Partial,
        std::nullopt,
        {{{Constellation::Gps, 1}}, {{undeclared_enum_value<Constellation>(), 2}}}};
    FileBuilder builder;
    builder.add(source(0));
#if GEOTRACE_CPP_EXCEPTIONS
    CHECK_THROWS_AS(builder.add(report), std::invalid_argument);
#else
    builder.add(report);
#endif
    CHECK(builder.status().code == GTD_ERR_INVALID_ARGUMENT);
    CHECK(builder.status().description.find("observation report row 1") != std::string::npos);
    CHECK(builder.try_finish().error().description == builder.status().description);
}
