/**
 * Aggregate data from multiple sources into a single .gtd GeoTrace data file.
 *
 * Scenario: your GPS unit logs fixes to one source, and a separate system (a
 * test harness, an annotation tool, a sensor log) records named events with
 * their own timestamps.  Both are added independently to the builder. finish()
 * sorts everything by time and interpolates each annotation's map position from
 * the two surrounding GPS fixes.
 */

#include <geotrace/geotrace.hpp>

#include <array>
#include <cstddef>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <iostream>
#include <string>
#include <string_view>

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
        builder.title("Merged GPS + annotations").device("Aggregator v1.0");

        // Source 1: GPS track (lat, lon, heading), one fix every 10 s.
        struct TrackPoint {
            double lat;
            double lon;
            double heading;
        };
        const std::array<TrackPoint, 6> gps = {{
            {51.5074, -0.1278, 90.0},
            {51.5075, -0.1276, 91.0},
            {51.5076, -0.1274, 89.5},
            {51.5077, -0.1272, 88.0},
            {51.5078, -0.1270, 90.0},
            {51.5079, -0.1268, 90.5},
        }};
        std::int64_t idx = 0;
        for (const auto &point : gps) {
            geotrace::NavFix fix{geotrace::FixTime::receiver(timestamp_at(idx * 10)),
                                 geotrace::Angle::degrees(point.lat),
                                 geotrace::Angle::degrees(point.lon)};
            fix.heading = geotrace::Angle::degrees(point.heading);
            builder.add(fix);
            ++idx;
        }

        // Source 2: annotations from a separate log.  Their map positions are
        // not supplied - finish() interpolates them from the GPS fixes by time.
        struct Marker {
            std::int64_t offset;
            std::string_view label;
            geotrace::MarkerIcon icon;
        };
        const std::array<Marker, 3> markers = {{
            {5, "Pothole", geotrace::MarkerIcon::Warning},
            {15, "Speed camera", geotrace::MarkerIcon::Circle},
            {25, "Junction", geotrace::MarkerIcon::Pin},
        }};
        std::size_t marker_count = 0;
        for (const auto &marker : markers) {
            builder.add(geotrace::Annotation{timestamp_at(marker.offset), std::string(marker.label),
                                             marker.icon});
            ++marker_count;
        }

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_from_multiple_sources.gtd";
        file.write_to_file(out);

        std::cout << "Merged " << file.nav_point_count() << " GPS fixes + " << marker_count
                  << " annotations -> " << out.string() << "\n";
        std::cout << "Annotations were interpolated onto the track by timestamp.\n";

        std::filesystem::remove(out);
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
