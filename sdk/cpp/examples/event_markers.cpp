/**
 * Write and read back a .gtd file containing event markers (raw paths).
 *
 * Event markers are timed, hierarchical events anchored to the GPS track.  Each
 * carries a slash-separated variant path (e.g. "power/boot" or
 * "connectivity/agps/request") that GeoTrace uses to group and filter events.
 * Per-variant styles set an icon and color. Unlisted variants get a
 * deterministic fallback color derived from their path.
 *
 * For a compile-checked taxonomy, see typed_events.cpp.
 */

#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <iostream>
#include <string>
#include <string_view>

namespace {
// 2024-06-01T08:00:00Z keeps the output deterministic. File-scope so the `at`
// lambda can read it without a capture (MSVC C++17 requires capturing a local).
constexpr std::uint64_t kBase = 1717228800;
} // namespace

int main() {
    auto at = [](std::uint64_t secs) { return geotrace::Timestamp::from_seconds(kBase + secs); };

    try {
        geotrace::FileBuilder builder{};
        builder.title("Event marker tour").device("Example GPS v1.0");

        struct TrackPoint {
            double lat;
            double lon;
        };
        const TrackPoint track[] = {
            {51.5074, -0.1278}, {51.5080, -0.1265}, {51.5088, -0.1248},
            {51.5095, -0.1233}, {51.5103, -0.1217}, {51.5110, -0.1200},
        };
        std::size_t idx = 0;
        for (const auto &point : track) {
            geotrace::NavFix fix{};
            fix.gps_time = at(idx * 30);
            fix.lat = geotrace::Angle::degrees(point.lat);
            fix.lon = geotrace::Angle::degrees(point.lon);
            fix.heading = geotrace::Angle::degrees(90.0);
            builder.add(fix);
            ++idx;
        }

        struct Event {
            std::string_view path;
            std::uint64_t offset;
            std::string_view note; // empty = none
        };
        const Event events[] = {
            {"power/boot", 2, "cold start"},
            {"connectivity/agps/request", 5, "EPO fetch started"},
            {"connectivity/agps/success", 18, "EPO applied, TTFF reduced"},
            {"sensor/gps/lock_acquired", 20, ""},
            {"power/sleep", 145, ""},
        };
        for (const auto &e : events)
            builder.add(
                geotrace::EventMarker{std::string(e.path), at(e.offset), std::string(e.note)});

        builder.add_event_marker_style(
            geotrace::EventMarkerStyle{"power/boot", geotrace::MarkerIcon::Lightning, "#44BB44"});
        builder.add_event_marker_style(
            geotrace::EventMarkerStyle{"power/sleep", geotrace::MarkerIcon::Pin, "#4488FF"});

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_event_markers.gtd";
        file.write_to_file(out);

        const geotrace::NavFile loaded = geotrace::NavFile::open(out);
        std::cout << loaded.event_marker_count() << " event marker(s)\n";
        for (std::size_t i = 0; i < loaded.event_marker_count(); ++i) {
            const auto m = loaded.event_marker(i);
            std::cout << "  " << m.variant_path;
            if (!m.annotation.empty())
                std::cout << " - " << m.annotation;
            std::cout << "\n";
        }

        std::filesystem::remove(out);
    } catch (const geotrace::Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
