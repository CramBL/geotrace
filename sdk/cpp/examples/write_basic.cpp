#include <geotrace/geotrace.hpp>

#include <ctime>
#include <iostream>

int main() {
    using namespace geotrace;

    auto now = Timestamp::from_seconds(static_cast<std::uint64_t>(std::time(nullptr)));
    auto t1 = Timestamp::from_seconds(static_cast<std::uint64_t>(std::time(nullptr)) + 10);

    try {
        NavFile file = FileBuilder{}
                           .title("Quick tour")
                           .device("Example GPS v1.0")
                           .add_nav_fix(NavFix{
                               .gps_time = now,
                               .lat = Angle::degrees(51.5074),
                               .lon = Angle::degrees(-0.1278),
                               .heading = Angle::degrees(90.0),
                               .speed = Velocity::mps(5.5),
                               .eph_m = 3.2,
                           })
                           .add_satellite_report(SatelliteReport{
                               .gps_time = now,
                               .tracked =
                                   {
                                       Satellite{
                                           .constellation = Constellation::Gps,
                                           .prn = 1,
                                           .in_fix = true,
                                           .elevation_deg = 45.0,
                                           .azimuth_deg = 90.0,
                                           .snr_dbhz = 38.0,
                                       },
                                       Satellite{
                                           .constellation = Constellation::Galileo,
                                           .prn = 3,
                                           .in_fix = false,
                                           .elevation_deg = std::nullopt,
                                           .azimuth_deg = std::nullopt,
                                           .snr_dbhz = 22.0,
                                       },
                                   },
                           })
                           .add_nav_fix(NavFix{
                               .gps_time = t1,
                               .lat = Angle::degrees(51.5080),
                               .lon = Angle::degrees(-0.1265),
                               .heading = Angle::degrees(85.0),
                               .speed = Velocity::mps(5.8),
                               .eph_m = 2.9,
                           })
                           .add_annotation(Annotation{
                               .time = now,
                               .label = "Start point",
                               .icon = MarkerIcon::Pin,
                           })
                           .finish();

        file.write_to_file("output.gtd");
        std::cout << "wrote output.gtd\n";
    } catch (const Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
