/**
 * Gold dataset reference test for the GeoTrace C++ SDK.
 *
 * Reads the CSV fixtures in tests/fixtures/gold_dataset/, builds a .gtd file,
 * then verifies the round-trip.  Run from the repository root:
 *
 *   ./sdk/cpp/build/gold/examples/gold_dataset
 */

#include <geotrace/geotrace.hpp>

#include <algorithm>
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

std::vector<std::string> split(const std::string &line, char delim) {
    std::vector<std::string> cols;
    std::istringstream ss(line);
    std::string col;
    while (std::getline(ss, col, delim))
        cols.push_back(std::move(col));
    // std::getline drops the trailing empty field for lines that end with the delimiter.
    if (!line.empty() && line.back() == delim)
        cols.emplace_back();
    return cols;
}

std::vector<std::string> split_csv(const std::string &line) {
    return split(line, ',');
}

// The first N comma-separated fields of `line`, or `std::nullopt` when it holds fewer.
template <std::size_t N>
std::optional<std::array<std::string, N>> split_csv_fields(const std::string &line) {
    auto cols = split_csv(line);
    if (cols.size() < N)
        return std::nullopt;
    std::array<std::string, N> fields;
    std::move(cols.begin(), cols.begin() + static_cast<std::ptrdiff_t>(N), fields.begin());
    return fields;
}

bool is_leap(int y) noexcept {
    return (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
}

int month_days(int m, int y) noexcept {
    static constexpr std::array<int, 12> dom = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return (m == 2 && is_leap(y)) ? 29 : dom.at(static_cast<std::size_t>(m - 1));
}

/* Parse "YYYY-MM-DDTHH:MM:SS+HH:MM" to a Timestamp, or `std::nullopt` on failure. */
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
        // Optional fractional seconds (".ffffff"), kept as microseconds.
        std::size_t i = 19;
        long frac_us = 0;
        if (i < s.size() && s.at(i) == '.') {
            ++i;
            std::string digits;
            while (i < s.size() && s.at(i) >= '0' && s.at(i) <= '9') {
                digits.push_back(s.at(i));
                ++i;
            }
            digits.resize(6, '0'); // pad / truncate to microseconds
            frac_us = std::stol(digits);
        }
        // Optional timezone offset ("+HH:MM" / "-HH:MM").
        char sign = '+';
        int tz_h = 0, tz_m = 0;
        if (i < s.size() && (s.at(i) == '+' || s.at(i) == '-')) {
            sign = s.at(i);
            if (i + 6 <= s.size()) {
                tz_h = std::stoi(s.substr(i + 1, 2));
                tz_m = std::stoi(s.substr(i + 4, 2));
            }
        }
        long days = 0;
        for (int y = 1970; y < Y; y++)
            days += is_leap(y) ? 366 : 365;
        for (int m = 1; m < Mo; m++)
            days += month_days(m, Y);
        days += D - 1;
        long secs = (days * 86400L) + (H * 3600L) + (Mi * 60L) + S;
        const long tz = ((static_cast<long>(tz_h) * 60L) + tz_m) * 60L;
        secs += (sign == '-') ? tz : -tz;
        const std::uint64_t micros =
            (static_cast<std::uint64_t>(secs) * 1000000ULL) + static_cast<std::uint64_t>(frac_us);
        return geotrace::Timestamp::from_micros(micros);
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
    auto fields = split_csv_fields<5>(rtrim(std::move(line)));
    if (!fields)
        throw geotrace::IoError("meta.csv: need 5 columns");
    const auto &[title, device, notes, identity, travel_mode_name] = *fields;
    auto travel_mode = geotrace::travel_mode_from_name(travel_mode_name);
    if (!travel_mode)
        throw geotrace::IoError("meta.csv: unknown travel mode: " + travel_mode_name);
    b.title(title).device(device).notes(notes).identity(identity).travel_mode(*travel_mode);
}

