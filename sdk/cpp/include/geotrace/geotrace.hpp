/**
 * @file geotrace.hpp
 * @brief GeoTrace C++ SDK - idiomatic C++17 header-only wrapper over the GeoTrace C SDK.
 *
 * Usage:
 * @code
 * #include <geotrace/geotrace.hpp>
 * @endcode
 *
 * Link against `GeoTrace::Cpp` (which transitively links `GeoTrace::C`).
 *
 * All types live in the `geotrace` namespace.
 * All errors are reported as exceptions derived from `geotrace::Error`.
 */

#pragma once

#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include <geotrace.h>   // C SDK (already has extern "C" guards)

#define GEOTRACE_CPP_VERSION       "0.1.0"
#define GEOTRACE_CPP_VERSION_MAJOR 0
#define GEOTRACE_CPP_VERSION_MINOR 1
#define GEOTRACE_CPP_VERSION_PATCH 0

// Minimal std::span polyfill for C++17.  Replaced by std::span on C++20.
#if defined(__cpp_lib_span) && __cpp_lib_span >= 202002L
#  include <span>
namespace geotrace { template <typename T> using span = std::span<T>; }
#else
namespace geotrace {

template <typename T>
class span {
public:
    using element_type = T;
    using value_type   = std::remove_cv_t<T>;
    using size_type    = std::size_t;
    using pointer      = T*;
    using iterator     = pointer;

    constexpr span() noexcept = default;
    constexpr span(pointer ptr, size_type count) noexcept : data_(ptr), size_(count) {}

    template <std::size_t N>
    constexpr span(T (&arr)[N]) noexcept : data_(arr), size_(N) {}

    template <typename Container>
    constexpr span(Container& c) noexcept
        : data_(c.data()), size_(c.size()) {}

    template <typename Container>
    constexpr span(const Container& c) noexcept
        : data_(c.data()), size_(c.size()) {}

    constexpr pointer   data()  const noexcept { return data_; }
    constexpr size_type size()  const noexcept { return size_; }
    constexpr bool      empty() const noexcept { return size_ == 0; }
    constexpr iterator  begin() const noexcept { return data_; }
    constexpr iterator  end()   const noexcept { return data_ + size_; }
    constexpr T&        operator[](size_type i) const noexcept { return data_[i]; }

private:
    pointer   data_ = nullptr;
    size_type size_ = 0;
};

} // namespace geotrace
#endif


namespace geotrace {

// Forward declarations
class NavFile;


/** Base for all GeoTrace SDK errors. */
struct Error : std::runtime_error {
    using std::runtime_error::runtime_error;
};

/** Base for errors that occur during file construction. */
struct BuildError : Error { using Error::Error; };

/** `finish()` was called without adding any nav fixes. */
struct NoNavFixesError : BuildError { using BuildError::BuildError; };

/** One or more annotations fall outside the nav fix time range. */
struct AnnotationsOutOfRangeError : BuildError {
    std::size_t count;
    AnnotationsOutOfRangeError(std::size_t n, const std::string& msg)
        : BuildError(msg), count(n) {}
};

/** I/O error (file not found, permission denied, etc.). */
struct IoError : Error { using Error::Error; };

/** HDF5 library error. */
struct Hdf5Error : Error { using Error::Error; };

/** File was written by a newer, incompatible SDK version. */
struct UnsupportedVersionError : Error { using Error::Error; };

/** A variant path argument was malformed. */
struct InvalidPathError : Error { using Error::Error; };


namespace detail {

[[noreturn]] inline void throw_status(GtdStatus s) {
    const char* raw = ::gtd_last_error();
    std::string msg = (raw != nullptr) ? raw : "unknown error";
    switch (s) {
        case GTD_ERR_NO_NAV_FIXES:
            throw NoNavFixesError(msg);
        case GTD_ERR_ANNOTATIONS_OOB:
            throw AnnotationsOutOfRangeError(0, msg);
        case GTD_ERR_INVALID_PATH:
            throw InvalidPathError(msg);
        case GTD_ERR_IO:
            throw IoError(msg);
        case GTD_ERR_HDF5:
            throw Hdf5Error(msg);
        case GTD_ERR_VERSION:
            throw UnsupportedVersionError(msg);
        default:
            throw Error(msg);
    }
}

inline void check(GtdStatus s) {
    if (s != GTD_OK) throw_status(s);
}

inline void check_range(GtdStatus s) {
    if (s == GTD_ERR_NULL_ARGUMENT) {
        const char* raw = ::gtd_last_error();
        throw std::out_of_range((raw != nullptr) ? raw : "index out of range");
    }
    if (s != GTD_OK) throw_status(s);
}

inline GtdOptF64 to_c(std::optional<double> v) noexcept {
    return v ? GTD_SOME_F64(*v) : GTD_NONE_F64;
}

} // namespace detail


/**
 * UTC Unix epoch timestamp in microseconds.
 *
 * Use `Timestamp::none()` to represent an absent value.
 */
struct Timestamp {
    std::int64_t unix_micros = 0;

