#include <geotrace/geotrace.hpp>

#if GEOTRACE_CPP_EXCEPTIONS
#include <exception>
#endif
#include <iostream>
#include <optional>

int main() {
#if GEOTRACE_CPP_EXCEPTIONS
    try {
#endif
        using geotrace::Angle;
        using geotrace::FileBuilder;
        using geotrace::FixTime;
        using geotrace::NavFix;
        using geotrace::SatelliteCoverage;
        using geotrace::SatelliteObservationReport;
        using geotrace::SatelliteScope;
        using geotrace::SolutionState;
        using geotrace::Timestamp;

        NavFix fix{FixTime::host(Timestamp::from_seconds(1700000000)), Angle::degrees(0),
                   Angle::degrees(0)};
        fix.state = SolutionState::Fix;
        fix.used_count = 8;
        fix.hdop = 0.9;
        FileBuilder builder;
        builder.add(fix);
        fix.time = FixTime::host(Timestamp::from_seconds(1700000010));
        fix.lon = Angle::degrees(10);
        builder.add(fix);

        SatelliteObservationReport poll{
            FixTime::host(Timestamp::from_seconds(1700000002)),
            SatelliteScope::global(),
            SatelliteCoverage::Partial,
            12,
            {{{geotrace::Constellation::Gps, 1}, std::nullopt, std::nullopt, 40.0F}}};
        builder.add(poll);
        const auto file = builder.try_finish();
        if (file.is_err()) {
            std::cerr << file.error().description << '\n';
            return 1;
        }
        const auto status = file.value().try_write_to_file("navigation_polling.gtd");
        if (status.is_err()) {
            std::cerr << status.description << '\n';
            return 1;
        }
        return 0;
#if GEOTRACE_CPP_EXCEPTIONS
    } catch (const std::exception &error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
#endif
}
