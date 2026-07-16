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
 *
 * Errors are reported two ways: the default methods throw exceptions derived
 * from `geotrace::Error`. The parallel `try_*` methods instead return a
 * `Result`/`Status` by value and never throw. When the SDK is compiled without
 * exception support (`-fno-exceptions`, or `GEOTRACE_CPP_NO_EXCEPTIONS`), use
 * the `try_*` API.
 */

#pragma once

#if __has_include(<version>)
#include <version>
#endif
#if __has_include(<compare>)
#include <compare>
#endif
#include <cassert>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>

#include <geotrace.h> // C SDK (already has extern "C" guards)
#include <geotrace/unit_catalog.hpp>

#define GEOTRACE_CPP_VERSION       "0.5.0"
#define GEOTRACE_CPP_VERSION_MAJOR 0
#define GEOTRACE_CPP_VERSION_MINOR 5
#define GEOTRACE_CPP_VERSION_PATCH 0

// Exception support is detected the idiomatic way, via the standard
// `__cpp_exceptions` feature-test macro (MSVC predates it, so `_CPPUNWIND` is
// also accepted). Define GEOTRACE_CPP_NO_EXCEPTIONS to force the no-exceptions
// path even when the compiler supports them. The throwing API is the default.
// When exceptions are unavailable, use the `try_*` methods and `Result`/`Status`
// instead, which report errors by value and never throw.
#if !defined(GEOTRACE_CPP_NO_EXCEPTIONS) && (defined(__cpp_exceptions) || defined(_CPPUNWIND))
#define GEOTRACE_CPP_EXCEPTIONS 1
#else
#define GEOTRACE_CPP_EXCEPTIONS 0
#endif

