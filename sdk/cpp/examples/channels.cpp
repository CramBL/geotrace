/**
 * Write a .gtd file with ad-hoc sensor channels, then read them back.
 *
 * A channel is a named time series sampled at its own rate, correlated with the
 * nav track by timestamp.  It can be scalar (an inclinometer angle) or a vector
 * whose components share one sample clock (an accelerometer's x/y/z axes).
 */

#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <iostream>

int main() {
    const auto t0 = geotrace::Timestamp::from_seconds(1717228800ULL); // 2024-06-01T08:00:00Z
    const auto t1 = geotrace::Timestamp::from_seconds(1717228801ULL);

    try {
        geotrace::FileBuilder builder{};
        builder.title("Channel tour");

        geotrace::NavFix fix{};
        fix.gps_time = t0;
        fix.lat = geotrace::Angle::degrees(51.5074);
        fix.lon = geotrace::Angle::degrees(-0.1278);
        builder.add(fix);

        geotrace::Channel incline{};
        incline.name = "incline";
        incline.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Deg);
        incline.description = "boom inclinometer";
        incline.times = {t0, t1};
        incline.values = {1.5, 2.0};
        builder.add(incline);

        geotrace::Channel accel{};
        accel.name = "accel";
        accel.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Mg);
        accel.components = {"x", "y", "z"};
        accel.times = {t0, t1};
        accel.values = {100.0, 200.0, 980.0, -100.0, 300.0, 1020.0};
        builder.add(accel);

        geotrace::Channel quality{};
        quality.name = "quality";
        quality.unit = geotrace::ChannelUnit::custom("vendor score");
        quality.times = {t0, t1};
        quality.values = {80.0, 81.0};
        builder.add(quality);

        auto file = builder.finish();

        std::cout << file.channel_count() << " channels:\n";
        for (std::size_t i = 0; i < file.channel_count(); ++i) {
            const auto ch = file.channel(i);
            std::cout << "  " << ch.name << ' ' << ch.times.size() << " samples";
            if (ch.unit) {
                std::cout << " [" << ch.unit->label() << ']';
            }
            if (ch.is_vector()) {
                std::cout << " components:";
                for (const auto &c : ch.components) {
                    std::cout << ' ' << c;
                }
            }
            std::cout << '\n';
        }
    } catch (const geotrace::Error &e) {
        std::cerr << "error: " << e.what() << '\n';
        return 1;
    }
    return 0;
}
