/**
 * Type-safe event markers with the EventEnum<> facility.
 *
 * Model the event taxonomy as `enum class` levels and specialise
 * `geotrace::EventEnum<>` to give each level a path segment.  The compiler then
 * rejects any event that is not a known variant, and `event_path()` composes
 * nested paths like "connectivity/agps/request" at the call site - no raw
 * strings, no typos.
 *
 * This mirrors the Rust SDK's `#[derive(EventKind)]` example with the idiom a
 * C++ developer reaches for: enums plus a small trait.
 */

#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <iostream>
#include <string_view>

namespace {

// 2024-06-01T08:00:00Z. File-scope so the `at` lambda can read it without a
// capture (MSVC C++17 requires capturing a local).
constexpr std::uint64_t kBase = 1717228800;

// The event taxonomy: one `enum class` per level of the hierarchy.
enum class Power : std::uint8_t { Boot, Sleep, BatteryLow };
enum class Connectivity : std::uint8_t { Agps };
enum class Agps : std::uint8_t { Request, Success, Timeout };
enum class Sensor : std::uint8_t { Gps };
enum class Gps : std::uint8_t { LockAcquired, LockLost };

} // namespace

// Each level names itself (`base`) and maps its values to leaf segments.
template <> struct geotrace::EventEnum<Power> {
    static constexpr std::string_view base = "power";
    static constexpr std::string_view seg(Power p) {
        switch (p) {
        case Power::Boot:
            return "boot";
        case Power::Sleep:
            return "sleep";
        case Power::BatteryLow:
            return "battery_low";
        }
        return "";
    }
};

template <> struct geotrace::EventEnum<Connectivity> {
    static constexpr std::string_view base = "connectivity";
    static constexpr std::string_view seg(Connectivity c) {
        switch (c) {
        case Connectivity::Agps:
            return "agps";
        }
        return "";
    }
};

template <> struct geotrace::EventEnum<Agps> {
    static constexpr std::string_view base = "agps";
    static constexpr std::string_view seg(Agps a) {
        switch (a) {
        case Agps::Request:
            return "request";
        case Agps::Success:
            return "success";
        case Agps::Timeout:
            return "timeout";
        }
        return "";
    }
};

template <> struct geotrace::EventEnum<Sensor> {
    static constexpr std::string_view base = "sensor";
    static constexpr std::string_view seg(Sensor s) {
        switch (s) {
        case Sensor::Gps:
            return "gps";
        }
        return "";
    }
};

template <> struct geotrace::EventEnum<Gps> {
    static constexpr std::string_view base = "gps";
    static constexpr std::string_view seg(Gps g) {
        switch (g) {
        case Gps::LockAcquired:
            return "lock_acquired";
        case Gps::LockLost:
            return "lock_lost";
        }
        return "";
    }
};

int main() {
    // A lambda over the file-scope kBase keeps the event timeline readable.
    auto at = [](std::uint64_t secs) { return geotrace::Timestamp::from_seconds(kBase + secs); };

    try {
        geotrace::FileBuilder builder{};
        builder.title("Typed event tour").device("Example GPS v1.0");

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

        // Single-level events take the enum value directly.
        builder.add_event(Power::Boot, at(2), "cold start");
        // Nested events compose with event_path(). The compiler checks every level.
        builder.add_event(geotrace::event_path(Connectivity::Agps, Agps::Request), at(5),
                          "EPO fetch started");
        builder.add_event(geotrace::event_path(Connectivity::Agps, Agps::Success), at(18),
                          "EPO applied");
        builder.add_event(geotrace::event_path(Sensor::Gps, Gps::LockAcquired), at(20));
        builder.add_event(Power::BatteryLow, at(130), "14%");
        builder.add_event(Power::Sleep, at(145));

        // Styles are per-variant. event_path() feeds them the same typed path.
        builder.add_event_marker_style(geotrace::EventMarkerStyle{
            geotrace::event_path(Power::Boot).str(), geotrace::MarkerIcon::Lightning, "#44BB44"});
        builder.add_event_marker_style(geotrace::EventMarkerStyle{
            geotrace::event_path(Power::Sleep).str(), geotrace::MarkerIcon::Pin, "#4488FF"});

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_typed_events.gtd";
        file.write_to_file(out);

        const geotrace::NavFile loaded = geotrace::NavFile::open(out);
        std::cout << loaded.nav_point_count() << " fixes, " << loaded.event_marker_count()
                  << " event markers\n";
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
