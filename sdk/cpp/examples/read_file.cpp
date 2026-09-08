/**
 * Open a .gtd file and print a summary of its contents.
 *
 * Pass a path on the command line to inspect an existing file:
 *
 *     ./read_file path/to/file.gtd
 *
 * With no argument the example first writes a small file to a temp directory
 * and then reads that back, so it is runnable on its own.
 */

#include <geotrace/geotrace.hpp>
#include <geotrace/unit_catalog.hpp>

#include <array>
#include <cstddef>
#include <cstdint>
#include <exception>
#include <filesystem>
#include <iomanip>
#include <iostream>
#include <iterator>
#include <string>
#include <vector>

namespace {

// 2024-06-01T08:00:00Z keeps the sample file deterministic.
constexpr std::int64_t kBase = 1717228800;

void print_nav_points(const geotrace::NavFile &file) {
    if (file.nav_point_count() == 0) {
        return;
    }

    std::cout << "Nav points: " << file.nav_point_count() << "\n";
    for (std::size_t i = 0; i < file.nav_point_count(); ++i) {
        const auto point = file.nav_point(i);
        std::cout << "  [" << i << "] " << std::fixed << std::setprecision(5)
                  << point.lat.as_degrees() << ", " << point.lon.as_degrees();
        if (point.speed) {
            std::cout << "  " << std::setprecision(1) << point.speed->as_mps() << " m/s";
        }
        if (point.satellite_count > 0) {
            std::cout << "  sats=" << point.satellite_count;
        }
        std::cout << "\n";
    }
}

void print_markers(const geotrace::NavFile &file) {
    if (file.marker_count() == 0) {
        return;
    }

    std::cout << "Markers: " << file.marker_count() << "\n";
    for (std::size_t i = 0; i < file.marker_count(); ++i) {
        const auto marker = file.marker(i);
        std::cout << "  [" << i << "] " << std::fixed << std::setprecision(5)
                  << marker.lat.as_degrees() << ", " << marker.lon.as_degrees()
                  << "  icon=" << static_cast<std::uint32_t>(marker.icon_code);
        if (!marker.label.empty()) {
            std::cout << " - " << marker.label;
        }
        std::cout << "\n";
    }
}

void print_event_markers(const geotrace::NavFile &file) {
    if (file.event_marker_count() == 0) {
        return;
    }

    std::cout << "Event markers: " << file.event_marker_count() << "\n";
    for (std::size_t i = 0; i < file.event_marker_count(); ++i) {
        const auto marker = file.event_marker(i);
        std::cout << "  [" << i << "] " << marker.variant_path;
        if (!marker.annotation.empty()) {
            std::cout << " - " << marker.annotation;
        }
        std::cout << "\n";
    }
}

void print_event_marker_styles(const geotrace::NavFile &file) {
    if (file.event_marker_style_count() == 0) {
        return;
    }

    std::cout << "Event marker styles: " << file.event_marker_style_count() << "\n";
    for (std::size_t i = 0; i < file.event_marker_style_count(); ++i) {
        const auto style = file.event_marker_style(i);
        const std::string icon = style.icon_name.empty() ? "auto" : style.icon_name;
        const std::string color = style.color_hex.empty() ? "auto" : style.color_hex;
        std::cout << "  [" << i << "] " << style.variant_path << "  icon=" << icon
                  << "  color=" << color << "\n";
    }
}

void print_channels(const geotrace::NavFile &file) {
    if (file.channel_count() == 0) {
        return;
    }

    std::cout << "Channels: " << file.channel_count() << "\n";
    for (std::size_t i = 0; i < file.channel_count(); ++i) {
        const auto channel = file.channel(i);
        std::cout << "  [" << i << "] " << channel.name << ' ' << channel.times.size()
                  << " samples";
        if (channel.unit) {
            std::cout << " [" << channel.unit->label() << ']';
        }
        if (channel.is_vector()) {
            std::cout << " components:";
            for (const auto &component : channel.components) {
                std::cout << ' ' << component;
            }
        }
        std::cout << "\n";
    }
}

/** Write a sample file holding one of every section this example prints. */
void write_sample_file(const std::filesystem::path &path) {
    const auto timestamp_at = [](std::int64_t secs) {
        return geotrace::Timestamp::from_seconds(kBase + secs);
    };

    geotrace::FileBuilder builder{};
    builder.title("Sample track").device("Example GPS v1.0");

    struct TrackPoint {
        std::int64_t offset_s;
        double lat;
        double lon;
    };
    const std::array<TrackPoint, 3> track = {{
        {0, 51.5074, -0.1278},
        {30, 51.5088, -0.1248},
        {60, 51.5103, -0.1217},
    }};
    for (const auto &point : track) {
        geotrace::NavFix fix{geotrace::FixTime::receiver(timestamp_at(point.offset_s)),
                             geotrace::Angle::degrees(point.lat),
                             geotrace::Angle::degrees(point.lon)};
        fix.heading = geotrace::Angle::degrees(90.0);
        fix.speed = geotrace::Velocity::mps(5.5);
        builder.add(fix);
    }

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
    galileo_prn3.snr_dbhz = 22.0F;

    builder.add(geotrace::SatelliteReport{geotrace::FixTime::receiver(timestamp_at(0)),
                                          {gps_prn1, galileo_prn3}});
    builder.add(geotrace::Annotation{timestamp_at(10), "Start point", geotrace::MarkerIcon::Pin});
    builder.add(geotrace::EventMarker{"power/boot", timestamp_at(2), "cold start"});
    builder.add_event_marker_style(
        geotrace::EventMarkerStyle{"power/boot", geotrace::MarkerIcon::Lightning, "#44BB44"});

    geotrace::Channel incline{};
    incline.name = "incline";
    incline.unit = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Deg);
    incline.times = {timestamp_at(0), timestamp_at(30), timestamp_at(60)};
    incline.values = {1.0, 1.5, 2.0};
    builder.add(incline);

    builder.finish().write_to_file(path);
}

} // namespace

int main(int argc, char **argv) {
    try {
        const std::vector<std::string> args(argv, std::next(argv, argc));
        const std::filesystem::path sample =
            std::filesystem::temp_directory_path() / "geotrace_read_file_sample.gtd";
        const bool generated = args.size() < 2;
        if (generated) {
            write_sample_file(sample);
        }
        const std::filesystem::path path = generated ? sample : std::filesystem::path(args.at(1));

        const geotrace::NavFile file = geotrace::NavFile::open(path);

        if (!file.title().empty()) {
            std::cout << "Title:  " << file.title() << "\n";
        }
        if (!file.device().empty()) {
            std::cout << "Device: " << file.device() << "\n";
        }

        print_nav_points(file);
        print_markers(file);
        print_event_markers(file);
        print_event_marker_styles(file);
        print_channels(file);

        if (generated) {
            std::filesystem::remove(sample);
        }
    } catch (const std::exception &e) {
        std::cerr << "error: " << e.what() << "\n";
        return 1;
    }
}
