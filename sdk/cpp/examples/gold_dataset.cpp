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
#include <iterator>
#include <map>
#include <optional>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace fs = std::filesystem;

namespace {

std::string rtrim(std::string text) {
    while (!text.empty() && (text.back() == '\r' || text.back() == '\n' || text.back() == ' ')) {
        text.pop_back();
    }
    return text;
}

std::vector<std::string> split(const std::string &line, char delim) {
    std::vector<std::string> cols;
    std::istringstream stream(line);
    std::string col;
    while (std::getline(stream, col, delim)) {
        cols.push_back(std::move(col));
    }
    // std::getline drops the trailing empty field for lines that end with the delimiter.
    if (!line.empty() && line.back() == delim) {
        cols.emplace_back();
    }
    return cols;
}

std::vector<std::string> split_csv(const std::string &line) {
    return split(line, ',');
}

// The first N comma-separated fields of `line`, or `std::nullopt` when it holds fewer.
template <std::size_t N>
std::optional<std::array<std::string, N>> split_csv_fields(const std::string &line) {
    auto cols = split_csv(line);
    if (cols.size() < N) {
        return std::nullopt;
    }
    std::array<std::string, N> fields;
    std::move(cols.begin(), cols.begin() + static_cast<std::ptrdiff_t>(N), fields.begin());
    return fields;
}

bool is_leap(int year) noexcept {
    return (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
}

int month_days(int month, int year) noexcept {
    static constexpr std::array<int, 12> dom = {31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return (month == 2 && is_leap(year)) ? 29 : dom.at(static_cast<std::size_t>(month - 1));
}

/* Parse "YYYY-MM-DDTHH:MM:SS+HH:MM" to a Timestamp, or `std::nullopt` on failure. */
std::optional<geotrace::Timestamp> parse_ts(const std::string &text) {
    if (text.size() < 19) {
        return std::nullopt;
    }
    try {
        auto year = std::stoi(text.substr(0, 4));
        auto month = std::stoi(text.substr(5, 2));
        auto day = std::stoi(text.substr(8, 2));
        auto hour = std::stoi(text.substr(11, 2));
        auto minute = std::stoi(text.substr(14, 2));
        auto second = std::stoi(text.substr(17, 2));
        // Optional fractional seconds (".ffffff"), kept as microseconds.
        std::size_t pos = 19;
        long frac_us = 0;
        if (pos < text.size() && text.at(pos) == '.') {
            ++pos;
            std::string digits;
            while (pos < text.size() && text.at(pos) >= '0' && text.at(pos) <= '9') {
                digits.push_back(text.at(pos));
                ++pos;
            }
            digits.resize(6, '0'); // pad / truncate to microseconds
            frac_us = std::stol(digits);
        }
        // Optional timezone offset ("+HH:MM" / "-HH:MM").
        char sign = '+';
        int tz_hours = 0;
        int tz_minutes = 0;
        if (pos < text.size() && (text.at(pos) == '+' || text.at(pos) == '-')) {
            sign = text.at(pos);
            if (pos + 6 <= text.size()) {
                tz_hours = std::stoi(text.substr(pos + 1, 2));
                tz_minutes = std::stoi(text.substr(pos + 4, 2));
            }
        }
        long days = 0;
        for (int y = 1970; y < year; y++) {
            days += is_leap(y) ? 366 : 365;
        }
        for (int m = 1; m < month; m++) {
            days += month_days(m, year);
        }
        days += day - 1;
        long secs = (days * 86400L) + (hour * 3600L) + (minute * 60L) + second;
        const long tz_seconds = ((static_cast<long>(tz_hours) * 60L) + tz_minutes) * 60L;
        secs += (sign == '-') ? tz_seconds : -tz_seconds;
        const std::uint64_t micros =
            (static_cast<std::uint64_t>(secs) * 1000000ULL) + static_cast<std::uint64_t>(frac_us);
        return geotrace::Timestamp::from_micros(micros);
    } catch (const std::exception &) {
        return std::nullopt;
    }
}

geotrace::Constellation parse_constellation(const std::string &name) {
    if (name == "gps") {
        return geotrace::Constellation::Gps;
    }
    if (name == "glonass") {
        return geotrace::Constellation::Glonass;
    }
    if (name == "galileo") {
        return geotrace::Constellation::Galileo;
    }
    if (name == "beidou") {
        return geotrace::Constellation::Beidou;
    }
    throw std::invalid_argument("unknown constellation: " + name);
}

geotrace::MarkerIcon parse_icon(const std::string &name) {
    if (name.empty() || name == "auto") {
        return geotrace::MarkerIcon::Auto;
    }
    if (name == "pin") {
        return geotrace::MarkerIcon::Pin;
    }
    if (name == "cross") {
        return geotrace::MarkerIcon::Cross;
    }
    if (name == "circle") {
        return geotrace::MarkerIcon::Circle;
    }
    if (name == "lightning") {
        return geotrace::MarkerIcon::Lightning;
    }
    if (name == "warning") {
        return geotrace::MarkerIcon::Warning;
    }
    if (name == "error") {
        return geotrace::MarkerIcon::Error;
    }
    if (name == "check") {
        return geotrace::MarkerIcon::Check;
    }
    if (name == "satellite") {
        return geotrace::MarkerIcon::Satellite;
    }
    if (name == "satellite_lost") {
        return geotrace::MarkerIcon::SatelliteLost;
    }
    if (name == "gear") {
        return geotrace::MarkerIcon::Gear;
    }
    if (name == "refresh") {
        return geotrace::MarkerIcon::Refresh;
    }
    if (name == "download") {
        return geotrace::MarkerIcon::Download;
    }
    if (name == "upload") {
        return geotrace::MarkerIcon::Upload;
    }
    if (name == "wrench") {
        return geotrace::MarkerIcon::Wrench;
    }
    return geotrace::MarkerIcon::Auto;
}

std::optional<double> parse_opt_double(const std::string &text) {
    if (text.empty()) {
        return std::nullopt;
    }
    try {
        std::size_t pos = 0;
        double value = std::stod(text, &pos);
        return (pos > 0) ? std::optional<double>{value} : std::nullopt;
    } catch (const std::exception &) {
        return std::nullopt;
    }
}

struct SatRow {
    std::string gps_time;
    std::string sys_time;
    geotrace::Satellite sat;
};

geotrace::FixTime required_fix_time(const geotrace::RecordedFixTimestamps &recorded,
                                    const std::string &source) {
    const auto time = geotrace::FixTime::from_recorded(recorded);
    if (!time) {
        throw geotrace::IoError(source + " has no timestamp");
    }
    return *time;
}

std::ifstream open_csv(const fs::path &base, const std::string &name) {
    auto path = base / name;
    std::ifstream file(path);
    if (!file.is_open()) {
        throw geotrace::IoError("cannot open: " + path.string());
    }
    return file;
}

void load_meta(geotrace::FileBuilder &builder, const fs::path &base) {
    auto file = open_csv(base, "meta.csv");
    std::string line;
    std::getline(file, line); // skip header
    if (!std::getline(file, line)) {
        throw geotrace::IoError("meta.csv: missing data row");
    }
    auto fields = split_csv_fields<5>(rtrim(std::move(line)));
    if (!fields) {
        throw geotrace::IoError("meta.csv: need 5 columns");
    }
    const auto &[title, device, notes, identity, travel_mode_name] = *fields;
    auto travel_mode = geotrace::travel_mode_from_name(travel_mode_name);
    if (!travel_mode) {
        throw geotrace::IoError("meta.csv: unknown travel mode: " + travel_mode_name);
    }
    builder.title(title).device(device).notes(notes).identity(identity).travel_mode(*travel_mode);
}

void load_event_styles(geotrace::FileBuilder &builder, const fs::path &base) {
    auto file = open_csv(base, "event_styles.csv");
    std::string line;
    std::getline(file, line); // skip header
    while (std::getline(file, line)) {
        line = rtrim(std::move(line));
        if (line.empty()) {
            continue;
        }
        auto fields = split_csv_fields<3>(line);
        if (!fields) {
            continue;
        }
        const auto &[variant_path, icon, color] = *fields;
        builder.add_event_marker_style(
            geotrace::EventMarkerStyle{variant_path, parse_icon(icon), color});
    }
}

std::vector<SatRow> load_satellites(const fs::path &base) {
    auto file = open_csv(base, "satellites.csv");
    std::string line;
    std::getline(file, line); // skip header
    std::vector<SatRow> rows;
    while (std::getline(file, line)) {
        line = rtrim(std::move(line));
        if (line.empty()) {
            continue;
        }
        auto fields = split_csv_fields<8>(line);
        if (!fields) {
            continue;
        }
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

void load_fixes(geotrace::FileBuilder &builder, const fs::path &base,
                const std::vector<SatRow> &sats) {
    auto file = open_csv(base, "fixes.csv");
    std::set<std::pair<std::string, std::string>> fix_times;
    std::string line;
    std::getline(file, line); // skip header
    while (std::getline(file, line)) {
        line = rtrim(std::move(line));
        if (line.empty()) {
            continue;
        }
        auto fields = split_csv_fields<8>(line);
        if (!fields) {
            continue;
        }
        [[maybe_unused]] const auto &[track_id, gps_time, sys_time, lat, lon, heading_deg,
                                      speed_kmh, eph_m] = *fields;

        geotrace::RecordedFixTimestamps recorded{};
        recorded.gps_time = parse_ts(gps_time);
        recorded.sys_time = parse_ts(sys_time);
        const auto time = required_fix_time(recorded, "fixes.csv row " + line);

        auto hdg = parse_opt_double(heading_deg);
        auto kmh = parse_opt_double(speed_kmh);

        builder.add(geotrace::NavFix{
            time,
            geotrace::Angle::degrees(std::stod(lat)),
            geotrace::Angle::degrees(std::stod(lon)),
            hdg ? std::optional{geotrace::Angle::degrees(*hdg)} : std::nullopt,
            kmh ? std::optional{geotrace::Velocity::kmh(*kmh)} : std::nullopt,
            parse_opt_double(eph_m),
        });

        std::vector<geotrace::Satellite> tracked;
        for (const auto &row : sats) {
            if (row.gps_time == gps_time && row.sys_time == sys_time) {
                tracked.push_back(row.sat);
            }
        }
        if (!tracked.empty()) {
            builder.add(geotrace::SatelliteReport{time, std::move(tracked)});
        }

        fix_times.emplace(gps_time, sys_time);
    }

    // Reports at a time no fix row holds. The builder gives each one a ghost fix.
    std::map<std::pair<std::string, std::string>, std::vector<geotrace::Satellite>> orphans;
    for (const auto &row : sats) {
        if (fix_times.find({row.gps_time, row.sys_time}) == fix_times.end()) {
            orphans[{row.gps_time, row.sys_time}].push_back(row.sat);
        }
    }
    for (const auto &[times, tracked] : orphans) {
        geotrace::RecordedFixTimestamps recorded{};
        recorded.gps_time = parse_ts(times.first);
        recorded.sys_time = parse_ts(times.second);

        const auto time = required_fix_time(recorded, "satellites.csv row (" + times.first + ", " +
                                                          times.second + ")");
        builder.add(geotrace::SatelliteReport{time, tracked});
    }
}

void load_markers(geotrace::FileBuilder &builder, const fs::path &base) {
    auto file = open_csv(base, "markers.csv");
    std::string line;
    std::getline(file, line); // skip header
    while (std::getline(file, line)) {
        line = rtrim(std::move(line));
        if (line.empty()) {
            continue;
        }
        auto fields = split_csv_fields<3>(line);
        if (!fields) {
            continue;
        }
        const auto &[time, label, icon] = *fields;
        auto timestamp = parse_ts(time);
        if (!timestamp) {
            throw geotrace::IoError("markers.csv: missing timestamp");
        }
        builder.add(geotrace::Annotation{*timestamp, label, parse_icon(icon)});
    }
}

void load_events(geotrace::FileBuilder &builder, const fs::path &base) {
    auto file = open_csv(base, "events.csv");
    std::string line;
    std::getline(file, line); // skip header
    while (std::getline(file, line)) {
        line = rtrim(std::move(line));
        if (line.empty()) {
            continue;
        }
        auto fields = split_csv_fields<3>(line);
        if (!fields) {
            continue;
        }
        const auto &[sys_time, variant_path, annotation] = *fields;
        auto timestamp = parse_ts(sys_time);
        if (!timestamp) {
            throw geotrace::IoError("events.csv: missing sys_time");
        }
        builder.add(geotrace::EventMarker{variant_path, *timestamp, annotation});
    }
}

// Returns the channel named in the row. Appends a new channel from the row's
// unit, period, description and component columns when no earlier row named it.
geotrace::Channel &channel_for_row(std::vector<geotrace::Channel> &channels,
                                   const std::array<std::string, 7> &fields) {
    const auto &[name, unit, period_deg, description, components, time, values] = fields;
    for (auto &existing : channels) {
        if (existing.name == name) {
            return existing;
        }
    }
    geotrace::Channel channel;
    channel.name = name;
    if (!unit.empty()) {
        channel.unit = geotrace::ChannelUnit::parse_recognized(unit);
    }
    if (!period_deg.empty()) {
        channel.period = geotrace::Angle::degrees(std::stod(period_deg));
    }
    channel.description = description;
    if (!components.empty()) {
        channel.components = split(components, ';');
    }
    channels.push_back(std::move(channel));
    return channels.back();
}

void load_channels(geotrace::FileBuilder &builder, const fs::path &base) {
    auto file = open_csv(base, "channels.csv");
    std::string line;
    std::getline(file, line); // skip header

    // One Channel per name, built up across its rows (each row is one sample).
    std::vector<geotrace::Channel> channels;
    while (std::getline(file, line)) {
        line = rtrim(std::move(line));
        if (line.empty()) {
            continue;
        }
        auto fields = split_csv_fields<7>(line);
        if (!fields) {
            continue;
        }
        [[maybe_unused]] const auto &[name, unit, period_deg, description, components, time,
                                      values] = *fields;

        geotrace::Channel &channel = channel_for_row(channels, *fields);
        auto timestamp = parse_ts(time);
        if (!timestamp) {
            throw geotrace::IoError("channels.csv: invalid timestamp");
        }
        channel.times.push_back(*timestamp);
        for (const auto &value : split(values, ';')) {
            channel.values.push_back(std::stod(value));
        }
    }

    for (const auto &channel : channels) {
        builder.add_channel(channel);
    }
}

void verify_counts(const geotrace::NavFile &file) {
    auto check = [](bool cond, const char *msg) {
        if (!cond) {
            throw std::runtime_error(std::string("FAIL: ") + msg);
        }
    };

    check(file.title().find("Gold Dataset") != std::string_view::npos, "title missing");
    check(file.device().find("Synthetic Generator") != std::string_view::npos, "device missing");
    check(file.notes().find("cross-SDK") != std::string_view::npos, "notes missing");
    check(file.identity() == "gold-standard-v2", "identity wrong");
    check(file.travel_mode() == "bicycle", "travel mode wrong");

    auto nav_points = file.nav_point_count();
    if (nav_points != 200) {
        throw std::runtime_error("expected 200 nav points, got " + std::to_string(nav_points));
    }

    std::size_t anti = 0;
    for (std::size_t i = 0; i < nav_points; i++) {
        auto point = file.nav_point(i);
        if (point.lon.as_degrees() > 179.9 || point.lon.as_degrees() < -179.9) {
            anti++;
        }
    }
    if (anti != 11) {
        throw std::runtime_error("expected 11 antimeridian pts, got " + std::to_string(anti));
    }

    auto event_markers = file.event_marker_count();
    if (event_markers != 7) {
        throw std::runtime_error("expected 7 event markers, got " + std::to_string(event_markers));
    }

    auto channel_count = file.channel_count();
    if (channel_count != 2) {
        throw std::runtime_error("expected 2 channels, got " + std::to_string(channel_count));
    }
    for (std::size_t i = 0; i < channel_count; i++) {
        auto channel = file.channel(i);
        if (channel.name == "accel") {
            check(channel.is_vector() &&
                      channel.components == std::vector<std::string>{"x", "y", "z"},
                  "accel components wrong");
        }
        if (channel.name == "heading_raw") {
            check(channel.period.has_value() && channel.period->as_degrees() == 360.0,
                  "heading_raw period wrong");
        }
    }
}

} // namespace

int main(int argc, char **argv) {
    try {
        const std::vector<std::string> args(argv, std::next(argv, argc));
        const fs::path base = args.size() >= 2 ? args.at(1) : "tests/fixtures/gold_dataset";
        const fs::path out = base / "gold_cpp.gtd";

        geotrace::FileBuilder builder{};
        builder.lenient();
        load_meta(builder, base);
        load_event_styles(builder, base);
        auto sats = load_satellites(base);
        load_fixes(builder, base, sats);
        load_markers(builder, base);
        load_events(builder, base);
        load_channels(builder, base);

        auto nav = builder.finish();
        nav.write_to_file(out);

        verify_counts(nav);

        std::cout << "Written: " << out << "\n";
        std::cout << "Gold dataset verified. Nav points: 200, Event markers: 7, Channels: 2\n";
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
