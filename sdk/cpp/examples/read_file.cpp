#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <iostream>

int main(int argc, char **argv) {
    if (argc < 2) {
        std::cerr << "usage: " << argv[0] << " <file.gtd>\n";
        return 1;
    }

    try {
        auto file = geotrace::NavFile::open(argv[1]);

        if (!file.device().empty())
            std::cout << "Device: " << file.device() << "\n";
        if (!file.title().empty())
            std::cout << "Title: " << file.title() << "\n";

        std::cout << file.nav_point_count() << " nav fix(es)\n";

        for (std::size_t i = 0; i < file.nav_point_count(); ++i) {
            auto p = file.nav_point(i);
            std::cout << "  [" << i << "] " << p.lat.as_degrees() << ", " << p.lon.as_degrees();
            if (p.speed) {
                std::cout << "  speed=" << p.speed->as_kmh() << " km/h";
            }
            if (p.satellite_count > 0) {
                std::cout << "  sats=" << p.satellite_count;
                for (std::size_t j = 0; j < p.satellite_count; ++j) {
                    auto s = file.satellite(i, j);
                    if (s.in_fix)
                        std::cout << " (prn=" << s.prn << " in_fix)";
                }
            }
            std::cout << "\n";
        }

        if (file.event_marker_count() > 0) {
            std::cout << file.event_marker_count() << " event marker(s)\n";
            for (std::size_t i = 0; i < file.event_marker_count(); ++i) {
                auto m = file.event_marker(i);
                std::cout << "  [" << i << "] " << m.variant_path;
                if (!m.annotation.empty())
                    std::cout << " - " << m.annotation;
                std::cout << "\n";
            }
        }
    } catch (const geotrace::Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
