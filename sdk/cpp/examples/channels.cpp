/**
 * Write a .gtd file with ad-hoc sensor channels, then read them back.
 *
 * A channel is a named time series sampled at its own rate, correlated with the
 * nav track by timestamp.  It can be scalar (an inclinometer angle) or a vector
 * whose components share one sample clock (an accelerometer's x/y/z axes).  This
 * example also shows recognized milli-g values and a custom display-only unit.
 */

#include <geotrace/geotrace.hpp>
#include <geotrace/unit_catalog.hpp>

#include <cstddef>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <vector>

namespace {
// 2024-06-01T08:00:00Z keeps the output deterministic.
constexpr std::int64_t kBase = 1717228800;
} // namespace

int main() {
    try {
        // Three samples, one second apart. A real recorder would sample faster
        // than the fixes. The channel keeps its own clock either way.
        const std::vector<geotrace::Timestamp> times = {
            geotrace::Timestamp::from_seconds(kBase),
            geotrace::Timestamp::from_seconds(kBase + 1),
            geotrace::Timestamp::from_seconds(kBase + 2),
        };

        geotrace::FileBuilder builder{};
        builder.title("Channel tour");

        builder.add(geotrace::NavFix{geotrace::FixTime::receiver(times.front()),
                                     geotrace::Angle::degrees(51.5074),
                                     geotrace::Angle::degrees(-0.1278)});

        // A scalar channel: one value per timestamp.
        geotrace::Channel incline{};
        incline.name = "incline";
        incline.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Deg);
        incline.description = "boom inclinometer";
        incline.times = times;
        incline.values = {1.0, 1.5, 2.0};
        builder.add(incline);

        // A vector channel: `values` is row-major, one row of x/y/z per
        // timestamp. Declaring the unit as `mg` lets GeoTrace's query language
        // compare it against acceleration literals.
        geotrace::Channel accel{};
        accel.name = "accel";
        accel.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Mg);
        accel.description = "IMU acceleration";
        accel.components = {"x", "y", "z"};
        accel.times = times;
        accel.values = {
            0.0,   200.0, 980.0, //
            100.0, 200.0, 980.0, //
            200.0, 200.0, 980.0,
        };
        builder.add(accel);

        // A custom unit is displayed verbatim and stays dimensionless in queries.
        geotrace::Channel quality{};
        quality.name = "quality";
        const auto quality_unit = geotrace::ChannelUnit::try_custom("vendor score");
        if (quality_unit.is_err()) {
            std::cerr << quality_unit.error().description << '\n';
            return 1;
        }
        quality.unit = quality_unit.value();
        quality.times = times;
        quality.values = {80.0, 81.0, 82.0};
        builder.add(quality);

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_channels.gtd";
        file.write_to_file(out);

        const geotrace::NavFile loaded = geotrace::NavFile::open(out);
        std::cout << loaded.channel_count() << " channels:\n";
        for (std::size_t i = 0; i < loaded.channel_count(); ++i) {
            const auto channel = loaded.channel(i);
            std::cout << "  " << std::left << std::setw(10) << channel.name << ' '
                      << channel.times.size() << " samples";
            if (channel.unit) {
                std::cout << " [" << channel.unit->label() << ']';
            }
            if (channel.is_vector()) {
                std::cout << " components:";
                for (const auto &component : channel.components) {
                    std::cout << ' ' << component;
                }
            }
            std::cout << '\n';
        }

        std::filesystem::remove(out);
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << '\n';
        return 1;
    }
    return 0;
}
