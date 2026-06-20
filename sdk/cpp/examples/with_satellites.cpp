/**
 * Write a .gtd file that pairs each GPS fix with a satellite visibility report.
 *
 * A satellite report is a snapshot of every tracked satellite at one instant:
 * its constellation, PRN, whether it contributed to the fix, and signal quality
 * (elevation, azimuth, SNR).  Reports are matched to the nearest fix, so giving
 * each report the same timestamp as its fix keeps them aligned.
 *
 * The example writes the file, reads it back, and prints per-fix satellite
 * counts - the data GeoTrace shows in its sky view.
 */

#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <iostream>

namespace {
// 2024-06-01T08:00:00Z. File-scope so the `at` lambda can read it without a
// capture (MSVC C++17 requires capturing a local).
constexpr std::uint64_t kBase = 1717228800;
} // namespace

int main() {
    auto at = [](std::uint64_t secs) { return geotrace::Timestamp::from_seconds(kBase + secs); };

    try {
        geotrace::FileBuilder builder{};
        builder.title("Satellite quality tour").device("Example GNSS v1.0");

        struct TrackPoint {
            double lat;
            double lon;
        };
        const TrackPoint track[] = {
            {51.5074, -0.1278},
            {51.5080, -0.1265},
            {51.5088, -0.1248},
            {51.5095, -0.1233},
        };
        std::size_t idx = 0;
        for (const auto &point : track) {
            const geotrace::Timestamp t = at(idx);

            geotrace::NavFix fix{};
            fix.gps_time = t;
            fix.lat = geotrace::Angle::degrees(point.lat);
            fix.lon = geotrace::Angle::degrees(point.lon);
            fix.heading = geotrace::Angle::degrees(90.0);
            fix.speed = geotrace::Velocity::mps(5.5);
            builder.add_nav_fix(fix);

            // SNR climbs slightly each second as the receiver settles.
            const double snr = 36.0 + static_cast<double>(idx);

            geotrace::SatelliteReport report{};
            report.gps_time = t;

            geotrace::Satellite g1{};
            g1.constellation = geotrace::Constellation::Gps;
            g1.prn = 1;
            g1.in_fix = true;
            g1.elevation_deg = 45.0;
            g1.azimuth_deg = 90.0;
            g1.snr_dbhz = snr;
            report.tracked.push_back(g1);

            geotrace::Satellite g5{};
            g5.constellation = geotrace::Constellation::Gps;
            g5.prn = 5;
            g5.in_fix = true;
            g5.snr_dbhz = snr - 2.0;
            report.tracked.push_back(g5);

            geotrace::Satellite e3{};
            e3.constellation = geotrace::Constellation::Galileo;
            e3.prn = 3;
            e3.in_fix = false;
            e3.snr_dbhz = 21.0;
            report.tracked.push_back(e3);

            builder.add_satellite_report(report);
            ++idx;
        }

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_with_satellites.gtd";
        file.write_to_file(out);

        const geotrace::NavFile loaded = geotrace::NavFile::open(out);
        std::cout << loaded.nav_point_count() << " nav point(s)\n";
        for (std::size_t i = 0; i < loaded.nav_point_count(); ++i) {
            const auto p = loaded.nav_point(i);
            std::size_t in_fix = 0;
            for (std::size_t j = 0; j < p.satellite_count; ++j) {
                if (loaded.satellite(i, j).in_fix)
                    ++in_fix;
            }
            std::cout << "  [" << i << "] " << p.satellite_count << " tracked, " << in_fix
                      << " in fix\n";
        }

        std::filesystem::remove(out);
    } catch (const geotrace::Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