    static Timestamp none() noexcept {
        return Timestamp{::gtd_ts_none().unix_micros};
    }
    static Timestamp from_seconds(std::uint64_t s) noexcept {
        return Timestamp{::gtd_ts_from_seconds(s).unix_micros};
    }
    static Timestamp from_millis(std::uint64_t ms) noexcept {
        return Timestamp{::gtd_ts_from_millis(ms).unix_micros};
    }
    static Timestamp from_micros(std::uint64_t us) noexcept {
        return Timestamp{::gtd_ts_from_micros(us).unix_micros};
    }
    static Timestamp from_nanos(std::uint64_t ns) noexcept {
        return Timestamp{::gtd_ts_from_nanos(ns).unix_micros};
    }

    bool is_none() const noexcept {
        return ::gtd_ts_is_none(GtdTimestamp{unix_micros}) != 0;
    }

    bool operator==(Timestamp other) const noexcept {
        return unix_micros == other.unix_micros;
    }
    bool operator!=(Timestamp other) const noexcept { return !(*this == other); }
};

/** Angular measurement stored in degrees. */
class Angle {
public:
    static Angle degrees(double deg) noexcept { return Angle{deg}; }
    static Angle radians(double rad) noexcept {
        return Angle{rad * (180.0 / M_PI)};
    }

    double as_degrees() const noexcept { return deg_; }
    double as_radians() const noexcept { return deg_ * (M_PI / 180.0); }

    bool operator==(Angle other) const noexcept { return deg_ == other.deg_; }
    bool operator!=(Angle other) const noexcept { return !(*this == other); }

private:
    explicit Angle(double deg) noexcept : deg_(deg) {}
    double deg_ = 0.0;
};

/** Velocity stored in metres per second. */
class Velocity {
public:
    static Velocity mps  (double v) noexcept { return Velocity{v}; }
    static Velocity kmh  (double v) noexcept { return Velocity{v / 3.6}; }
    static Velocity knots(double v) noexcept { return Velocity{v * 0.514444}; }

    double as_mps()   const noexcept { return mps_; }
    double as_kmh()   const noexcept { return mps_ * 3.6; }
    double as_knots() const noexcept { return mps_ / 0.514444; }

