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
#include <optional>
#include <vector>

namespace {

// 2024-06-01T08:00:00Z. File-scope so the `at` lambda can read it without a
// capture (MSVC C++17 requires capturing a local).
constexpr std::int64_t kBase = 1717228800;

struct TrackPoint {
    std::int64_t offset_s;
    double lat;
    double lon;
    double heading_deg;
    double speed_mps;
    double eph_m;
};

// A short urban loop through Southwark, London, one fix every 10 s.
constexpr std::array<TrackPoint, 6> kTrack = {{
    {0, 51.5030, -0.0978, 5.0, 0.0, 4.2},
    {10, 51.5038, -0.0975, 8.0, 3.1, 3.8},
    {20, 51.5045, -0.0971, 12.0, 4.4, 3.5},
    {30, 51.5053, -0.0966, 10.0, 4.6, 3.1},
    {40, 51.5060, -0.0961, 7.0, 4.4, 2.9},
    {50, 51.5067, -0.0957, 5.0, 3.8, 3.0},
}};

// A mixed GPS, Galileo and GLONASS sky: eight satellites, five in the fix.
// GLONASS 5 has an SNR and no elevation or azimuth. A receiver reports that for
// a satellite whose position it has not computed.
const std::array<geotrace::Satellite, 8> kSky = {{
    {geotrace::Constellation::Gps, 3, true, 72.0F, 145.0F, 44.0F},
    {geotrace::Constellation::Gps, 8, true, 58.0F, 230.0F, 41.0F},
    {geotrace::Constellation::Gps, 14, true, 41.0F, 60.0F, 37.0F},
    {geotrace::Constellation::Gps, 22, false, 18.0F, 310.0F, 28.0F},
    {geotrace::Constellation::Galileo, 7, true, 65.0F, 195.0F, 42.0F},
    {geotrace::Constellation::Galileo, 12, true, 33.0F, 90.0F, 35.0F},
    {geotrace::Constellation::Galileo, 19, false, 12.0F, 15.0F, 22.0F},
    {geotrace::Constellation::Glonass, 5, false, std::nullopt, std::nullopt, 31.0F},
}};

} // namespace

int main() {
    auto timestamp_at = [](std::int64_t secs) {
        return geotrace::Timestamp::from_seconds(kBase + secs);
    };

    try {
        geotrace::FileBuilder builder{};
        builder.title("Satellite quality tour").device("Example GNSS v1.0");

        std::size_t index = 0;
        for (const auto &point : kTrack) {
            const geotrace::Timestamp time = timestamp_at(point.offset_s);

            geotrace::NavFix fix{geotrace::FixTime::receiver(time),
                                 geotrace::Angle::degrees(point.lat),
                                 geotrace::Angle::degrees(point.lon)};
            fix.heading = geotrace::Angle::degrees(point.heading_deg);
            fix.speed = geotrace::Velocity::mps(point.speed_mps);
            fix.eph_m = point.eph_m;
            builder.add(fix);

            // SNR climbs slightly along the track as the receiver settles.
            const float snr_gain = 0.5F * static_cast<float>(index);
            std::vector<geotrace::Satellite> tracked(kSky.begin(), kSky.end());
            for (auto &satellite : tracked) {
                if (satellite.snr_dbhz) {
                    satellite.snr_dbhz = *satellite.snr_dbhz + snr_gain;
                }
            }

            builder.add(geotrace::SatelliteReport{geotrace::FixTime::receiver(time), tracked});
            ++index;
        }

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_with_satellites.gtd";
        file.write_to_file(out);

        const geotrace::NavFile loaded = geotrace::NavFile::open(out);
        std::cout << "Nav points: " << loaded.nav_point_count() << "\n";
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
