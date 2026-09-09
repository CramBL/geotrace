#include <geotrace/geotrace.hpp>

#include <cmath>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <iostream>

namespace {

// 2024-06-01T08:00:00Z keeps the written file deterministic.
constexpr std::int64_t kFixSeconds = 1717228800;
constexpr double kFixLatDeg = 51.5074;
constexpr double kFixLonDeg = -0.1278;

// The tolerance is this tight because the format stores a coordinate as a
// double: a lossless round trip reproduces the written latitude exactly.
constexpr double kLatToleranceDeg = 1e-9;

} // namespace

int main() {
    try {
        const std::filesystem::path path{GEOTRACE_SMOKE_PATH};

        geotrace::FileBuilder builder{};
        builder.add(geotrace::NavFix{
            geotrace::FixTime::receiver(geotrace::Timestamp::from_seconds(kFixSeconds)),
            geotrace::Angle::degrees(kFixLatDeg), geotrace::Angle::degrees(kFixLonDeg)});
        builder.finish().write_to_file(path);

        const geotrace::NavFile read_back = geotrace::NavFile::open(path);
        if (read_back.nav_point_count() != 1) {
            std::cerr << "expected 1 nav point, got " << read_back.nav_point_count() << "\n";
            return 1;
        }

        const double lat_deg = read_back.nav_point(0).lat.as_degrees();
        if (std::abs(lat_deg - kFixLatDeg) > kLatToleranceDeg) {
            std::cerr << "expected latitude " << kFixLatDeg << ", got " << lat_deg << "\n";
            return 1;
        }
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }

    std::cout << "smoke_cpp OK, geotrace-cpp " << GEOTRACE_CPP_VERSION << "\n";
    return 0;
}