    bool operator==(Velocity other) const noexcept { return mps_ == other.mps_; }
    bool operator!=(Velocity other) const noexcept { return !(*this == other); }

private:
    explicit Velocity(double mps) noexcept : mps_(mps) {}
    double mps_ = 0.0;
};


enum class Constellation { Gps, Glonass, Galileo, Beidou };

enum class MarkerIcon {
    Pin, Cross, Circle, Lightning, Warning, Error, Check,
    Satellite, SatelliteLost, Gear, Refresh, Download, Upload, Wrench,
    Auto,
};

namespace detail {

inline GtdConstellation to_c(Constellation c) noexcept {
    switch (c) {
        case Constellation::Gps:     return GTD_CONSTELLATION_GPS;
        case Constellation::Glonass: return GTD_CONSTELLATION_GLONASS;
        case Constellation::Galileo: return GTD_CONSTELLATION_GALILEO;
        case Constellation::Beidou:  return GTD_CONSTELLATION_BEIDOU;
    }
    return GTD_CONSTELLATION_GPS;
}

inline Constellation from_c(GtdConstellation c) noexcept {
    switch (c) {
        case GTD_CONSTELLATION_GPS:     return Constellation::Gps;
        case GTD_CONSTELLATION_GLONASS: return Constellation::Glonass;
        case GTD_CONSTELLATION_GALILEO: return Constellation::Galileo;
        case GTD_CONSTELLATION_BEIDOU:  return Constellation::Beidou;
    }
    return Constellation::Gps;
}

inline GtdMarkerIcon to_c(MarkerIcon icon) noexcept {
    switch (icon) {
        case MarkerIcon::Pin:           return GTD_ICON_PIN;
        case MarkerIcon::Cross:         return GTD_ICON_CROSS;
        case MarkerIcon::Circle:        return GTD_ICON_CIRCLE;
        case MarkerIcon::Lightning:     return GTD_ICON_LIGHTNING;
        case MarkerIcon::Warning:       return GTD_ICON_WARNING;
        case MarkerIcon::Error:         return GTD_ICON_ERROR;
        case MarkerIcon::Check:         return GTD_ICON_CHECK;
        case MarkerIcon::Satellite:     return GTD_ICON_SATELLITE;
        case MarkerIcon::SatelliteLost: return GTD_ICON_SATELLITE_LOST;
        case MarkerIcon::Gear:          return GTD_ICON_GEAR;
        case MarkerIcon::Refresh:       return GTD_ICON_REFRESH;
        case MarkerIcon::Download:      return GTD_ICON_DOWNLOAD;
        case MarkerIcon::Upload:        return GTD_ICON_UPLOAD;
        case MarkerIcon::Wrench:        return GTD_ICON_WRENCH;
        case MarkerIcon::Auto:          return GTD_ICON_AUTO;
    }
    return GTD_ICON_AUTO;
}

inline GtdTimestamp to_c(Timestamp ts) noexcept {
    return GtdTimestamp{ts.unix_micros};
}

inline Timestamp from_c(GtdTimestamp ts) noexcept {
    return Timestamp{ts.unix_micros};
}

} // namespace detail


/** A single GPS navigation fix. */
struct NavFix {
    Timestamp              gps_time = Timestamp::none();
    Timestamp              sys_time = Timestamp::none();
    Angle                  lat      = Angle::degrees(0.0);
    Angle                  lon      = Angle::degrees(0.0);
    std::optional<Angle>   heading;
    std::optional<Velocity> speed;
    std::optional<double>  eph_m;
};

/** One satellite in a visibility report. */
struct Satellite {
    Constellation         constellation = Constellation::Gps;
    std::uint32_t         prn           = 0;
    bool                  in_fix        = false;
    std::optional<double> elevation_deg;
    std::optional<double> azimuth_deg;
    std::optional<double> snr_dbhz;
};

/** A snapshot of satellite visibility at a point in time. */
struct SatelliteReport {
    Timestamp              gps_time = Timestamp::none();
    Timestamp              sys_time = Timestamp::none();
    std::vector<Satellite> tracked;
};

/** A legacy map-pin annotation. */
struct Annotation {
    Timestamp   time;
    std::string label;
    MarkerIcon  icon = MarkerIcon::Auto;
};

/** A structured event marker placed on the map. */
struct EventMarker {
    std::string variant_path;
    Timestamp   sys_time;
    std::string annotation;  // empty = none
};

/** Display style for all events of a given variant. */
struct EventMarkerStyle {
    std::string variant_path;
    MarkerIcon  icon      = MarkerIcon::Auto;
    std::string color_hex;  // empty = auto (hash-derived); format: "#RRGGBB"
};


/** Data for one navigation fix, returned by `NavFile::nav_point()`. */
struct NavPointView {
    Timestamp                gps_time;
    Timestamp                sys_time;
    Angle                    lat = Angle::degrees(0.0);
    Angle                    lon = Angle::degrees(0.0);
    std::optional<Angle>     heading;
    std::optional<Velocity>  speed;
    std::optional<double>    eph_m;
    std::size_t              satellite_count = 0;
};

/** Satellite data for one tracked satellite, returned by `NavFile::satellite()`. */
struct SatelliteView {
    Constellation         constellation = Constellation::Gps;
    std::uint32_t         prn    = 0;
    bool                  in_fix = false;
    std::optional<double> elevation_deg;
    std::optional<double> azimuth_deg;
    std::optional<double> snr_dbhz;
};

/**
 * Event marker data returned by `NavFile::event_marker()`.
 *
 * String fields are copies - they are valid regardless of the `NavFile`'s lifetime.
 */
struct EventMarkerView {
    std::string variant_path;
    Timestamp   sys_time;
    Angle       lat = Angle::degrees(0.0);
    Angle       lon = Angle::degrees(0.0);
    std::string annotation;  // empty if none
};


/**
 * Constructs a GeoTrace navigation file.
 *
 * Call `add_nav_fix()` (at least once), then `std::move(builder).finish()`
 * to produce a `NavFile`.
 *
 * **Non-copyable; movable.**  Destroyed automatically if `finish()` is never called.
 */
class FileBuilder {
public:
    /**
     * Create a new builder.
     *
     * @throws std::bad_alloc on allocation failure.
     */
    FileBuilder() : impl_(::gtd_builder_create()) {
        if (!impl_) throw std::bad_alloc{};
    }

