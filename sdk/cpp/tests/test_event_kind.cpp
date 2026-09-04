#include <doctest/doctest.h>
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <string>
#include <string_view>

using geotrace::Angle;
using geotrace::event_path;
using geotrace::FileBuilder;
using geotrace::NavFile;
using geotrace::NavFix;
using geotrace::Timestamp;

namespace {

enum class Power : std::uint8_t { Boot, Sleep, BatteryLow };
enum class Connectivity : std::uint8_t { Agps };
enum class Agps : std::uint8_t { Request, Success };

} // namespace

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
        }
        return "";
    }
};

// The SFINAE guard must reject types without an EventEnum<> specialisation.
static_assert(!geotrace::detail::is_event_enum<int>::value);
static_assert(geotrace::detail::is_event_enum<Power>::value);

TEST_CASE("event_path composes base/seg and suppresses nested bases") {
    struct Case {
        std::string actual;
        std::string_view expected;
    };

    const Case cases[] = {
        // A single level emits that enum's own base, then its leaf segment.
        {event_path(Power::Boot).str(), "power/boot"},
        {event_path(Power::BatteryLow).str(), "power/battery_low"},
        {event_path(Agps::Request).str(), "agps/request"},
        // Nested composition emits the outer base once, then each inner `seg()` -
        // the inner enum's own base (`"agps"`) is suppressed, never repeated.
        {event_path(Connectivity::Agps, Agps::Request).str(), "connectivity/agps/request"},
        {event_path(Connectivity::Agps, Agps::Success).str(), "connectivity/agps/success"},
    };

    for (const auto &c : cases) {
        CHECK(c.actual == std::string(c.expected));
    }
}

TEST_CASE("add_event: typed values round-trip as event markers") {
    NavFix f0;
    f0.gps_time = Timestamp::from_seconds(1700000000ULL);
    f0.lat = Angle::degrees(51.5074);
    f0.lon = Angle::degrees(-0.1278);

    NavFix f1;
    f1.gps_time = Timestamp::from_seconds(1700000030ULL);
    f1.lat = Angle::degrees(51.5080);
    f1.lon = Angle::degrees(-0.1265);

    FileBuilder builder;
    builder.add_nav_fix(f0).add_nav_fix(f1);
    builder.add_event(Power::Boot, Timestamp::from_seconds(1700000005ULL), "cold start");
    builder.add_event(event_path(Connectivity::Agps, Agps::Request),
                      Timestamp::from_seconds(1700000010ULL));

    const NavFile file = builder.finish();

    REQUIRE(file.event_marker_count() == 2);
    CHECK(file.event_marker(0).variant_path == "power/boot");
    CHECK(file.event_marker(0).annotation == "cold start");
    CHECK(file.event_marker(1).variant_path == "connectivity/agps/request");
    CHECK(file.event_marker(1).annotation.empty());
}
