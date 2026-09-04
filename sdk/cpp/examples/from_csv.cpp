/**
 * Convert GPS data from CSV text into a .gtd GeoTrace data file.
 *
 * Scenario: your GPS logger exports fixes as CSV rows.  Parse each row, feed the
 * fields to the builder, then finish() to produce a validated file ready for
 * GeoTrace to open.  In a real workflow you would read the CSV from a file.
 *
 * Timestamps here are whole Unix epoch seconds to keep the parser tiny.
 */

#include <geotrace/geotrace.hpp>

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <iostream>
#include <sstream>
#include <string>
#include <string_view>

namespace {

constexpr std::string_view kCsvData = "timestamp_s,lat,lon,heading_deg,speed_mps\n"
                                      "1705309200,51.5074,-0.1278,90.0,12.5\n"
                                      "1705309201,51.5075,-0.1276,91.0,12.6\n"
                                      "1705309202,51.5076,-0.1274,89.5,12.4\n"
                                      "1705309203,51.5077,-0.1272,88.0,12.3\n"
                                      "1705309204,51.5078,-0.1270,90.0,12.5\n"
                                      "1705309205,51.5079,-0.1268,90.5,12.6\n";

// Parse one "ts,lat,lon,heading,speed" row. Returns false on a malformed line.
bool parse_row(const std::string &line, geotrace::NavFix &fix) {
    std::istringstream ls(line);
    std::uint64_t ts = 0;
    double lat = 0.0;
    double lon = 0.0;
    double heading = 0.0;
    double speed = 0.0;
    char comma = 0;
    if (!(ls >> ts >> comma >> lat >> comma >> lon >> comma >> heading >> comma >> speed))
        return false;

    fix.gps_time = geotrace::Timestamp::from_seconds(ts);
    fix.lat = geotrace::Angle::degrees(lat);
    fix.lon = geotrace::Angle::degrees(lon);
    fix.heading = geotrace::Angle::degrees(heading);
    fix.speed = geotrace::Velocity::mps(speed);
    return true;
}

} // namespace

int main() {
    try {
        geotrace::FileBuilder builder{};
        builder.title("Imported from CSV").device("CSV importer v1.0");

        std::istringstream csv{std::string(kCsvData)};
        std::string line;
        std::getline(csv, line); // skip header

        std::size_t rows = 0;
        while (std::getline(csv, line)) {
            if (line.empty())
                continue;
            geotrace::NavFix fix{};
            if (parse_row(line, fix)) {
                builder.add(fix);
                ++rows;
            }
        }

        const geotrace::NavFile file = builder.finish();

        const std::filesystem::path out =
            std::filesystem::temp_directory_path() / "geotrace_from_csv.gtd";
        file.write_to_file(out);

        std::cout << "Parsed " << rows << " CSV rows into " << file.nav_point_count()
                  << " nav points -> " << out.string() << "\n";

        std::filesystem::remove(out);
    } catch (const geotrace::Error &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
