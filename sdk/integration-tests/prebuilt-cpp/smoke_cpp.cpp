// Minimal consumer of the prebuilt C++ SDK: build a one-fix file in memory and
// print the SDK version. Mirrors what a downstream project does after
// find_package(GeoTraceCpp).
#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <iostream>

int main() {
    using namespace geotrace;

    try {
        FileBuilder{} builder;
        builder.title("smoke");

        NavFix fix;
        fix.gps_time = Timestamp::from_seconds(1700000000U);
        fix.lat = Angle::degrees(51.5074);
        fix.lon = Angle::degrees(-0.1278);
        builder.add_nav_fix(fix);

        const NavFile file = builder.finish();
        file.write_to_file("smoke_cpp_out.gtd");
    } catch (const Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }

    std::cout << "smoke OK, geotrace-cpp " << GEOTRACE_CPP_VERSION << "\n";
    return 0;
}
