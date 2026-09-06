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

#include <array>
#include <cstddef>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <iostream>

namespace {
// 2024-06-01T08:00:00Z. File-scope so the `at` lambda can read it without a
// capture (MSVC C++17 requires capturing a local).
constexpr std::int64_t kBase = 1717228800;
} // namespace

int main() {
    auto timestamp_at = [](std::int64_t secs) {
        return geotrace::Timestamp::from_seconds(kBase + secs);
    };

    try {
        geotrace::FileBuilder builder{};
        builder.title("Satellite quality tour").device("Example GNSS v1.0");

        struct TrackPoint {
            double lat;
            double lon;
        };
        const std::array<TrackPoint, 4> track = {{
            {51.5074, -0.1278},
            {51.5080, -0.1265},
            {51.5088, -0.1248},
            {51.5095, -0.1233},
        }};
        std::int64_t idx = 0;
        for (const auto &point : track) {
            const geotrace::Timestamp time = timestamp_at(idx);

            geotrace::NavFix fix{geotrace::FixTime::receiver(time),
                                 geotrace::Angle::degrees(point.lat),
                                 geotrace::Angle::degrees(point.lon)};
            fix.heading = geotrace::Angle::degrees(90.0);
            fix.speed = geotrace::Velocity::mps(5.5);
            builder.add(fix);

            // SNR climbs slightly each second as the receiver settles.
            const float snr = 36.0F + static_cast<float>(idx);

            geotrace::Satellite gps_prn1{};
            gps_prn1.constellation = geotrace::Constellation::Gps;
            gps_prn1.prn = 1;
            gps_prn1.in_fix = true;
            gps_prn1.elevation_deg = 45.0F;
            gps_prn1.azimuth_deg = 90.0F;
            gps_prn1.snr_dbhz = snr;

            geotrace::Satellite gps_prn5{};
            gps_prn5.constellation = geotrace::Constellation::Gps;
            gps_prn5.prn = 5;
            gps_prn5.in_fix = true;
            gps_prn5.snr_dbhz = snr - 2.0F;

            geotrace::Satellite galileo_prn3{};
            galileo_prn3.constellation = geotrace::Constellation::Galileo;
            galileo_prn3.prn = 3;
            galileo_prn3.in_fix = false;
            galileo_prn3.snr_dbhz = 21.0F;

            builder.add(geotrace::SatelliteReport{geotrace::FixTime::receiver(time),
                                                  {gps_prn1, gps_prn5, galileo_prn3}});
            ++idx;
        }

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_with_satellites.gtd";
        file.write_to_file(out);

        const geotrace::NavFile loaded = geotrace::NavFile::open(out);
        std::cout << loaded.nav_point_count() << " nav point(s)\n";
        for (std::size_t i = 0; i < loaded.nav_point_count(); ++i) {
            const auto point = loaded.nav_point(i);
            std::size_t in_fix = 0;
            for (std::size_t j = 0; j < point.satellite_count; ++j) {
                if (loaded.satellite(i, j).in_fix) {
                    ++in_fix;
                }
            }
            std::cout << "  [" << i << "] " << point.satellite_count << " tracked, " << in_fix
                      << " in fix\n";
        }

        std::filesystem::remove(out);
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