void load_event_styles(geotrace::FileBuilder &b, const fs::path &base) {
    auto f = open_csv(base, "event_styles.csv");
    std::string line;
    std::getline(f, line); // skip header
    while (std::getline(f, line)) {
        line = rtrim(std::move(line));
        if (line.empty())
            continue;
        auto fields = split_csv_fields<3>(line);
        if (!fields)
            continue;
        const auto &[variant_path, icon, color] = *fields;
        b.add_event_marker_style(geotrace::EventMarkerStyle{variant_path, parse_icon(icon), color});
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
        auto fields = split_csv_fields<8>(line);
        if (!fields)
            continue;
        const auto &[gps_time, sys_time, constellation, prn, in_fix, elevation, azimuth, snr] =
            *fields;
        rows.push_back(SatRow{
            gps_time,
            sys_time,
            geotrace::Satellite{
                parse_constellation(constellation),
                static_cast<std::uint32_t>(std::stoul(prn)),
                in_fix == "true",
                parse_opt_double(elevation),
                parse_opt_double(azimuth),
                parse_opt_double(snr),
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
        auto fields = split_csv_fields<8>(line);
        if (!fields)
            continue;
        [[maybe_unused]] const auto &[track_id, gps_time, sys_time, lat, lon, heading_deg,
                                      speed_kmh, eph_m] = *fields;

        auto gps_ts = parse_ts(gps_time);
        auto sys_ts = parse_ts(sys_time);
        auto hdg = parse_opt_double(heading_deg);
        auto kmh = parse_opt_double(speed_kmh);

        b.add(geotrace::NavFix{
            gps_ts.value_or(geotrace::Timestamp::none()),
            sys_ts.value_or(geotrace::Timestamp::none()),
            geotrace::Angle::degrees(std::stod(lat)),
            geotrace::Angle::degrees(std::stod(lon)),
            hdg ? std::optional{geotrace::Angle::degrees(*hdg)} : std::nullopt,
            kmh ? std::optional{geotrace::Velocity::kmh(*kmh)} : std::nullopt,
            parse_opt_double(eph_m),
        });

        geotrace::SatelliteReport report{};
        report.gps_time = gps_ts.value_or(geotrace::Timestamp::none());
        report.sys_time = sys_ts.value_or(geotrace::Timestamp::none());
        for (const auto &row : sats) {
            if (row.gps_time == gps_time && row.sys_time == sys_time)
                report.tracked.push_back(row.sat);
        }
        if (!report.tracked.empty())
            b.add(report);
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
        auto fields = split_csv_fields<3>(line);
        if (!fields)
            continue;
        const auto &[time, label, icon] = *fields;
        auto ts = parse_ts(time);
        if (!ts)
            throw geotrace::IoError("markers.csv: missing timestamp");
        b.add(geotrace::Annotation{*ts, label, parse_icon(icon)});
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
        auto fields = split_csv_fields<3>(line);
        if (!fields)
            continue;
        const auto &[sys_time, variant_path, annotation] = *fields;
        auto ts = parse_ts(sys_time);
        if (!ts)
            throw geotrace::IoError("events.csv: missing sys_time");
        b.add(geotrace::EventMarker{variant_path, *ts, annotation});
    }
}

void load_channels(geotrace::FileBuilder &b, const fs::path &base) {
    auto f = open_csv(base, "channels.csv");
    std::string line;
    std::getline(f, line); // skip header

    // One Channel per name, built up across its rows (each row is one sample).
    std::vector<geotrace::Channel> channels;
    while (std::getline(f, line)) {
        line = rtrim(std::move(line));
        if (line.empty())
            continue;
        auto fields = split_csv_fields<7>(line);
        if (!fields)
            continue;
        const auto &[name, unit, period_deg, description, components, time, values] = *fields;

        geotrace::Channel *ch = nullptr;
        for (auto &existing : channels) {
            if (existing.name == name) {
                ch = &existing;
                break;
            }
        }
        if (ch == nullptr) {
            geotrace::Channel channel;
            channel.name = name;
            if (!unit.empty())
                channel.unit = geotrace::ChannelUnit::parse_recognized(unit);
            if (!period_deg.empty())
                channel.period = geotrace::Angle::degrees(std::stod(period_deg));
            channel.description = description;
            if (!components.empty())
                channel.components = split(components, ';');
            channels.push_back(std::move(channel));
            ch = &channels.back();
        }

        auto ts = parse_ts(time);
        if (!ts)
            throw geotrace::IoError("channels.csv: invalid timestamp");
        ch->times.push_back(*ts);
        for (const auto &value : split(values, ';'))
            ch->values.push_back(std::stod(value));
    }

    for (const auto &channel : channels)
        b.add_channel(channel);
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
    check(file.travel_mode() == "bicycle", "travel mode wrong");

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

    auto nch = file.channel_count();
    if (nch != 2)
        throw std::runtime_error("expected 2 channels, got " + std::to_string(nch));
    for (std::size_t i = 0; i < nch; i++) {
        auto ch = file.channel(i);
        if (ch.name == "accel")
            check(ch.is_vector() && ch.components == std::vector<std::string>{"x", "y", "z"},
                  "accel components wrong");
        if (ch.name == "heading_raw")
            check(ch.period.has_value() && ch.period->as_degrees() == 360.0,
                  "heading_raw period wrong");
    }
}

} // namespace

int main(int argc, char **argv) {
    try {
        const fs::path base = (argc >= 2) ? argv[1] : "tests/fixtures/gold_dataset";
        const fs::path out = base / "gold_cpp.gtd";

        geotrace::FileBuilder b{};
        b.lenient();
        load_meta(b, base);
        load_event_styles(b, base);
        auto sats = load_satellites(base);
        load_fixes(b, base, sats);
        load_markers(b, base);
        load_events(b, base);
        load_channels(b, base);

        auto nav = b.finish();
        nav.write_to_file(out);

        verify_counts(nav);

        std::cout << "Written: " << out << "\n";
        std::cout << "Gold dataset verified. Nav points: 189, Event markers: 6, Channels: 2\n";
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