// Minimal std::span polyfill for C++17.  Replaced by std::span on C++20.
#if defined(__cpp_lib_span) && __cpp_lib_span >= 202002L
#include <span>
namespace geotrace {
template <typename T> using span = std::span<T>;
}
#else
namespace geotrace {

template <typename T> class span {
  public:
    using element_type = T;
    using value_type = std::remove_cv_t<T>;
    using size_type = std::size_t;
    using pointer = T *;
    using iterator = pointer;

    constexpr span() noexcept = default;
    constexpr span(pointer ptr, size_type count) noexcept : data_(ptr), size_(count) {}

    template <std::size_t N> constexpr span(T (&arr)[N]) noexcept : data_(arr), size_(N) {}

    template <typename Container,
              typename = std::enable_if_t<
                  !std::is_same_v<std::decay_t<Container>, span> &&
                  std::is_convertible_v<decltype(std::declval<Container &>().data()), pointer>>>
    constexpr span(Container &c) noexcept : data_(c.data()), size_(c.size()) {}

    template <
        typename Container,
        typename = std::enable_if_t<
            !std::is_same_v<std::decay_t<Container>, span> &&
            std::is_convertible_v<decltype(std::declval<const Container &>().data()), const T *>>>
    constexpr span(const Container &c) noexcept : data_(c.data()), size_(c.size()) {}

    constexpr pointer data() const noexcept { return data_; }
    constexpr size_type size() const noexcept { return size_; }
    constexpr bool empty() const noexcept { return size_ == 0; }
    constexpr iterator begin() const noexcept { return data_; }
    constexpr iterator end() const noexcept { return data_ + size_; }
    constexpr T &operator[](size_type i) const noexcept { return data_[i]; }

  private:
    pointer data_ = nullptr;
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
struct BuildError : Error {
    using Error::Error;
};

/** `finish()` was called without adding any nav fixes. */
struct NoNavFixesError : BuildError {
    using BuildError::BuildError;
};

/** One or more annotations fall outside the nav fix time range. */
struct AnnotationsOutOfRangeError : BuildError {
    std::size_t count;
    AnnotationsOutOfRangeError(std::size_t n, const std::string &msg) : BuildError(msg), count(n) {}
};

/** I/O error (file not found, permission denied, etc.). */
struct IoError : Error {
    using Error::Error;
};

/** HDF5 library error. */
struct Hdf5Error : Error {
    using Error::Error;
};

/** File was written by a newer, incompatible SDK version. */
struct UnsupportedVersionError : Error {
    using Error::Error;
};

/** A variant path argument was malformed. */
struct InvalidPathError : Error {
    using Error::Error;
};

/** Malformed or corrupt `.gtd` file content (decode failed). */
struct ParseError : Error {
    using Error::Error;
};

/**
 * A channel was malformed (bad name/component, length mismatch, or duplicate
 * name). Derives `Error` rather than `BuildError`, mirroring `InvalidPathError`:
 * both are input-validation errors, even though the duplicate-name check fires
 * at `finish()`.
 */
struct InvalidChannelError : Error {
    using Error::Error;
};

namespace detail {

[[noreturn]] inline void abort_with(const std::string &msg) {
    const std::string line = "geotrace: " + msg + "\n";
    static_cast<void>(std::fputs(line.c_str(), stderr));
    std::abort();
}

// Throw the typed exception for a status. Without exception support this is only
// reached by the throwing API (the `try_*` / Result API never calls it), so it
// prints and aborts as a last resort.
[[noreturn]] inline void throw_typed(GtdStatus s, const std::string &msg) {
#if GEOTRACE_CPP_EXCEPTIONS
    switch (s) {
    case GTD_ERR_NULL_ARGUMENT:
        // The C++ wrapper never passes null pointers, so this only arises from
        // an out-of-range accessor index.
        throw std::out_of_range(msg);
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
    case GTD_ERR_PARSE:
        throw ParseError(msg);
    case GTD_ERR_INVALID_CHANNEL:
        throw InvalidChannelError(msg);
    default:
        throw Error(msg);
    }
#else
    (void)s;
    abort_with(msg);
#endif
}

// Encode a filesystem path as UTF-8 for the C API.
inline std::string path_string(const std::filesystem::path &p) {
#if defined(__cpp_lib_char8_t)
    const auto u8 = p.u8string();
    return std::string(u8.begin(), u8.end());
#else
    return p.u8string();
#endif
}

inline GtdOptF64 to_c(std::optional<double> v) noexcept {
    // Plain aggregate construction rather than GTD_SOME_F64/GTD_NONE_F64: those
    // macros expand to C99 compound-literal + designated-initializer syntax,
    // which MSVC rejects in C++ mode (errors C4576/C7555).
    GtdOptF64 result{};
    if (v) {
        result.value = *v;
        result.present = 1;
    }
    return result;
}

struct BuilderDeleter {
    void operator()(GtdFileBuilder *p) const noexcept { ::gtd_builder_destroy(p); }
};

struct NavFileDeleter {
    void operator()(GtdNavFile *p) const noexcept { ::gtd_nav_file_destroy(p); }
};

} // namespace detail

/**
 * An error returned by value from a `try_*` method, instead of thrown.
 *
 * `code` is the underlying `GtdStatus`. `description` is a human-readable
 * message. This is the non-throwing error channel: check `is_ok()` or call
 * `value_or_throw()` on the enclosing `Result`.
 */
struct Status {
    GtdStatus code = GTD_OK;
    std::string description;

    Status() = default;
    Status(GtdStatus c, std::string d) : code(c), description(std::move(d)) {}

    /// Build a `Status` from a `GtdStatus`, capturing the thread-local message.
    static Status from(GtdStatus s) {
        if (s == GTD_OK)
            return Status{};
        const char *raw = ::gtd_last_error();
        return Status{s, (raw != nullptr) ? raw : "unknown error"};
    }

    bool is_ok() const noexcept { return code == GTD_OK; }
    bool is_err() const noexcept { return code != GTD_OK; }
    explicit operator bool() const noexcept { return is_ok(); }

    /// Throw the matching exception on failure (no-op on success). With
    /// exceptions disabled this prints and aborts, so prefer `is_ok()` there.
    void throw_on_failure() const {
        if (is_err())
            detail::throw_typed(code, description);
    }
};

/**
 * The result of a fallible operation: either a value or a `Status` error.
 *
 * Modelled on Rust's `Result`. Inspect `is_ok()` / `error()` and call `value()`,
 * or call `value_or_throw()` to throw the error (or abort without exceptions).
 */
template <typename T> struct Result {
    Result(T v) : value_(std::move(v)) {}
    // An error result must carry a real error: an ok status here would falsely
    // report success with a default-constructed value.
    Result(Status s) : status_(std::move(s)) { assert(status_.is_err()); }
    Result() = delete;

    bool is_ok() const noexcept { return status_.is_ok(); }
    bool is_err() const noexcept { return status_.is_err(); }
    explicit operator bool() const noexcept { return is_ok(); }
    const Status &error() const noexcept { return status_; }

    const T *get_if() const noexcept { return value_ ? &*value_ : nullptr; }
    T *get_if() noexcept { return value_ ? &*value_ : nullptr; }

    const T &value() const & {
        status_.throw_on_failure();
        assert(value_.has_value());
        return *value_;
    }
    T &value() & {
        status_.throw_on_failure();
        assert(value_.has_value());
        return *value_;
    }

    const T &value_or_throw() const & { return value(); }
    T value_or_throw() && {
        status_.throw_on_failure();
        assert(value_.has_value());
        return std::move(*value_);
    }

  private:
    std::optional<T> value_;
    Status status_;
};

/**
 * UTC Unix epoch timestamp in microseconds.
 *
 * Use `Timestamp::none()` to represent an absent value.
 */
struct Timestamp {
    std::int64_t unix_micros = 0;

    static constexpr std::int64_t kNoneVal = -9223372036854775807LL - 1;

    static constexpr Timestamp none() noexcept { return Timestamp{kNoneVal}; }
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

    bool is_none() const noexcept { return unix_micros == kNoneVal; }

#if defined(__cpp_impl_three_way_comparison) && __cpp_impl_three_way_comparison >= 201907L
    auto operator<=>(const Timestamp &) const = default;
#else
    bool operator==(Timestamp other) const noexcept { return unix_micros == other.unix_micros; }
    bool operator!=(Timestamp other) const noexcept { return !(*this == other); }
    bool operator<(Timestamp other) const noexcept { return unix_micros < other.unix_micros; }
    bool operator<=(Timestamp other) const noexcept { return unix_micros <= other.unix_micros; }
    bool operator>(Timestamp other) const noexcept { return unix_micros > other.unix_micros; }
    bool operator>=(Timestamp other) const noexcept { return unix_micros >= other.unix_micros; }
#endif
};

/** Angular measurement stored in degrees. */
class Angle {
  public:
    static Angle degrees(double deg) noexcept { return Angle{deg}; }
    static Angle radians(double rad) noexcept { return Angle{rad * (180.0 / kPi)}; }

    double as_degrees() const noexcept { return deg_; }
    double as_radians() const noexcept { return deg_ * (kPi / 180.0); }

#if defined(__cpp_impl_three_way_comparison) && __cpp_impl_three_way_comparison >= 201907L
    auto operator<=>(const Angle &) const = default;
#else
    bool operator==(Angle other) const noexcept { return deg_ == other.deg_; }
    bool operator!=(Angle other) const noexcept { return !(*this == other); }
    bool operator<(Angle other) const noexcept { return deg_ < other.deg_; }
    bool operator<=(Angle other) const noexcept { return deg_ <= other.deg_; }
    bool operator>(Angle other) const noexcept { return deg_ > other.deg_; }
    bool operator>=(Angle other) const noexcept { return deg_ >= other.deg_; }
#endif

  private:
    // M_PI is a POSIX extension not guaranteed by the C++ standard (absent on MSVC
    // without _USE_MATH_DEFINES), so we use our own constant instead.
    static constexpr double kPi = 3.141592653589793238462643383279502884;
    explicit Angle(double deg) noexcept : deg_(deg) {}
    double deg_ = 0.0;
};

/** Velocity stored in metres per second. */
class Velocity {
  public:
    // Conversion factors kept bit-identical to the Rust SDK (units.rs
    // MPS_PER_KMH / MPS_PER_KNOT) so the same input yields the same stored m/s
    // across SDKs. Use the constant-multiply form (`v * (1.0/3.6)`), not
    // `v / 3.6`, which differs by 1 ULP for some values.
    static constexpr double kMpsPerKmh = 1.0 / 3.6;
    static constexpr double kMpsPerKnot = 1852.0 / 3600.0;

    static Velocity mps(double v) noexcept { return Velocity{v}; }
    static Velocity kmh(double v) noexcept { return Velocity{v * kMpsPerKmh}; }
    static Velocity knots(double v) noexcept { return Velocity{v * kMpsPerKnot}; }

    double as_mps() const noexcept { return mps_; }
    double as_kmh() const noexcept { return mps_ / kMpsPerKmh; }
    double as_knots() const noexcept { return mps_ / kMpsPerKnot; }

#if defined(__cpp_impl_three_way_comparison) && __cpp_impl_three_way_comparison >= 201907L
    auto operator<=>(const Velocity &) const = default;
#else
    bool operator==(Velocity other) const noexcept { return mps_ == other.mps_; }
    bool operator!=(Velocity other) const noexcept { return !(*this == other); }
    bool operator<(Velocity other) const noexcept { return mps_ < other.mps_; }
    bool operator<=(Velocity other) const noexcept { return mps_ <= other.mps_; }
    bool operator>(Velocity other) const noexcept { return mps_ > other.mps_; }
    bool operator>=(Velocity other) const noexcept { return mps_ >= other.mps_; }
#endif

  private:
    explicit Velocity(double mps) noexcept : mps_(mps) {}
    double mps_ = 0.0;
};

enum class Constellation : std::uint8_t { Gps, Glonass, Galileo, Beidou, Navic, Qzss };

enum class MarkerIcon : std::uint8_t {
    Pin,
    Cross,
    Circle,
    Lightning,
    Warning,
    Error,
    Check,
    Satellite,
    SatelliteLost,
    Gear,
    Refresh,
    Download,
    Upload,
    Wrench,
    Auto,
};

/** Platform a recording was made on, declared by the recorder. */
enum class TravelMode : std::uint8_t {
    Car,
    Motorcycle,
    Bicycle,
    Pedestrian,
    Boat,
    Rail,
    Aircraft,
};

namespace detail {

inline GtdConstellation to_c(Constellation c) noexcept {
    switch (c) {
    case Constellation::Gps:
        return GTD_CONSTELLATION_GPS;
    case Constellation::Glonass:
        return GTD_CONSTELLATION_GLONASS;
    case Constellation::Galileo:
        return GTD_CONSTELLATION_GALILEO;
    case Constellation::Beidou:
        return GTD_CONSTELLATION_BEIDOU;
    case Constellation::Navic:
        return GTD_CONSTELLATION_NAVIC;
    case Constellation::Qzss:
        return GTD_CONSTELLATION_QZSS;
    }
    return GTD_CONSTELLATION_GPS;
}

inline Constellation from_c(GtdConstellation c) noexcept {
    switch (c) {
    case GTD_CONSTELLATION_GPS:
        return Constellation::Gps;
    case GTD_CONSTELLATION_GLONASS:
        return Constellation::Glonass;
    case GTD_CONSTELLATION_GALILEO:
        return Constellation::Galileo;
    case GTD_CONSTELLATION_BEIDOU:
        return Constellation::Beidou;
    case GTD_CONSTELLATION_NAVIC:
        return Constellation::Navic;
    case GTD_CONSTELLATION_QZSS:
        return Constellation::Qzss;
    }
    return Constellation::Gps;
}

inline GtdMarkerIcon to_c(MarkerIcon icon) noexcept {
    switch (icon) {
    case MarkerIcon::Pin:
        return GTD_ICON_PIN;
    case MarkerIcon::Cross:
        return GTD_ICON_CROSS;
    case MarkerIcon::Circle:
        return GTD_ICON_CIRCLE;
    case MarkerIcon::Lightning:
        return GTD_ICON_LIGHTNING;
    case MarkerIcon::Warning:
        return GTD_ICON_WARNING;
    case MarkerIcon::Error:
        return GTD_ICON_ERROR;
    case MarkerIcon::Check:
        return GTD_ICON_CHECK;
    case MarkerIcon::Satellite:
        return GTD_ICON_SATELLITE;
    case MarkerIcon::SatelliteLost:
        return GTD_ICON_SATELLITE_LOST;
    case MarkerIcon::Gear:
        return GTD_ICON_GEAR;
    case MarkerIcon::Refresh:
        return GTD_ICON_REFRESH;
    case MarkerIcon::Download:
        return GTD_ICON_DOWNLOAD;
    case MarkerIcon::Upload:
        return GTD_ICON_UPLOAD;
    case MarkerIcon::Wrench:
        return GTD_ICON_WRENCH;
    case MarkerIcon::Auto:
        return GTD_ICON_AUTO;
    }
    return GTD_ICON_AUTO;
}

inline GtdTravelMode to_c(TravelMode mode) noexcept {
    switch (mode) {
    case TravelMode::Car:
        return GTD_TRAVEL_MODE_CAR;
    case TravelMode::Motorcycle:
        return GTD_TRAVEL_MODE_MOTORCYCLE;
    case TravelMode::Bicycle:
        return GTD_TRAVEL_MODE_BICYCLE;
    case TravelMode::Pedestrian:
        return GTD_TRAVEL_MODE_PEDESTRIAN;
    case TravelMode::Boat:
        return GTD_TRAVEL_MODE_BOAT;
    case TravelMode::Rail:
        return GTD_TRAVEL_MODE_RAIL;
    case TravelMode::Aircraft:
        return GTD_TRAVEL_MODE_AIRCRAFT;
    }
    return GTD_TRAVEL_MODE_CAR;
}

inline TravelMode from_c(GtdTravelMode mode) noexcept {
    switch (mode) {
    case GTD_TRAVEL_MODE_CAR:
        return TravelMode::Car;
    case GTD_TRAVEL_MODE_MOTORCYCLE:
        return TravelMode::Motorcycle;
    case GTD_TRAVEL_MODE_BICYCLE:
        return TravelMode::Bicycle;
    case GTD_TRAVEL_MODE_PEDESTRIAN:
        return TravelMode::Pedestrian;
    case GTD_TRAVEL_MODE_BOAT:
        return TravelMode::Boat;
    case GTD_TRAVEL_MODE_RAIL:
        return TravelMode::Rail;
    case GTD_TRAVEL_MODE_AIRCRAFT:
        return TravelMode::Aircraft;
    }
    return TravelMode::Car;
}

inline GtdTimestamp to_c(Timestamp ts) noexcept {
    return GtdTimestamp{ts.unix_micros};
}

inline Timestamp from_c(GtdTimestamp ts) noexcept {
    return Timestamp{ts.unix_micros};
}

} // namespace detail

/** Wire name of @p mode, e.g. `"car"` for `TravelMode::Car`. */
inline std::string_view travel_mode_name(TravelMode mode) noexcept {
    return std::string_view{::gtd_travel_mode_name(detail::to_c(mode))};
}

/**
 * Parse a wire name (as produced by travel_mode_name() or read from
 * NavFile::travel_mode()) back into a travel mode.
 *
 * Returns `std::nullopt` for names outside the known set.
 */
inline std::optional<TravelMode> travel_mode_from_name(const std::string &name) noexcept {
    GtdTravelMode mode{};
    if (::gtd_travel_mode_from_name(name.c_str(), &mode) != GTD_OK)
        return std::nullopt;
    return detail::from_c(mode);
}

/** A single GPS navigation fix. */
struct NavFix {
    Timestamp gps_time = Timestamp::none();
    Timestamp sys_time = Timestamp::none();
    Angle lat = Angle::degrees(0.0);
    Angle lon = Angle::degrees(0.0);
    std::optional<Angle> heading;
    std::optional<Velocity> speed;
    std::optional<double> eph_m;
};

/** One satellite in a visibility report. */
struct Satellite {
    Constellation constellation = Constellation::Gps;
    std::uint32_t prn = 0;
    bool in_fix = false;
    std::optional<double> elevation_deg;
    std::optional<double> azimuth_deg;
    std::optional<double> snr_dbhz;
};

/** A snapshot of satellite visibility at a point in time. */
struct SatelliteReport {
    Timestamp gps_time = Timestamp::none();
    Timestamp sys_time = Timestamp::none();
    std::vector<Satellite> tracked;
};

/** A legacy map-pin annotation. */
struct Annotation {
    Timestamp time;
    std::string label;
    MarkerIcon icon = MarkerIcon::Auto;
};

/** A structured event marker placed on the map. */
struct EventMarker {
    std::string variant_path;
    Timestamp sys_time;
    std::string annotation; // empty = none
};

/** Display style for all events of a given variant. */
struct EventMarkerStyle {
    std::string variant_path;
    MarkerIcon icon = MarkerIcon::Auto;
    std::string color_hex; // empty = auto (hash-derived). format: "#RRGGBB"
};

/**
 * A scalar or vector sensor channel sampled at its own rate.
 *
 * Leave `components` empty for a scalar channel, or list one label per column
 * for a vector channel (an accelerometer's `{"x", "y", "z"}`). `values` is
 * row-major: `times.size()` rows of one column (scalar) or `components.size()`
 * columns (vector).
 */
class ChannelUnit {
  public:
    static ChannelUnit recognized(RecognizedUnit unit) {
        return ChannelUnit{recognized_unit_label(unit), false};
    }

    static ChannelUnit custom(std::string label) {
        return try_custom(std::move(label)).value_or_throw();
    }

    static Result<ChannelUnit> try_custom(std::string label) {
        return try_parse(std::move(label), GTD_CHANNEL_UNIT_CUSTOM, true);
    }

    static ChannelUnit parse_recognized(std::string_view label) {
        return try_parse_recognized(label).value_or_throw();
    }

    static Result<ChannelUnit> try_parse_recognized(std::string_view label) {
        return try_parse(std::string{label}, GTD_CHANNEL_UNIT_RECOGNIZED, false);
    }

    const std::string &label() const noexcept { return label_; }
    bool is_custom() const noexcept { return custom_; }

    friend bool operator==(const ChannelUnit &a, const ChannelUnit &b) noexcept {
        return a.custom_ == b.custom_ && a.label_ == b.label_;
    }

  private:
    friend class NavFile;

    ChannelUnit(std::string label, bool custom) : label_{std::move(label)}, custom_{custom} {}

    static ChannelUnit from_file_label(std::string label, bool custom) {
        return custom ? ChannelUnit{std::move(label), true} : parse_recognized(label);
    }

    static Result<ChannelUnit> try_parse(std::string label, GtdChannelUnitMode mode, bool custom) {
        std::size_t required = 0;
        GtdStatus status = ::gtd_channel_unit_parse(label.c_str(), static_cast<std::uint32_t>(mode),
                                                    nullptr, 0, &required);
        if (status != GTD_OK)
            return Status::from(status);
        std::vector<char> canonical(required);
        status = ::gtd_channel_unit_parse(label.c_str(), static_cast<std::uint32_t>(mode),
                                          canonical.data(), canonical.size(), &required);
        if (status != GTD_OK)
            return Status::from(status);
        return ChannelUnit{std::string{canonical.data()}, custom};
    }

    std::string label_;
    bool custom_;
};

struct Channel {
    std::string name;
    std::optional<ChannelUnit> unit;
    std::optional<Angle> period;         // wrap period, none = linear
    std::string description;             // empty = none
    std::vector<std::string> components; // empty = scalar channel
    std::vector<Timestamp> times;
    std::vector<double> values;
};

/** Channel data returned by `NavFile::channel()`. String fields are copies. */
struct ChannelView {
    std::string name;
    std::optional<ChannelUnit> unit;
    std::optional<Angle> period;         // none = linear
    std::string description;             // empty = none
    std::vector<std::string> components; // empty = scalar channel
    std::vector<Timestamp> times;
    std::vector<double> values; // row-major, times.size() * max(components.size(), 1)

    /** Whether this is a vector channel (has named components). */
    bool is_vector() const noexcept { return !components.empty(); }
};

/** Data for one navigation fix, returned by `NavFile::nav_point()`. */
struct NavPointView {
    Timestamp gps_time;
    Timestamp sys_time;
    Angle lat = Angle::degrees(0.0);
    Angle lon = Angle::degrees(0.0);
    std::optional<Angle> heading;
    std::optional<Velocity> speed;
    std::optional<double> eph_m;
    std::size_t satellite_count = 0;
};

/** Satellite data for one tracked satellite, returned by `NavFile::satellite()`. */
struct SatelliteView {
    Constellation constellation = Constellation::Gps;
    std::uint32_t prn = 0;
    bool in_fix = false;
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
    Timestamp sys_time;
    Angle lat = Angle::degrees(0.0);
    Angle lon = Angle::degrees(0.0);
    std::string annotation; // empty if none
};

/**
 * @name Type-safe event kinds
 *
 * Model an event taxonomy as `enum class` levels and specialise `EventEnum<E>`
 * to give each level a path segment.  `event_path()` composes a slash-separated
 * `variant_path` at the call site, and `FileBuilder::add_event()` accepts the
 * enum values directly - the compiler rejects anything that is not a known
 * event enum.
 *
 * @code
 * enum class Power { Boot, Sleep, BatteryLow };
 * template <> struct geotrace::EventEnum<Power> {
 *     static constexpr std::string_view base = "power";
 *     static constexpr std::string_view seg(Power p) {
 *         switch (p) {
 *         case Power::Boot:       return "boot";
 *         case Power::Sleep:      return "sleep";
 *         case Power::BatteryLow: return "battery_low";
 *         }
 *         return "";
 *     }
 * };
 *
 * builder.add_event(Power::Boot, ts, "cold start");          // "power/boot"
 * builder.add_event(event_path(Connectivity::Agps, Agps::Request), ts);
 * @endcode
 * @{
 */

/**
 * Trait describing one level of an event taxonomy.
 *
 * Specialise for each `enum class` level with two members:
 *  - `static constexpr std::string_view base` - the segment naming this level.
 *  - `static constexpr std::string_view seg(E)` - the leaf segment per value.
 *
 * `seg()` must return a non-empty segment for **every** enumerator. Write it as
 * a `switch` with no `default` so that adding an enumerator without a matching
 * case is a compile error under `-Wswitch`/`-Werror` (the SDK examples and
 * tests build that way). A value that falls through would compose a malformed
 * path such as `"power/"`, surfacing only as a runtime `InvalidPathError`.
 *
 * The primary template is left undefined so that only specialised enums are
 * accepted by `event_path()` and `FileBuilder::add_event()`.
 */
template <class E> struct EventEnum;

namespace detail {
template <class E, class = void> struct is_event_enum : std::false_type {};
template <class E>
struct is_event_enum<
    E, std::void_t<decltype(EventEnum<E>::base), decltype(EventEnum<E>::seg(std::declval<E>()))>>
    : std::true_type {};
} // namespace detail

#if defined(__cpp_concepts) && __cpp_concepts >= 201907L
/**
 * C++20 Concept for a fully-formed EventEnum specialisation.
 *
 * Constrains `event_path()` and `FileBuilder::add_event()` on C++20 builds,
 * giving a clear "constraint not satisfied" diagnostic when a type lacks a
 * proper EventEnum<> specialisation, rather than the cryptic substitution
 * failure that the C++17 SFINAE path produces.
 */
template <typename E>
concept EventEnumValue = requires(E val) {
    { EventEnum<E>::base } -> std::convertible_to<std::string_view>;
    { EventEnum<E>::seg(val) } -> std::convertible_to<std::string_view>;
};
#endif

/**
 * A composed, slash-separated event `variant_path`.
 *
 * Produced by `event_path()`. Use `str()` for the owned string (e.g. to set
 * `EventMarkerStyle::variant_path`). The `std::string_view` conversion is
 * `explicit` so the owning temporary can't silently dangle behind a view.
 */
class EventPath {
  public:
    explicit EventPath(std::string path) noexcept : path_(std::move(path)) {}

    const std::string &str() const noexcept { return path_; }
    explicit operator std::string_view() const noexcept { return path_; }

  private:
    std::string path_;
};

namespace detail {
template <class E> void append_event_seg(std::string &out, E v, bool with_base) {
    if (with_base)
        out += EventEnum<E>::base;
    out += '/';
    out += EventEnum<E>::seg(v);
}
} // namespace detail

/**
 * Compose an event path from one or more taxonomy levels.
 *
 * The first value contributes `base + "/" + seg`, each further value appends
 * `"/" + seg`, so `event_path(Connectivity::Agps, Agps::Request)` yields
 * `"connectivity/agps/request"`.
 */
#if defined(__cpp_concepts) && __cpp_concepts >= 201907L
template <EventEnumValue E, EventEnumValue... Es> EventPath event_path(E v0, Es... vs) {
#else
template <class E, class... Es> EventPath event_path(E v0, Es... vs) {
    static_assert(detail::is_event_enum<E>::value,
                  "event_path: no EventEnum<> specialisation for this type");
    static_assert((detail::is_event_enum<Es>::value && ...),
                  "event_path: no EventEnum<> specialisation for a nested type");
#endif
    std::string out;
    detail::append_event_seg(out, v0, true);
    (detail::append_event_seg(out, vs, false), ...);
    return EventPath{std::move(out)};
}

/** @} */

/**
 * Constructs a GeoTrace navigation file.
 *
 * Call `add_nav_fix()` (at least once), then `builder.finish()` to produce a
 * `NavFile`. `finish()` consumes the builder, do not reuse it afterwards.
 *
 * **Non-copyable, movable.**  Destroyed automatically if `finish()` is never called.
 */
class FileBuilder {
  public:
    /**
     * Create a new builder.
     *
     * On allocation failure the builder is left in an error state, surfaced by
     * `status()` and by `finish()` / `try_finish()`.
     */
    FileBuilder() : impl_(::gtd_builder_create()) {
        if (!impl_)
            status_ = Status{GTD_ERR_INTERNAL, "failed to allocate the .gtd builder"};
    }

    FileBuilder(const FileBuilder &) = delete;
    FileBuilder &operator=(const FileBuilder &) = delete;

    FileBuilder(FileBuilder &&) noexcept = default;
    FileBuilder &operator=(FileBuilder &&) noexcept = default;

    ~FileBuilder() = default;

    /** @name Metadata setters (must be called before the first `add_*` call). */
    ///@{

    FileBuilder &title(const std::string &v) {
        record(::gtd_builder_set_title(impl_.get(), v.c_str()));
        return *this;
    }

    FileBuilder &device(const std::string &v) {
        record(::gtd_builder_set_device(impl_.get(), v.c_str()));
        return *this;
    }

    FileBuilder &notes(const std::string &v) {
        record(::gtd_builder_set_notes(impl_.get(), v.c_str()));
        return *this;
    }

    FileBuilder &identity(const std::string &v) {
        record(::gtd_builder_set_identity(impl_.get(), v.c_str()));
        return *this;
    }

    /** Declare the platform the recording was made on. */
    FileBuilder &travel_mode(TravelMode v) {
        record(::gtd_builder_set_travel_mode(impl_.get(), detail::to_c(v)));
        return *this;
    }

    /** Downgrade out-of-range annotation errors to warnings. */
    FileBuilder &lenient() noexcept {
        ::gtd_builder_set_lenient(impl_.get());
        return *this;
    }

    ///@}

    /** @name Data ingestion */
    ///@{

    FileBuilder &add_nav_fix(const NavFix &fix) {
        const std::optional<double> heading_deg =
            fix.heading ? std::optional<double>{fix.heading->as_degrees()} : std::nullopt;
        const std::optional<double> speed_mps =
            fix.speed ? std::optional<double>{fix.speed->as_mps()} : std::nullopt;
        record(::gtd_builder_add_nav_fix(impl_.get(), detail::to_c(fix.gps_time),
                                         detail::to_c(fix.sys_time), fix.lat.as_degrees(),
                                         fix.lon.as_degrees(), detail::to_c(heading_deg),
                                         detail::to_c(speed_mps), detail::to_c(fix.eph_m)));
        return *this;
    }

    FileBuilder &add_satellite_report(const SatelliteReport &report) {
        std::vector<GtdSatellite> sats;
        sats.reserve(report.tracked.size());
        for (const auto &s : report.tracked) {
            sats.push_back(GtdSatellite{
                detail::to_c(s.constellation),
                s.prn,
                static_cast<std::uint8_t>(s.in_fix ? 1 : 0),
                detail::to_c(s.elevation_deg),
                detail::to_c(s.azimuth_deg),
                detail::to_c(s.snr_dbhz),
            });
        }
        record(::gtd_builder_add_satellite_report(impl_.get(), detail::to_c(report.gps_time),
                                                  detail::to_c(report.sys_time), sats.data(),
                                                  sats.size()));
        return *this;
    }

    FileBuilder &add_annotation(const Annotation &ann) {
        const char *label = ann.label.empty() ? nullptr : ann.label.c_str();
        record(::gtd_builder_add_annotation(impl_.get(), detail::to_c(ann.time), label,
                                            detail::to_c(ann.icon)));
        return *this;
    }

    /**
     * Add a structured event marker.
     * @throws InvalidPathError if `variant_path` is malformed.
     */
    FileBuilder &add_event_marker(const EventMarker &marker) {
        const char *ann = marker.annotation.empty() ? nullptr : marker.annotation.c_str();
        record(::gtd_builder_add_event_marker(impl_.get(), marker.variant_path.c_str(),
                                              detail::to_c(marker.sys_time), ann));
        return *this;
    }

    FileBuilder &add_event_marker_style(const EventMarkerStyle &style) {
        const char *color = style.color_hex.empty() ? nullptr : style.color_hex.c_str();
        record(::gtd_builder_add_event_marker_style(impl_.get(), style.variant_path.c_str(),
                                                    detail::to_c(style.icon), color));
        return *this;
    }

    /**
     * Add a scalar or vector sensor channel.
     * @throws InvalidChannelError if the name/a component is malformed or
     *         `values` is not `times.size() * max(components.size(), 1)` long.
     */
    FileBuilder &add_channel(const Channel &ch) {
        std::vector<const char *> components;
        components.reserve(ch.components.size());
        for (const auto &label : ch.components)
            components.push_back(label.c_str());

        std::vector<GtdTimestamp> times;
        times.reserve(ch.times.size());
        for (const auto &t : ch.times)
            times.push_back(detail::to_c(t));

        const std::optional<double> period_deg =
            ch.period ? std::optional<double>{ch.period->as_degrees()} : std::nullopt;

        GtdChannel c{};
        c.name = ch.name.c_str();
        c.unit = ch.unit ? ch.unit->label().c_str() : nullptr;
        c.period_deg = detail::to_c(period_deg);
        c.description = ch.description.empty() ? nullptr : ch.description.c_str();
        c.components = components.empty() ? nullptr : components.data();
        c.n_components = components.size();
        c.times = times.data();
        c.n_times = times.size();
        c.values = ch.values.data();
        c.n_values = ch.values.size();
        const auto mode =
            ch.unit && ch.unit->is_custom() ? GTD_CHANNEL_UNIT_CUSTOM : GTD_CHANNEL_UNIT_RECOGNIZED;
        record(::gtd_builder_add_channel_with_unit_mode(impl_.get(), &c, mode));
        return *this;
    }

    /**
     * Add a type-safe event marker from an event-taxonomy value.
     *
     * Accepts any `enum class` with an `EventEnum<>` specialisation. The path is
     * `base + "/" + seg(v)`.  Use `event_path()` for nested taxonomies.
     * @throws InvalidPathError if the composed path is malformed.
     */
#if defined(__cpp_concepts) && __cpp_concepts >= 201907L
    template <EventEnumValue E>
    FileBuilder &add_event(E v, Timestamp sys_time, std::string note = {}) {
#else
    template <class E, std::enable_if_t<detail::is_event_enum<E>::value, int> = 0>
    FileBuilder &add_event(E v, Timestamp sys_time, std::string note = {}) {
#endif
        return add_event(event_path(v), sys_time, std::move(note));
    }

    /**
     * Add a type-safe event marker from a composed `EventPath`.
     * @throws InvalidPathError if the path is malformed.
     */
    FileBuilder &add_event(const EventPath &path, Timestamp sys_time, std::string note = {}) {
        return add_event_marker(EventMarker{path.str(), sys_time, std::move(note)});
    }

    /**
     * @name add() - dispatch by type
     *
     * Convenience sugar that forwards to the matching `add_*` method, chosen at
     * compile time by overload resolution. Pass a constructed timeline object:
     *
     * @code
     * builder.add(fix).add(report).add(annotation);
     * @endcode
     *
     * Per-variant styling stays on `add_event_marker_style()`, and the typed
     * event helpers stay on `add_event()`. Neither is an `add()` overload.
     */
    ///@{
    FileBuilder &add(const NavFix &fix) { return add_nav_fix(fix); }
    FileBuilder &add(const SatelliteReport &report) { return add_satellite_report(report); }
    FileBuilder &add(const Annotation &ann) { return add_annotation(ann); }
    FileBuilder &add(const EventMarker &marker) { return add_event_marker(marker); }
    FileBuilder &add(const Channel &ch) { return add_channel(ch); }
    ///@}

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

    /**
     * Non-throwing `finish()`: returns the `NavFile`, or the first error
     * recorded by any builder call (or by the finalisation itself).
     */
    Result<NavFile> try_finish();

    /** The first error recorded so far, or an ok status. */
    const Status &status() const noexcept { return status_; }

  private:
    // Record the first error. With exceptions enabled, throw it immediately so
    // the throwing API still reports at the call site. Without exceptions it
    // stays sticky and is surfaced by status() / try_finish().
    void record(GtdStatus s) {
        if (status_.is_ok() && s != GTD_OK) {
            status_ = Status::from(s);
#if GEOTRACE_CPP_EXCEPTIONS
            status_.throw_on_failure();
#endif
        }
    }

    Status status_;
    std::unique_ptr<GtdFileBuilder, detail::BuilderDeleter> impl_;
};

/**
 * A parsed or newly-built GeoTrace navigation file.
 *
 * **Non-copyable, movable.**
 */
class NavFile {
  public:
    /** An empty, invalid file. Only meaningful as the unset value of a `Result`. */
    NavFile() noexcept = default;

    NavFile(const NavFile &) = delete;
    NavFile &operator=(const NavFile &) = delete;

    NavFile(NavFile &&) noexcept = default;
    NavFile &operator=(NavFile &&) noexcept = default;

    ~NavFile() = default;

    /** Open and parse a `.gtd` file, or return the error. */
    static Result<NavFile> try_open(const std::filesystem::path &p) {
        GtdNavFile *out = nullptr;
        const GtdStatus s = ::gtd_nav_file_open(detail::path_string(p).c_str(), &out);
        if (s != GTD_OK)
            return Status::from(s);
        return NavFile(out);
    }

    /**
     * Open and parse a `.gtd` file.
     * @throws IoError, UnsupportedVersionError, Hdf5Error, ParseError on failure.
     */
    static NavFile open(const std::filesystem::path &p) { return try_open(p).value_or_throw(); }

    /** Parse a `.gtd` file from a memory buffer, or return the error. */
    static Result<NavFile> try_from_bytes(span<const std::uint8_t> data) {
        GtdNavFile *out = nullptr;
        const GtdStatus s = ::gtd_nav_file_from_bytes(data.data(), data.size(), &out);
        if (s != GTD_OK)
            return Status::from(s);
        return NavFile(out);
    }

    /** Convenience overload for `std::vector<uint8_t>`. */
    static Result<NavFile> try_from_bytes(const std::vector<std::uint8_t> &data) {
        return try_from_bytes(span<const std::uint8_t>{data});
    }

    /**
     * Parse a `.gtd` file from a memory buffer.
     * @throws IoError, UnsupportedVersionError, Hdf5Error, ParseError on failure.
     */
    static NavFile from_bytes(span<const std::uint8_t> data) {
        return try_from_bytes(data).value_or_throw();
    }

    /** Convenience overload for `std::vector<uint8_t>`. */
    static NavFile from_bytes(const std::vector<std::uint8_t> &data) {
        return try_from_bytes(span<const std::uint8_t>{data}).value_or_throw();
    }

    /** Write the file to disk, or return the error. */
    Status try_write_to_file(const std::filesystem::path &p) const {
        return Status::from(
            ::gtd_nav_file_write_to_path(impl_.get(), detail::path_string(p).c_str()));
    }

    /**
     * Write the file to disk. The `.gtd` extension is appended if the path has none.
     * @throws IoError, Hdf5Error on failure.
     */
    void write_to_file(const std::filesystem::path &p) const {
        try_write_to_file(p).throw_on_failure();
    }

    /** Serialise to a byte vector, or return the error. */
    Result<std::vector<std::uint8_t>> try_to_bytes() const {
        std::uint8_t *buf = nullptr;
        std::size_t len = 0;
        const GtdStatus s = ::gtd_nav_file_to_bytes(impl_.get(), &buf, &len);
        if (s != GTD_OK)
            return Status::from(s);
        auto deleter = [len](std::uint8_t *p) noexcept { ::gtd_free_bytes(p, len); };
        const std::unique_ptr<std::uint8_t, decltype(deleter)> guard(buf, deleter);
        return std::vector<std::uint8_t>{buf, buf + len};
    }

    /**
     * Serialise to a byte vector.
     * @throws IoError, Hdf5Error on failure.
     */
    std::vector<std::uint8_t> to_bytes() const { return try_to_bytes().value_or_throw(); }

    /** @name Metadata (returns empty string_view when field is absent). */
    ///@{

    std::string_view title() const noexcept {
        const char *s = ::gtd_nav_file_title(impl_.get());
        return s ? std::string_view{s} : std::string_view{};
    }
    std::string_view device() const noexcept {
        const char *s = ::gtd_nav_file_device(impl_.get());
        return s ? std::string_view{s} : std::string_view{};
    }
    std::string_view notes() const noexcept {
        const char *s = ::gtd_nav_file_notes(impl_.get());
        return s ? std::string_view{s} : std::string_view{};
    }
    std::string_view identity() const noexcept {
        const char *s = ::gtd_nav_file_identity(impl_.get());
        return s ? std::string_view{s} : std::string_view{};
    }

    /**
     * Travel mode wire name, or an empty view when the field is absent.
     *
     * The value is the raw wire string (e.g. `"car"`); pass it to
     * travel_mode_from_name() for the typed enum. A file written by a newer
     * SDK may carry a wire name outside the known set - such values are still
     * returned here verbatim, never dropped.
     */
    std::string_view travel_mode() const noexcept {
        const char *s = ::gtd_nav_file_travel_mode(impl_.get());
        return s ? std::string_view{s} : std::string_view{};
    }

    ///@}

    /** Number of navigation fixes in the file. */
    std::size_t nav_point_count() const noexcept {
        return ::gtd_nav_file_nav_point_count(impl_.get());
    }

    /** Return the navigation fix at @p idx, or an out-of-range error. */
    Result<NavPointView> try_nav_point(std::size_t idx) const {
        GtdNavPointInfo info{};
        const GtdStatus s = ::gtd_nav_file_get_nav_point(impl_.get(), idx, &info);
        if (s != GTD_OK)
            return Status::from(s);

        NavPointView v{};
        v.gps_time = detail::from_c(info.gps_time);
        v.sys_time = detail::from_c(info.sys_time);
        v.lat = Angle::degrees(info.lat_deg);
        v.lon = Angle::degrees(info.lon_deg);
        v.heading = info.heading_deg.present
                        ? std::optional<Angle>{Angle::degrees(info.heading_deg.value)}
                        : std::nullopt;
        v.speed = info.speed_mps.present
                      ? std::optional<Velocity>{Velocity::mps(info.speed_mps.value)}
                      : std::nullopt;
        v.eph_m = info.eph_m.present ? std::optional<double>{info.eph_m.value} : std::nullopt;
        v.satellite_count = info.sat_count;
        return v;
    }

    /**
     * Return the navigation fix at @p idx.
     * @throws std::out_of_range if `idx >= nav_point_count()`.
     */
    NavPointView nav_point(std::size_t idx) const { return try_nav_point(idx).value_or_throw(); }

    /** Return satellite data for a tracked satellite, or an out-of-range error. */
    Result<SatelliteView> try_satellite(std::size_t nav_idx, std::size_t sat_idx) const {
        GtdSatInfo info{};
        const GtdStatus s = ::gtd_nav_file_get_satellite(impl_.get(), nav_idx, sat_idx, &info);
        if (s != GTD_OK)
            return Status::from(s);

        SatelliteView v{};
        v.constellation = detail::from_c(info.constellation);
        v.prn = info.prn;
        v.in_fix = info.in_fix != 0;
        v.elevation_deg = info.elevation_deg.present
                              ? std::optional<double>{info.elevation_deg.value}
                              : std::nullopt;
        v.azimuth_deg =
            info.azimuth_deg.present ? std::optional<double>{info.azimuth_deg.value} : std::nullopt;
        v.snr_dbhz =
            info.snr_dbhz.present ? std::optional<double>{info.snr_dbhz.value} : std::nullopt;
        return v;
    }

    /**
     * Return satellite data for a specific tracked satellite.
     * @throws std::out_of_range if either index is out of range or the fix has no satellite report.
     */
    SatelliteView satellite(std::size_t nav_idx, std::size_t sat_idx) const {
        return try_satellite(nav_idx, sat_idx).value_or_throw();
    }

    /** Number of event markers in the file. */
    std::size_t event_marker_count() const noexcept {
        return ::gtd_nav_file_event_marker_count(impl_.get());
    }

    /** Return the event marker at @p idx, or an out-of-range error. */
    Result<EventMarkerView> try_event_marker(std::size_t idx) const {
        GtdEventMarkerInfo info{};
        const GtdStatus s = ::gtd_nav_file_get_event_marker(impl_.get(), idx, &info);
        if (s != GTD_OK)
            return Status::from(s);

        EventMarkerView v{};
        v.variant_path = info.variant_path;
        v.sys_time = detail::from_c(info.sys_time);
        v.lat = Angle::degrees(info.lat_deg);
        v.lon = Angle::degrees(info.lon_deg);
        v.annotation = info.has_annotation ? std::string{info.annotation} : std::string{};
        return v;
    }

    /**
     * Return the event marker at @p idx.
     * @throws std::out_of_range if `idx >= event_marker_count()`.
     */
    EventMarkerView event_marker(std::size_t idx) const {
        return try_event_marker(idx).value_or_throw();
    }

    /** Number of channels in the file. */
    std::size_t channel_count() const noexcept { return ::gtd_nav_file_channel_count(impl_.get()); }

    /** Return the channel at @p idx, or an out-of-range error. */
    Result<ChannelView> try_channel(std::size_t idx) const {
        GtdChannelInfo info{};
        const GtdStatus s = ::gtd_nav_file_get_channel(impl_.get(), idx, &info);
        if (s != GTD_OK)
            return Status::from(s);

        ChannelView v{};
        v.name = info.name;
        size_t unit_len = 0;
        std::uint8_t unit_is_custom = 0;
        const GtdStatus unit_size_status = ::gtd_nav_file_get_channel_unit(
            impl_.get(), idx, nullptr, 0, &unit_len, &unit_is_custom);
        if (unit_size_status != GTD_OK)
            return Status::from(unit_size_status);
        if (unit_len > 0) {
            std::vector<char> unit_buffer(unit_len);
            const GtdStatus unit_status =
                ::gtd_nav_file_get_channel_unit(impl_.get(), idx, unit_buffer.data(),
                                                unit_buffer.size(), &unit_len, &unit_is_custom);
            if (unit_status != GTD_OK)
                return Status::from(unit_status);
            const std::string label{unit_buffer.data()};
            v.unit = ChannelUnit::from_file_label(label, unit_is_custom != 0);
        }
        v.period = info.period_deg.present
                       ? std::optional<Angle>{Angle::degrees(info.period_deg.value)}
                       : std::nullopt;
        v.description = info.has_description ? std::string{info.description} : std::string{};

        v.components.reserve(info.component_count);
        for (std::size_t c = 0; c < info.component_count; ++c) {
            // Matches GtdChannelInfo::name[256]; a longer label is truncated by
            // the C API, which cannot report the untruncated length.
            static constexpr std::size_t kChannelLabelCap = 256;
            char buf[kChannelLabelCap] = {};
            if (::gtd_nav_file_get_channel_component(impl_.get(), idx, c, buf, sizeof(buf)) ==
                GTD_OK)
                v.components.emplace_back(buf);
        }

        // info already holds the authoritative counts, so size the buffers from
        // it rather than querying the C accessors again.
        const std::size_t columns = info.component_count > 0 ? info.component_count : 1;
        std::vector<GtdTimestamp> raw_times(info.sample_count);
        if (info.sample_count > 0)
            ::gtd_nav_file_channel_times(impl_.get(), idx, raw_times.data(), raw_times.size());
        v.times.reserve(info.sample_count);
        for (const auto &t : raw_times)
            v.times.push_back(detail::from_c(t));

        v.values.resize(info.sample_count * columns);
        if (!v.values.empty())
            ::gtd_nav_file_channel_values(impl_.get(), idx, v.values.data(), v.values.size());

        return v;
    }

    /**
     * Return the channel at @p idx.
     * @throws std::out_of_range if `idx >= channel_count()`.
     */
    ChannelView channel(std::size_t idx) const { return try_channel(idx).value_or_throw(); }

  private:
    friend class FileBuilder;
    explicit NavFile(GtdNavFile *impl) noexcept : impl_(impl) {}
    std::unique_ptr<GtdNavFile, detail::NavFileDeleter> impl_;
};

inline Result<NavFile> FileBuilder::try_finish() {
    if (status_.is_err())
        return status_;
    GtdNavFile *out = nullptr;
    const GtdStatus s = ::gtd_builder_finish(impl_.release(), &out);
    if (s != GTD_OK)
        return Status::from(s);
    return NavFile(out);
}

inline NavFile FileBuilder::finish() {
    return try_finish().value_or_throw();
}

} // namespace geotrace
