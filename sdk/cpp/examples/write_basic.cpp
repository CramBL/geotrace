/**
 * Write a basic .gtd file from a hardcoded GPS track.
 *
 * The minimal write workflow: create a builder, set some metadata, add a few
 * nav fixes (plus an optional satellite report and a map annotation), call
 * finish(), and write the result to disk.
 */

#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <exception>
#include <filesystem>
#include <iostream>

namespace {
// 2024-06-01T08:00:00Z keeps the output deterministic.
constexpr std::int64_t kBase = 1717228800;
} // namespace

int main() {
    try {
        const auto first_fix_time = geotrace::Timestamp::from_seconds(kBase);
        const auto second_fix_time = geotrace::Timestamp::from_seconds(kBase + 10);

        geotrace::FileBuilder builder{};
        builder.title("Quick tour").device("Example GPS v1.0");

        geotrace::NavFix first_fix{geotrace::FixTime::receiver(first_fix_time),
                                   geotrace::Angle::degrees(51.5074),
                                   geotrace::Angle::degrees(-0.1278)};
        first_fix.heading = geotrace::Angle::degrees(90.0);
        first_fix.speed = geotrace::Velocity::mps(5.5);
        first_fix.eph_m = 3.2;
        builder.add(first_fix);

        geotrace::Satellite gps_prn1{};
        gps_prn1.constellation = geotrace::Constellation::Gps;
        gps_prn1.prn = 1;
        gps_prn1.in_fix = true;
        gps_prn1.elevation_deg = 45.0F;
        gps_prn1.azimuth_deg = 90.0F;
        gps_prn1.snr_dbhz = 38.0F;

        // Elevation and azimuth are optional. A receiver reports an SNR for a
        // satellite whose position it has not computed.
        geotrace::Satellite galileo_prn3{};
        galileo_prn3.constellation = geotrace::Constellation::Galileo;
        galileo_prn3.prn = 3;
        galileo_prn3.in_fix = false;
        galileo_prn3.snr_dbhz = 22.0F;

        builder.add(geotrace::SatelliteReport{geotrace::FixTime::receiver(first_fix_time),
                                              {gps_prn1, galileo_prn3}});

        geotrace::NavFix second_fix{geotrace::FixTime::receiver(second_fix_time),
                                    geotrace::Angle::degrees(51.5080),
                                    geotrace::Angle::degrees(-0.1265)};
        second_fix.heading = geotrace::Angle::degrees(85.0);
        second_fix.speed = geotrace::Velocity::mps(5.8);
        builder.add(second_fix);

        builder.add(geotrace::Annotation{first_fix_time, "Start point", geotrace::MarkerIcon::Pin});

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_write_basic.gtd";
        file.write_to_file(out);
        std::cout << "Wrote " << file.nav_point_count() << " nav points to " << out.string()
                  << "\n";

        std::filesystem::remove(out);
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
