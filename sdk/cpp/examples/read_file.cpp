#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <exception>
#include <iostream>
#include <iterator>
#include <string>
#include <vector>

int main(int argc, char **argv) {
    try {
        const std::vector<std::string> args(argv, std::next(argv, argc));
        if (args.size() < 2) {
            std::cerr << "usage: read_file <file.gtd>\n";
            return 1;
        }

        auto file = geotrace::NavFile::open(args.at(1));

        if (!file.device().empty()) {
            std::cout << "Device: " << file.device() << "\n";
        }
        if (!file.title().empty()) {
            std::cout << "Title: " << file.title() << "\n";
        }

        std::cout << file.nav_point_count() << " nav fix(es)\n";

        for (std::size_t i = 0; i < file.nav_point_count(); ++i) {
            auto point = file.nav_point(i);
            std::cout << "  [" << i << "] " << point.lat.as_degrees() << ", "
                      << point.lon.as_degrees();
            if (point.speed) {
                std::cout << "  speed=" << point.speed->as_kmh() << " km/h";
            }
            if (point.satellite_count > 0) {
                std::cout << "  sats=" << point.satellite_count;
                for (std::size_t j = 0; j < point.satellite_count; ++j) {
                    auto satellite = file.satellite(i, j);
                    if (satellite.in_fix) {
                        std::cout << " (prn=" << satellite.prn << " in_fix)";
                    }
                }
            }
            std::cout << "\n";
        }

        if (file.event_marker_count() > 0) {
            std::cout << file.event_marker_count() << " event marker(s)\n";
            for (std::size_t i = 0; i < file.event_marker_count(); ++i) {
                auto marker = file.event_marker(i);
                std::cout << "  [" << i << "] " << marker.variant_path;
                if (!marker.annotation.empty()) {
                    std::cout << " - " << marker.annotation;
                }
                std::cout << "\n";
            }
        }
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
