/**
 * Write a .gtd file with ad-hoc sensor channels, then read them back.
 *
 * A channel is a named time series sampled at its own rate, correlated with the
 * nav track by timestamp.  It can be scalar (an inclinometer angle) or a vector
 * whose components share one sample clock (an accelerometer's x/y/z axes).
 */

#include <geotrace/geotrace.hpp>
#include <geotrace/unit_catalog.hpp>

#include <cstddef>
#include <exception>
#include <iostream>

int main() {
    const auto first_sample_time =
        geotrace::Timestamp::from_seconds(1717228800ULL); // 2024-06-01T08:00:00Z
    const auto second_sample_time = geotrace::Timestamp::from_seconds(1717228801ULL);

    try {
        geotrace::FileBuilder builder{};
        builder.title("Channel tour");

        builder.add(geotrace::NavFix{geotrace::FixTime::receiver(first_sample_time),
                                     geotrace::Angle::degrees(51.5074),
                                     geotrace::Angle::degrees(-0.1278)});

        geotrace::Channel incline{};
        incline.name = "incline";
        incline.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Deg);
        incline.description = "boom inclinometer";
        incline.times = {first_sample_time, second_sample_time};
        incline.values = {1.5, 2.0};
        builder.add(incline);

        geotrace::Channel accel{};
        accel.name = "accel";
        accel.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Mg);
        accel.components = {"x", "y", "z"};
        accel.times = {first_sample_time, second_sample_time};
        accel.values = {100.0, 200.0, 980.0, -100.0, 300.0, 1020.0};
        builder.add(accel);

        geotrace::Channel quality{};
        quality.name = "quality";
        const auto quality_unit = geotrace::ChannelUnit::try_custom("vendor score");
        if (quality_unit.is_err()) {
            std::cerr << quality_unit.error().description << '\n';
            return 1;
        }
        quality.unit = quality_unit.value();
        quality.times = {first_sample_time, second_sample_time};
        quality.values = {80.0, 81.0};
        builder.add(quality);

        auto file = builder.finish();

        std::cout << file.channel_count() << " channels:\n";
        for (std::size_t i = 0; i < file.channel_count(); ++i) {
            const auto channel = file.channel(i);
            std::cout << "  " << channel.name << ' ' << channel.times.size() << " samples";
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
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << '\n';
        return 1;
    }
    return 0;
}
