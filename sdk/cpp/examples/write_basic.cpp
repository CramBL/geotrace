#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <ctime>
#include <iostream>

int main() {
    using namespace geotrace;

    auto now = Timestamp::from_seconds(static_cast<std::uint64_t>(std::time(nullptr)));
    auto t1 = Timestamp::from_seconds(static_cast<std::uint64_t>(std::time(nullptr)) + 10);

    try {
        FileBuilder builder;
        builder.title("Quick tour").device("Example GPS v1.0");

        NavFix f0{};
        f0.gps_time = now;
        f0.lat = Angle::degrees(51.5074);
        f0.lon = Angle::degrees(-0.1278);
        f0.heading = Angle::degrees(90.0);
        f0.speed = Velocity::mps(5.5);
        f0.eph_m = 3.2;
        builder.add_nav_fix(f0);

        SatelliteReport report{};
        report.gps_time = now;

        Satellite s1{};
        s1.constellation = Constellation::Gps;
        s1.prn = 1;
        s1.in_fix = true;
        s1.elevation_deg = 45.0;
        s1.azimuth_deg = 90.0;
        s1.snr_dbhz = 38.0;
        report.tracked.push_back(s1);

        Satellite s2{};
        s2.constellation = Constellation::Galileo;
        s2.prn = 3;
        s2.in_fix = false;
        s2.snr_dbhz = 22.0;
        report.tracked.push_back(s2);

        builder.add_satellite_report(report);

        NavFix f1{};
        f1.gps_time = t1;
        f1.lat = Angle::degrees(51.5080);
        f1.lon = Angle::degrees(-0.1265);
        f1.heading = Angle::degrees(85.0);
        f1.speed = Velocity::mps(5.8);
        f1.eph_m = 2.9;
        builder.add_nav_fix(f1);

        Annotation ann{};
        ann.time = now;
        ann.label = "Start point";
        ann.icon = MarkerIcon::Pin;
        builder.add_annotation(ann);

        const NavFile file = builder.finish();

        file.write_to_file("output.gtd");
        std::cout << "wrote output.gtd\n";
    } catch (const Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
