#include <geotrace/geotrace.hpp>

#include <cstdint>
#include <ctime>
#include <exception>
#include <iostream>

int main() {
    try {
        const auto now =
            geotrace::Timestamp::from_seconds(static_cast<std::int64_t>(std::time(nullptr)));
        const auto ten_seconds_later =
            geotrace::Timestamp::from_seconds(static_cast<std::int64_t>(std::time(nullptr)) + 10);

        geotrace::FileBuilder builder{};
        builder.title("Quick tour").device("Example GPS v1.0");

        geotrace::NavFix first_fix{geotrace::FixTime::receiver(now),
                                   geotrace::Angle::degrees(51.5074),
                                   geotrace::Angle::degrees(-0.1278)};
        first_fix.heading = geotrace::Angle::degrees(90.0);
        first_fix.speed = geotrace::Velocity::mps(5.5);
        first_fix.eph_m = 3.2;
        builder.add(first_fix);

        geotrace::Satellite gps_prn1{};
        gps_prn1.constellation = geotrace::Constellation::Gps;
        gps_prn1.prn = 1;
        gps_prn1.in_fix = true;
        gps_prn1.elevation_deg = 45.0F;
        gps_prn1.azimuth_deg = 90.0F;
        gps_prn1.snr_dbhz = 38.0F;

        geotrace::Satellite galileo_prn3{};
        galileo_prn3.constellation = geotrace::Constellation::Galileo;
        galileo_prn3.prn = 3;
        galileo_prn3.in_fix = false;
        galileo_prn3.snr_dbhz = 22.0F;

        builder.add(
            geotrace::SatelliteReport{geotrace::FixTime::receiver(now), {gps_prn1, galileo_prn3}});

        geotrace::NavFix second_fix{geotrace::FixTime::receiver(ten_seconds_later),
                                    geotrace::Angle::degrees(51.5080),
                                    geotrace::Angle::degrees(-0.1265)};
        second_fix.heading = geotrace::Angle::degrees(85.0);
        second_fix.speed = geotrace::Velocity::mps(5.8);
        second_fix.eph_m = 2.9;
        builder.add(second_fix);

        builder.add(geotrace::Annotation{now, "Start point", geotrace::MarkerIcon::Pin});

        const geotrace::NavFile file = builder.finish();

        file.write_to_file("output.gtd");
        std::cout << "wrote output.gtd\n";
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
