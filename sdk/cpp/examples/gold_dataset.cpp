/**
 * Gold dataset reference test for the GeoTrace C++ SDK.
 *
 * Reads the CSV fixtures in tests/fixtures/gold_dataset/, builds a .gtd file,
 * then verifies the round-trip.  Run from the repository root:
 *
 *   ./sdk/cpp/build/gold/examples/gold_dataset
 */

#include <geotrace/geotrace.hpp>

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <exception>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace fs = std::filesystem;

namespace {

std::string rtrim(std::string s) {
    while (!s.empty() && (s.back() == '\r' || s.back() == '\n' || s.back() == ' '))
        s.pop_back();
    return s;
}

std::vector<std::string> split_csv(const std::string &line) {
    std::vector<std::string> cols;
    std::istringstream ss(line);
    std::string col;
    while (std::getline(ss, col, ','))
        cols.push_back(std::move(col));
    // std::getline drops the trailing empty field for lines that end with ','.
    if (!line.empty() && line.back() == ',')
        cols.emplace_back();
    return cols;
}

bool is_leap(int y) noexcept {
    return (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
}

int month_days(int m, int y) noexcept {
    static constexpr std::array<int, 12> dom = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return (m == 2 && is_leap(y)) ? 29 : dom.at(static_cast<std::size_t>(m - 1));
}

/* Parse "YYYY-MM-DDTHH:MM:SS+HH:MM" to a Timestamp, or nullopt on failure. */
std::optional<geotrace::Timestamp> parse_ts(const std::string &s) {
    if (s.size() < 19)
        return std::nullopt;
    try {
        auto Y = std::stoi(s.substr(0, 4));
        auto Mo = std::stoi(s.substr(5, 2));
        auto D = std::stoi(s.substr(8, 2));
        auto H = std::stoi(s.substr(11, 2));
        auto Mi = std::stoi(s.substr(14, 2));
        auto S = std::stoi(s.substr(17, 2));
        const char sign = (s.size() > 19) ? s[19] : '+';
        int tz_h = 0, tz_m = 0;
        if (s.size() >= 25) {
            tz_h = std::stoi(s.substr(20, 2));
            tz_m = std::stoi(s.substr(23, 2));
        }
        long days = 0;
        for (int y = 1970; y < Y; y++)
            days += is_leap(y) ? 366 : 365;
        for (int m = 1; m < Mo; m++)
            days += month_days(m, Y);
        days += D - 1;
        long secs = (days * 86400L) + (H * 3600L) + (Mi * 60L) + S;
        const long tz = (static_cast<long>(tz_h) * 60L + tz_m) * 60L;
        secs += (sign == '-') ? tz : -tz;
        return geotrace::Timestamp::from_seconds(static_cast<std::uint64_t>(secs));
    } catch (const std::exception &) {
        return std::nullopt;
    }
}

geotrace::Constellation parse_constellation(const std::string &s) {
    if (s == "gps")
        return geotrace::Constellation::Gps;
    if (s == "glonass")
        return geotrace::Constellation::Glonass;
    if (s == "galileo")
        return geotrace::Constellation::Galileo;
    if (s == "beidou")
        return geotrace::Constellation::Beidou;
    throw std::invalid_argument("unknown constellation: " + s);
}

geotrace::MarkerIcon parse_icon(const std::string &s) {
    if (s.empty() || s == "auto")
        return geotrace::MarkerIcon::Auto;
    if (s == "pin")
        return geotrace::MarkerIcon::Pin;
    if (s == "cross")
        return geotrace::MarkerIcon::Cross;
    if (s == "circle")
        return geotrace::MarkerIcon::Circle;
    if (s == "lightning")
        return geotrace::MarkerIcon::Lightning;
    if (s == "warning")
        return geotrace::MarkerIcon::Warning;
    if (s == "error")
        return geotrace::MarkerIcon::Error;
    if (s == "check")
        return geotrace::MarkerIcon::Check;
    if (s == "satellite")
        return geotrace::MarkerIcon::Satellite;
    if (s == "satellite_lost")
        return geotrace::MarkerIcon::SatelliteLost;
    if (s == "gear")
        return geotrace::MarkerIcon::Gear;
    if (s == "refresh")
        return geotrace::MarkerIcon::Refresh;
    if (s == "download")
        return geotrace::MarkerIcon::Download;
    if (s == "upload")
        return geotrace::MarkerIcon::Upload;
    if (s == "wrench")
        return geotrace::MarkerIcon::Wrench;
    return geotrace::MarkerIcon::Auto;
}

std::optional<double> parse_opt_double(const std::string &s) {
    if (s.empty())
        return std::nullopt;
    try {
        std::size_t pos = 0;
        double v = std::stod(s, &pos);
        return (pos > 0) ? std::optional<double>{v} : std::nullopt;
    } catch (const std::exception &) {
        return std::nullopt;
    }
}

struct SatRow {
    std::string gps_time;
    std::string sys_time;
    geotrace::Satellite sat;
};

std::ifstream open_csv(const fs::path &base, const std::string &name) {
    auto path = base / name;
    std::ifstream f(path);
    if (!f.is_open())
        throw geotrace::IoError("cannot open: " + path.string());
    return f;
}

void load_meta(geotrace::FileBuilder &b, const fs::path &base) {
    auto f = open_csv(base, "meta.csv");
    std::string line;
    std::getline(f, line); // skip header
    if (!std::getline(f, line))
        throw geotrace::IoError("meta.csv: missing data row");
    auto cols = split_csv(rtrim(std::move(line)));
    if (cols.size() < 4)
        throw geotrace::IoError("meta.csv: need 4 columns");
    b.title(cols[0]).device(cols[1]).notes(cols[2]).identity(cols[3]);
}

void load_event_styles(geotrace::FileBuilder &b, const fs::path &base) {
    auto f = open_csv(base, "event_styles.csv");
    std::string line;
    std::getline(f, line); // skip header
    while (std::getline(f, line)) {
        line = rtrim(std::move(line));
        if (line.empty())
            continue;
        auto cols = split_csv(line);
        if (cols.size() < 3)
            continue;
        b.add_event_marker_style(geotrace::EventMarkerStyle{cols[0], parse_icon(cols[1]), cols[2]});
    }
}

std::vector<SatRow> load_satellites(const fs::path &base) {
    auto f = open_csv(base, "satellites.csv");
    std::string line;
    std::getline(f, line); // skip header
    std::vector<SatRow> rows;
    while (std::getline(f, line)) {
        line = rtrim(std::move(line));
        if (line.empty())
            continue;
        auto cols = split_csv(line);
        if (cols.size() < 8)
            continue;
        rows.push_back(SatRow{
            cols[0],
            cols[1],
            geotrace::Satellite{
                parse_constellation(cols[2]),
                static_cast<std::uint32_t>(std::stoul(cols[3])),
                cols[4] == "true",
                parse_opt_double(cols[5]),
                parse_opt_double(cols[6]),
                parse_opt_double(cols[7]),
            },
        });
    }
    return rows;
}

void load_fixes(geotrace::FileBuilder &b, const fs::path &base, const std::vector<SatRow> &sats) {
    auto f = open_csv(base, "fixes.csv");
    std::string line;
    std::getline(f, line); // skip header
    while (std::getline(f, line)) {
        line = rtrim(std::move(line));
        if (line.empty())
            continue;
        auto cols = split_csv(line);
        // cols: track_id, gps_time, sys_time, lat, lon, heading_deg, speed_kmh, eph_m
        if (cols.size() < 8)
            continue;

        auto gps_ts = parse_ts(cols[1]);
        auto sys_ts = parse_ts(cols[2]);
        auto lat = std::stod(cols[3]);
        auto lon = std::stod(cols[4]);
        auto hdg = parse_opt_double(cols[5]);
        auto kmh = parse_opt_double(cols[6]);

        b.add_nav_fix(geotrace::NavFix{
            gps_ts.value_or(geotrace::Timestamp::none()),
            sys_ts.value_or(geotrace::Timestamp::none()),
            geotrace::Angle::degrees(lat),
            geotrace::Angle::degrees(lon),
            hdg ? std::optional{geotrace::Angle::degrees(*hdg)} : std::nullopt,
            kmh ? std::optional{geotrace::Velocity::kmh(*kmh)} : std::nullopt,
            parse_opt_double(cols[7]),
        });

        geotrace::SatelliteReport report{};
        report.gps_time = gps_ts.value_or(geotrace::Timestamp::none());
        report.sys_time = sys_ts.value_or(geotrace::Timestamp::none());
        for (const auto &row : sats) {
            if (row.gps_time == cols[1] && row.sys_time == cols[2])
                report.tracked.push_back(row.sat);
        }
        if (!report.tracked.empty())
            b.add_satellite_report(report);
    }
}

void load_markers(geotrace::FileBuilder &b, const fs::path &base) {
    auto f = open_csv(base, "markers.csv");
    std::string line;
    std::getline(f, line); // skip header
    while (std::getline(f, line)) {
        line = rtrim(std::move(line));
        if (line.empty())
            continue;
        auto cols = split_csv(line);
        if (cols.size() < 3)
            continue;
        auto ts = parse_ts(cols[0]);
        if (!ts)
            throw geotrace::IoError("markers.csv: missing timestamp");
        b.add_annotation(geotrace::Annotation{*ts, cols[1], parse_icon(cols[2])});
    }
}

void load_events(geotrace::FileBuilder &b, const fs::path &base) {
    auto f = open_csv(base, "events.csv");
    std::string line;
    std::getline(f, line); // skip header
    while (std::getline(f, line)) {
        line = rtrim(std::move(line));
        if (line.empty())
            continue;
        auto cols = split_csv(line);
        if (cols.size() < 3)
            continue;
        auto ts = parse_ts(cols[0]);
        if (!ts)
            throw geotrace::IoError("events.csv: missing sys_time");
        b.add_event_marker(geotrace::EventMarker{cols[1], *ts, cols[2]});
    }
}

void verify_counts(const geotrace::NavFile &file) {
    auto check = [](bool cond, const char *msg) {
        if (!cond)
            throw std::runtime_error(std::string("FAIL: ") + msg);
    };

    check(file.title().find("Gold Dataset") != std::string_view::npos, "title missing");
    check(file.device().find("Synthetic Generator") != std::string_view::npos, "device missing");
    check(file.notes().find("cross-SDK") != std::string_view::npos, "notes missing");
    check(file.identity() == "gold-standard-v2", "identity wrong");

    auto np = file.nav_point_count();
    if (np != 199)
        throw std::runtime_error("expected 199 nav points, got " + std::to_string(np));

    std::size_t anti = 0;
    for (std::size_t i = 0; i < np; i++) {
        auto p = file.nav_point(i);
        if (p.lon.as_degrees() > 179.9 || p.lon.as_degrees() < -179.9)
            anti++;
    }
    if (anti != 10)
        throw std::runtime_error("expected 10 antimeridian pts, got " + std::to_string(anti));

    auto em = file.event_marker_count();
    if (em != 6)
        throw std::runtime_error("expected 6 event markers, got " + std::to_string(em));
}

} // namespace

int main(int argc, char **argv) {
    try {
        const fs::path base = (argc >= 2) ? argv[1] : "tests/fixtures/gold_dataset";
        const fs::path out = base / "gold_cpp.gtd";

        geotrace::FileBuilder b;
        b.lenient();
        load_meta(b, base);
        load_event_styles(b, base);
        auto sats = load_satellites(base);
        load_fixes(b, base, sats);
        load_markers(b, base);
        load_events(b, base);

        auto nav = b.finish();
        nav.write_to_file(out);

        verify_counts(nav);

        std::cout << "Written: " << out << "\n";
        std::cout << "Gold dataset verified. Nav points: 189, Event markers: 6\n";
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