    ~FileBuilder() {
        if (impl_) ::gtd_builder_destroy(impl_);
    }

    FileBuilder(const FileBuilder&) = delete;
    FileBuilder& operator=(const FileBuilder&) = delete;

    FileBuilder(FileBuilder&& other) noexcept : impl_(other.impl_) {
        other.impl_ = nullptr;
    }

    FileBuilder& operator=(FileBuilder&& other) noexcept {
        if (this != &other) {
            if (impl_) ::gtd_builder_destroy(impl_);
            impl_ = other.impl_;
            other.impl_ = nullptr;
        }
        return *this;
    }

    /** @name Metadata setters (must be called before the first `add_*` call). */
    ///@{

    FileBuilder& title(std::string v) {
        detail::check(::gtd_builder_set_title(impl_, v.c_str()));
        return *this;
    }

    FileBuilder& device(std::string v) {
        detail::check(::gtd_builder_set_device(impl_, v.c_str()));
        return *this;
    }

    FileBuilder& notes(std::string v) {
        detail::check(::gtd_builder_set_notes(impl_, v.c_str()));
        return *this;
    }

    FileBuilder& identity(std::string v) {
        detail::check(::gtd_builder_set_identity(impl_, v.c_str()));
        return *this;
    }

    /** Downgrade out-of-range annotation errors to warnings. */
    FileBuilder& lenient() noexcept {
        ::gtd_builder_set_lenient(impl_);
        return *this;
    }

    ///@}

    /** @name Data ingestion */
    ///@{

    FileBuilder& add_nav_fix(NavFix fix) {
        GtdOptF64 heading = fix.heading
            ? GTD_SOME_F64(fix.heading->as_degrees()) : GTD_NONE_F64;
        GtdOptF64 speed = fix.speed
            ? GTD_SOME_F64(fix.speed->as_mps()) : GTD_NONE_F64;
        detail::check(::gtd_builder_add_nav_fix(
            impl_,
            detail::to_c(fix.gps_time),
            detail::to_c(fix.sys_time),
            fix.lat.as_degrees(),
            fix.lon.as_degrees(),
            heading,
            speed,
            detail::to_c(fix.eph_m)
        ));
        return *this;
    }

    FileBuilder& add_satellite_report(SatelliteReport report) {
        std::vector<GtdSatellite> sats;
        sats.reserve(report.tracked.size());
        for (const auto& s : report.tracked) {
            sats.push_back(GtdSatellite{
                detail::to_c(s.constellation),
                s.prn,
                static_cast<std::uint8_t>(s.in_fix ? 1 : 0),
                detail::to_c(s.elevation_deg),
                detail::to_c(s.azimuth_deg),
                detail::to_c(s.snr_dbhz),
            });
        }
        detail::check(::gtd_builder_add_satellite_report(
            impl_,
            detail::to_c(report.gps_time),
            detail::to_c(report.sys_time),
            sats.data(),
            sats.size()
        ));
        return *this;
    }

