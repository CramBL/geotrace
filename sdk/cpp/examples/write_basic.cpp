#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <ctime>
#include <iostream>

int main() {
    auto now = geotrace::Timestamp::from_seconds(static_cast<std::uint64_t>(std::time(nullptr)));
    auto t1 =
        geotrace::Timestamp::from_seconds(static_cast<std::uint64_t>(std::time(nullptr)) + 10);

    try {
        geotrace::FileBuilder builder{};
        builder.title("Quick tour").device("Example GPS v1.0");

        geotrace::NavFix f0{};
        f0.gps_time = now;
        f0.lat = geotrace::Angle::degrees(51.5074);
        f0.lon = geotrace::Angle::degrees(-0.1278);
        f0.heading = geotrace::Angle::degrees(90.0);
        f0.speed = geotrace::Velocity::mps(5.5);
        f0.eph_m = 3.2;
        builder.add(f0);

        geotrace::SatelliteReport report{};
        report.gps_time = now;

        geotrace::Satellite s1{};
        s1.constellation = geotrace::Constellation::Gps;
        s1.prn = 1;
        s1.in_fix = true;
        s1.elevation_deg = 45.0;
        s1.azimuth_deg = 90.0;
        s1.snr_dbhz = 38.0;
        report.tracked.push_back(s1);

        geotrace::Satellite s2{};
        s2.constellation = geotrace::Constellation::Galileo;
        s2.prn = 3;
        s2.in_fix = false;
        s2.snr_dbhz = 22.0;
        report.tracked.push_back(s2);

        builder.add(report);

        geotrace::NavFix f1{};
        f1.gps_time = t1;
        f1.lat = geotrace::Angle::degrees(51.5080);
        f1.lon = geotrace::Angle::degrees(-0.1265);
        f1.heading = geotrace::Angle::degrees(85.0);
        f1.speed = geotrace::Velocity::mps(5.8);
        f1.eph_m = 2.9;
        builder.add(f1);

        geotrace::Annotation ann{};
        ann.time = now;
        ann.label = "Start point";
        ann.icon = geotrace::MarkerIcon::Pin;
        builder.add(ann);

        const geotrace::NavFile file = builder.finish();

        file.write_to_file("output.gtd");
        std::cout << "wrote output.gtd\n";
    } catch (const geotrace::Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