    FileBuilder& add_annotation(Annotation ann) {
        const char* label = ann.label.empty() ? nullptr : ann.label.c_str();
        detail::check(::gtd_builder_add_annotation(
            impl_,
            detail::to_c(ann.time),
            label,
            detail::to_c(ann.icon)
        ));
        return *this;
    }

    /**
     * Add a structured event marker.
     * @throws InvalidPathError if `variant_path` is malformed.
     */
    FileBuilder& add_event_marker(EventMarker marker) {
        const char* ann = marker.annotation.empty() ? nullptr : marker.annotation.c_str();
        detail::check(::gtd_builder_add_event_marker(
            impl_,
            marker.variant_path.c_str(),
            detail::to_c(marker.sys_time),
            ann
        ));
        return *this;
    }

    FileBuilder& add_event_marker_style(EventMarkerStyle style) {
        const char* color = style.color_hex.empty() ? nullptr : style.color_hex.c_str();
        detail::check(::gtd_builder_add_event_marker_style(
            impl_,
            style.variant_path.c_str(),
            detail::to_c(style.icon),
            color
        ));
        return *this;
    }

    ///@}

    /**
     * Finalise the builder and return a `NavFile`.
     *
     * The builder is **consumed** by this call regardless of success or failure.
     * Do not use the builder after calling `finish()`.
     *
     * @throws NoNavFixesError if no nav fixes were added.
     * @throws AnnotationsOutOfRangeError if annotations fall outside the time range.
     */
    NavFile finish();

private:
    GtdFileBuilder* impl_;
};


/**
 * A parsed or newly-built GeoTrace navigation file.
 *
 * **Non-copyable; movable.**
 */
class NavFile {
public:
    ~NavFile() {
        if (impl_) ::gtd_nav_file_destroy(impl_);
    }

    NavFile(const NavFile&) = delete;
    NavFile& operator=(const NavFile&) = delete;

    NavFile(NavFile&& other) noexcept : impl_(other.impl_) {
        other.impl_ = nullptr;
    }

    NavFile& operator=(NavFile&& other) noexcept {
        if (this != &other) {
            if (impl_) ::gtd_nav_file_destroy(impl_);
            impl_ = other.impl_;
            other.impl_ = nullptr;
        }
        return *this;
    }

    /**
     * Open and parse a `.gtd` file.
     * @throws IoError, UnsupportedVersionError, Hdf5Error on failure.
     */
    static NavFile open(const std::filesystem::path& p) {
        GtdNavFile* out = nullptr;
        detail::check(::gtd_nav_file_open(p.c_str(), &out));
        return NavFile(out);
    }

    /**
     * Parse a `.gtd` file from a memory buffer.
     * @throws IoError, UnsupportedVersionError, Hdf5Error on failure.
     */
    static NavFile from_bytes(span<const std::uint8_t> data) {
        GtdNavFile* out = nullptr;
        detail::check(::gtd_nav_file_from_bytes(data.data(), data.size(), &out));
        return NavFile(out);
    }

    /** Convenience overload for `std::vector<uint8_t>`. */
    static NavFile from_bytes(const std::vector<std::uint8_t>& data) {
        return from_bytes(span<const std::uint8_t>{data});
    }

    /**
     * Write the file to disk.
     * The `.gtd` extension is appended if the path has no extension.
     * @throws IoError, Hdf5Error on failure.
     */
    void write_to_file(const std::filesystem::path& p) const {
        detail::check(::gtd_nav_file_write_to_path(impl_, p.c_str()));
    }

    /**
     * Serialise to a byte vector.
     * @throws IoError, Hdf5Error on failure.
     */
    std::vector<std::uint8_t> to_bytes() const {
        std::uint8_t* buf = nullptr;
        std::size_t   len = 0;
        detail::check(::gtd_nav_file_to_bytes(impl_, &buf, &len));
        std::vector<std::uint8_t> result(buf, buf + len);
        ::gtd_free_bytes(buf, len);
        return result;
    }

    /** @name Metadata (returns empty string_view when field is absent). */
    ///@{

    std::string_view title()    const noexcept {
        const char* s = ::gtd_nav_file_title(impl_);
        return s ? std::string_view{s} : std::string_view{};
    }
    std::string_view device()   const noexcept {
        const char* s = ::gtd_nav_file_device(impl_);
        return s ? std::string_view{s} : std::string_view{};
    }
    std::string_view notes()    const noexcept {
        const char* s = ::gtd_nav_file_notes(impl_);
        return s ? std::string_view{s} : std::string_view{};
    }
    std::string_view identity() const noexcept {
        const char* s = ::gtd_nav_file_identity(impl_);
        return s ? std::string_view{s} : std::string_view{};
    }

    ///@}

    /** Number of navigation fixes in the file. */
    std::size_t nav_point_count() const noexcept {
        return ::gtd_nav_file_nav_point_count(impl_);
    }

    /**
     * Return the navigation fix at @p idx.
     * @throws std::out_of_range if `idx >= nav_point_count()`.
     */
    NavPointView nav_point(std::size_t idx) const {
        GtdNavPointInfo info{};
        detail::check_range(::gtd_nav_file_get_nav_point(impl_, idx, &info));

        NavPointView v;
        v.gps_time        = detail::from_c(info.gps_time);
        v.sys_time        = detail::from_c(info.sys_time);
        v.lat             = Angle::degrees(info.lat_deg);
        v.lon             = Angle::degrees(info.lon_deg);
        v.heading         = info.heading_deg.present
            ? std::optional<Angle>{Angle::degrees(info.heading_deg.value)} : std::nullopt;
        v.speed           = info.speed_mps.present
            ? std::optional<Velocity>{Velocity::mps(info.speed_mps.value)} : std::nullopt;
        v.eph_m           = info.eph_m.present
            ? std::optional<double>{info.eph_m.value} : std::nullopt;
        v.satellite_count = info.sat_count;
        return v;
    }

    /**
     * Return satellite data for a specific tracked satellite.
     * @throws std::out_of_range if either index is out of range or the fix has no satellite report.
     */
    SatelliteView satellite(std::size_t nav_idx, std::size_t sat_idx) const {
        GtdSatInfo info{};
        detail::check_range(::gtd_nav_file_get_satellite(impl_, nav_idx, sat_idx, &info));

        SatelliteView v;
        v.constellation = detail::from_c(info.constellation);
        v.prn           = info.prn;
        v.in_fix        = info.in_fix != 0;
        v.elevation_deg = info.elevation_deg.present
            ? std::optional<double>{info.elevation_deg.value} : std::nullopt;
        v.azimuth_deg   = info.azimuth_deg.present
            ? std::optional<double>{info.azimuth_deg.value} : std::nullopt;
        v.snr_dbhz      = info.snr_dbhz.present
            ? std::optional<double>{info.snr_dbhz.value} : std::nullopt;
        return v;
    }

    /** Number of event markers in the file. */
    std::size_t event_marker_count() const noexcept {
        return ::gtd_nav_file_event_marker_count(impl_);
    }

    /**
     * Return the event marker at @p idx.
     * @throws std::out_of_range if `idx >= event_marker_count()`.
     */
    EventMarkerView event_marker(std::size_t idx) const {
        GtdEventMarkerInfo info{};
        detail::check_range(::gtd_nav_file_get_event_marker(impl_, idx, &info));

        EventMarkerView v;
        v.variant_path = info.variant_path;
        v.sys_time     = detail::from_c(info.sys_time);
        v.lat          = Angle::degrees(info.lat_deg);
        v.lon          = Angle::degrees(info.lon_deg);
        v.annotation   = info.has_annotation ? std::string{info.annotation} : std::string{};
        return v;
    }

private:
    friend class FileBuilder;
    explicit NavFile(GtdNavFile* impl) noexcept : impl_(impl) {}
    GtdNavFile* impl_;
};


inline NavFile FileBuilder::finish() {
    GtdNavFile* out = nullptr;
    GtdStatus s = ::gtd_builder_finish(impl_, &out);
    impl_ = nullptr;  // builder is consumed regardless of success or failure
    detail::check(s);
    return NavFile(out);
}

} // namespace geotrace
