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

#ifndef GEOTRACE_GEOTRACE_HPP
#define GEOTRACE_GEOTRACE_HPP

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
#include <variant>
#include <vector>

#include <geotrace.h> // C SDK (already has extern "C" guards)
#include <geotrace/unit_catalog.hpp>

// A user tests the version with `#if`, where an `enum` is not visible.
// NOLINTBEGIN(cppcoreguidelines-macro-to-enum,modernize-macro-to-enum)
#define GEOTRACE_CPP_VERSION       "0.6.0"
#define GEOTRACE_CPP_VERSION_MAJOR 0
#define GEOTRACE_CPP_VERSION_MINOR 6
#define GEOTRACE_CPP_VERSION_PATCH 0
// NOLINTEND(cppcoreguidelines-macro-to-enum,modernize-macro-to-enum)

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

    // The three converting constructors are implicit and take a C array, as
    // std::span's do.
    // NOLINTNEXTLINE(hicpp-explicit-conversions,cppcoreguidelines-avoid-c-arrays,hicpp-avoid-c-arrays,modernize-avoid-c-arrays)
    template <std::size_t N> constexpr span(T (&arr)[N]) noexcept : data_(arr), size_(N) {}

    template <typename Container,
              typename = std::enable_if_t<
                  !std::is_same_v<std::decay_t<Container>, span> &&
                  std::is_convertible_v<decltype(std::declval<Container &>().data()), pointer>>>
    // NOLINTNEXTLINE(hicpp-explicit-conversions)
    constexpr span(Container &container) noexcept
        : data_(container.data()), size_(container.size()) {}

    template <
        typename Container,
        typename = std::enable_if_t<
            !std::is_same_v<std::decay_t<Container>, span> &&
            std::is_convertible_v<decltype(std::declval<const Container &>().data()), const T *>>>
    // NOLINTNEXTLINE(hicpp-explicit-conversions)
    constexpr span(const Container &container) noexcept
        : data_(container.data()), size_(container.size()) {}

    [[nodiscard]] constexpr pointer data() const noexcept { return data_; }
    [[nodiscard]] constexpr size_type size() const noexcept { return size_; }
    [[nodiscard]] constexpr bool empty() const noexcept { return size_ == 0; }
    [[nodiscard]] constexpr iterator begin() const noexcept { return data_; }
    [[nodiscard]] constexpr iterator end() const noexcept { return data_ + size_; }
    [[nodiscard]] constexpr T &operator[](size_type index) const noexcept {
        // The subscript is the polyfill's own bounds contract, as std::span's is.
        return data_[index]; // NOLINT(cppcoreguidelines-pro-bounds-pointer-arithmetic)
    }

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
    AnnotationsOutOfRangeError(std::size_t annotation_count, const std::string &msg)
        : BuildError(msg), count(annotation_count) {}
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

/** A string is longer than the `.gtd` field that holds it. */
struct FieldTooLongError : Error {
    using Error::Error;
};

/** Malformed or corrupt `.gtd` file content (decode failed). */
struct ParseError : Error {
    using Error::Error;
};

/**
 * A channel was malformed (bad name/component, length mismatch, or duplicate
 * name). Derives `Error`, as `InvalidPathError` does: both are
 * input-validation errors. The duplicate-name check runs at `finish()`.
 */
struct InvalidChannelError : Error {
    using Error::Error;
};

/** A `FileBuilder` setter was called after the first `add_*` call. */
struct CallOrderError : Error {
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
[[noreturn]] inline void throw_typed(GtdStatus status, const std::string &msg) {
#if GEOTRACE_CPP_EXCEPTIONS
    switch (status) {
    case GTD_ERR_NULL_ARGUMENT:
        // The caller reached the C API directly, bypassing the wrapper: the
        // wrapper itself never passes a null pointer.
        throw std::invalid_argument(msg);
    case GTD_ERR_OUT_OF_RANGE:
        throw std::out_of_range(msg);
    case GTD_ERR_CALL_ORDER:
        throw CallOrderError(msg);
    case GTD_ERR_NO_NAV_FIXES:
        throw NoNavFixesError(msg);
    case GTD_ERR_ANNOTATIONS_OOB:
        throw AnnotationsOutOfRangeError(0, msg);
    case GTD_ERR_INVALID_PATH:
        throw InvalidPathError(msg);
    case GTD_ERR_FIELD_TOO_LONG:
        throw FieldTooLongError(msg);
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
    case GTD_ERR_INVALID_ARGUMENT:
        throw std::invalid_argument(msg);
    default:
        throw Error(msg);
    }
#else
    (void)status;
    abort_with(msg);
#endif
}

// Encode a filesystem path as UTF-8 for the C API.
[[nodiscard]] inline std::string path_string(const std::filesystem::path &path) {
#ifdef __cpp_lib_char8_t
    const auto utf8 = path.u8string();
    return std::string(utf8.begin(), utf8.end());
#else
    return path.u8string();
#endif
}

[[nodiscard]] constexpr GtdOptF64 to_c(std::optional<double> value) noexcept {
    // GTD_SOME_F64 and GTD_NONE_F64 expand to C99 compound-literal +
    // designated-initializer syntax, which MSVC rejects in C++ mode (errors
    // C4576/C7555).
    GtdOptF64 result{};
    if (value) {
        result.value = *value;
        result.present = 1;
    }
    return result;
}

struct BuilderDeleter {
    void operator()(GtdFileBuilder *builder) const noexcept { ::gtd_builder_destroy(builder); }
};

struct NavFileDeleter {
    void operator()(GtdNavFile *file) const noexcept { ::gtd_nav_file_destroy(file); }
};

} // namespace detail

/**
 * An error returned by value from a `try_*` method.
 *
 * `code` is the underlying `GtdStatus`. `description` is a human-readable
 * message. This is the non-throwing error channel: check `is_ok()` or call
 * `value_or_throw()` on the enclosing `Result`.
 */
struct [[nodiscard]] Status {
    GtdStatus code = GTD_OK;
    std::string description;

    Status() = default;
    Status(GtdStatus status_code, std::string status_description)
        : code(status_code), description(std::move(status_description)) {}

    /// Build a `Status` from a `GtdStatus`, capturing the thread-local message.
    static Status from(GtdStatus status) {
        if (status == GTD_OK) {
            return Status{};
        }
        const char *raw = ::gtd_last_error();
        return Status{status, (raw != nullptr) ? raw : "unknown error"};
    }

    [[nodiscard]] constexpr bool is_ok() const noexcept { return code == GTD_OK; }
    [[nodiscard]] constexpr bool is_err() const noexcept { return code != GTD_OK; }
    [[nodiscard]] constexpr explicit operator bool() const noexcept { return is_ok(); }

    /// Throw the matching exception on failure (no-op on success). With
    /// exceptions disabled this prints and aborts, so prefer `is_ok()` there.
    void throw_on_failure() const {
        if (is_err()) {
            detail::throw_typed(code, description);
        }
    }
};

/**
 * The result of a fallible operation: either a value or a `Status` error.
 *
 * Modelled on Rust's `Result`. Inspect `is_ok()` / `error()` and call `value()`,
 * or call `value_or_throw()` to throw the error (or abort without exceptions).
 */
template <typename T> struct [[nodiscard]] Result {
    // Both constructors are implicit: a `try_*` method returns its value or
    // `Status::from(status)` directly.
    Result(T value) : value_(std::move(value)) {} // NOLINT(hicpp-explicit-conversions)
    // An error result must carry a real error: an ok status here would falsely
    // report success with a default-constructed value.
    // NOLINTNEXTLINE(hicpp-explicit-conversions)
    Result(Status status) : status_(std::move(status)) { assert(status_.is_err()); }
    Result() = delete;

    [[nodiscard]] constexpr bool is_ok() const noexcept { return status_.is_ok(); }
    [[nodiscard]] constexpr bool is_err() const noexcept { return status_.is_err(); }
    [[nodiscard]] constexpr explicit operator bool() const noexcept { return is_ok(); }
    [[nodiscard]] constexpr const Status &error() const noexcept { return status_; }

    [[nodiscard]] constexpr const T *get_if() const noexcept { return value_ ? &*value_ : nullptr; }
    [[nodiscard]] constexpr T *get_if() noexcept { return value_ ? &*value_ : nullptr; }

    [[nodiscard]] const T &value() const & {
        status_.throw_on_failure();
        assert(value_.has_value());
        return *value_;
    }
    [[nodiscard]] T &value() & {
        status_.throw_on_failure();
        assert(value_.has_value());
        return *value_;
    }

    [[nodiscard]] const T &value_or_throw() const & { return value(); }
    [[nodiscard]] T value_or_throw() && {
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
 * Always an instant. An absent timestamp is `std::optional<Timestamp>`.
 */
struct [[nodiscard]] Timestamp {
    std::int64_t unix_micros;

    explicit constexpr Timestamp(std::int64_t micros) noexcept : unix_micros(micros) {}

    static Timestamp from_seconds(std::uint64_t seconds) noexcept {
        return Timestamp{::gtd_ts_from_seconds(seconds).unix_micros};
    }
    static Timestamp from_millis(std::uint64_t millis) noexcept {
        return Timestamp{::gtd_ts_from_millis(millis).unix_micros};
    }
    static Timestamp from_micros(std::uint64_t micros) noexcept {
        return Timestamp{::gtd_ts_from_micros(micros).unix_micros};
    }
    static Timestamp from_nanos(std::uint64_t nanos) noexcept {
        return Timestamp{::gtd_ts_from_nanos(nanos).unix_micros};
    }

#if defined(__cpp_impl_three_way_comparison) && __cpp_impl_three_way_comparison >= 201907L
    auto operator<=>(const Timestamp &) const = default;
#else
    constexpr bool operator==(Timestamp other) const noexcept {
        return unix_micros == other.unix_micros;
    }
    constexpr bool operator!=(Timestamp other) const noexcept { return !(*this == other); }
    constexpr bool operator<(Timestamp other) const noexcept {
        return unix_micros < other.unix_micros;
    }
    constexpr bool operator<=(Timestamp other) const noexcept {
        return unix_micros <= other.unix_micros;
    }
    constexpr bool operator>(Timestamp other) const noexcept {
        return unix_micros > other.unix_micros;
    }
    constexpr bool operator>=(Timestamp other) const noexcept {
        return unix_micros >= other.unix_micros;
    }
#endif
};

/**
 * The two timestamps a recorder holds for one fix or satellite report, either
 * of which may be absent. A caller cannot transpose the two clocks: each has
 * its own field.
 */
struct RecordedFixTimestamps {
    std::optional<Timestamp> gps_time;
    std::optional<Timestamp> sys_time;
};

/** The clock or clocks that stamped a nav fix or a satellite report. */
class [[nodiscard]] FixTime {
  public:
    /** The receiver's timestamp, with no host clock recorded. */
    static constexpr FixTime receiver(Timestamp gps) noexcept { return FixTime{ReceiverOnly{gps}}; }

    /** The host clock's timestamp, taken while the receiver had no lock. */
    static constexpr FixTime host(Timestamp sys) noexcept { return FixTime{HostOnly{sys}}; }

    /** Both timestamps, recorded under lock on a host that also stamped it. */
    static constexpr FixTime both(Timestamp gps, Timestamp sys) noexcept {
        return FixTime{BothClocks{gps, sys}};
    }

    /** `std::nullopt` when the recorder holds neither timestamp. */
    [[nodiscard]] static constexpr std::optional<FixTime>
    from_recorded(const RecordedFixTimestamps &recorded) noexcept {
        if (recorded.gps_time && recorded.sys_time) {
            return both(*recorded.gps_time, *recorded.sys_time);
        }
        if (recorded.gps_time) {
            return receiver(*recorded.gps_time);
        }
        if (recorded.sys_time) {
            return host(*recorded.sys_time);
        }
        return std::nullopt;
    }

    [[nodiscard]] constexpr std::optional<Timestamp> gps_time() const noexcept {
        if (const auto *receiver_only = std::get_if<ReceiverOnly>(&clocks_)) {
            return receiver_only->gps;
        }
        if (const auto *both_clocks = std::get_if<BothClocks>(&clocks_)) {
            return both_clocks->gps;
        }
        return std::nullopt;
    }

    [[nodiscard]] constexpr std::optional<Timestamp> sys_time() const noexcept {
        if (const auto *host_only = std::get_if<HostOnly>(&clocks_)) {
            return host_only->sys;
        }
        if (const auto *both_clocks = std::get_if<BothClocks>(&clocks_)) {
            return both_clocks->sys;
        }
        return std::nullopt;
    }

  private:
    struct ReceiverOnly {
        Timestamp gps;
    };
    struct HostOnly {
        Timestamp sys;
    };
    struct BothClocks {
        Timestamp gps;
        Timestamp sys;
    };

    using Clocks = std::variant<ReceiverOnly, HostOnly, BothClocks>;

    explicit constexpr FixTime(Clocks clocks) noexcept : clocks_(clocks) {}

    Clocks clocks_;
};

/** Angular measurement stored in degrees. */
class [[nodiscard]] Angle {
  public:
    static constexpr Angle degrees(double deg) noexcept { return Angle{deg}; }
    static constexpr Angle radians(double rad) noexcept { return Angle{rad * kDegreesPerRadian}; }

    [[nodiscard]] constexpr double as_degrees() const noexcept { return deg_; }
    [[nodiscard]] constexpr double as_radians() const noexcept { return deg_ * kRadiansPerDegree; }

#if defined(__cpp_impl_three_way_comparison) && __cpp_impl_three_way_comparison >= 201907L
    auto operator<=>(const Angle &) const = default;
#else
    constexpr bool operator==(Angle other) const noexcept { return deg_ == other.deg_; }
    constexpr bool operator!=(Angle other) const noexcept { return !(*this == other); }
    constexpr bool operator<(Angle other) const noexcept { return deg_ < other.deg_; }
    constexpr bool operator<=(Angle other) const noexcept { return deg_ <= other.deg_; }
    constexpr bool operator>(Angle other) const noexcept { return deg_ > other.deg_; }
    constexpr bool operator>=(Angle other) const noexcept { return deg_ >= other.deg_; }
#endif

  private:
    // M_PI is a POSIX extension not guaranteed by the C++ standard (absent on MSVC
    // without _USE_MATH_DEFINES), so we use our own constant instead.
    static constexpr double kPi = 3.141592653589793238462643383279502884;
    // Each conversion multiplies by its own factor. Dividing by the other one
    // differs by 1 ULP for some values.
    static constexpr double kDegreesPerRadian = 180.0 / kPi;
    static constexpr double kRadiansPerDegree = kPi / 180.0;
    explicit constexpr Angle(double deg) noexcept : deg_(deg) {}
    double deg_ = 0.0;
};

/** Velocity stored in metres per second. */
class [[nodiscard]] Velocity {
  public:
    // Conversion factors kept bit-identical to the Rust SDK (units.rs
    // MPS_PER_KMH / MPS_PER_KNOT) so the same input yields the same stored m/s
    // across SDKs. Use the constant-multiply form (`v * (1.0/3.6)`), not
    // `v / 3.6`, which differs by 1 ULP for some values.
    static constexpr double kMpsPerKmh = 1.0 / 3.6;
    static constexpr double kMpsPerKnot = 1852.0 / 3600.0;

    static constexpr Velocity mps(double value) noexcept { return Velocity{value}; }
    static constexpr Velocity kmh(double value) noexcept { return Velocity{value * kMpsPerKmh}; }
    static constexpr Velocity knots(double value) noexcept { return Velocity{value * kMpsPerKnot}; }

    [[nodiscard]] constexpr double as_mps() const noexcept { return mps_; }
    [[nodiscard]] constexpr double as_kmh() const noexcept { return mps_ / kMpsPerKmh; }
    [[nodiscard]] constexpr double as_knots() const noexcept { return mps_ / kMpsPerKnot; }

#if defined(__cpp_impl_three_way_comparison) && __cpp_impl_three_way_comparison >= 201907L
    auto operator<=>(const Velocity &) const = default;
#else
    constexpr bool operator==(Velocity other) const noexcept { return mps_ == other.mps_; }
    constexpr bool operator!=(Velocity other) const noexcept { return !(*this == other); }
    constexpr bool operator<(Velocity other) const noexcept { return mps_ < other.mps_; }
    constexpr bool operator<=(Velocity other) const noexcept { return mps_ <= other.mps_; }
    constexpr bool operator>(Velocity other) const noexcept { return mps_ > other.mps_; }
    constexpr bool operator>=(Velocity other) const noexcept { return mps_ >= other.mps_; }
#endif

  private:
    explicit constexpr Velocity(double mps) noexcept : mps_(mps) {}
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

[[nodiscard]] constexpr GtdConstellation to_c(Constellation constellation) noexcept {
    switch (constellation) {
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

[[nodiscard]] constexpr Constellation from_c(GtdConstellation constellation) noexcept {
    switch (constellation) {
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

[[nodiscard]] constexpr GtdMarkerIcon to_c(MarkerIcon icon) noexcept {
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
    }
    return GTD_ICON_PIN;
}

[[nodiscard]] constexpr GtdMarkerIcon to_c(std::optional<MarkerIcon> icon) noexcept {
    return icon ? to_c(*icon) : GTD_ICON_AUTO;
}

[[nodiscard]] constexpr GtdTravelMode to_c(TravelMode mode) noexcept {
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

[[nodiscard]] constexpr TravelMode from_c(GtdTravelMode mode) noexcept {
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

[[nodiscard]] constexpr GtdTimestamp to_c(Timestamp timestamp) noexcept {
    return GtdTimestamp{timestamp.unix_micros};
}

[[nodiscard]] inline GtdTimestamp to_c(std::optional<Timestamp> timestamp) noexcept {
    return timestamp ? to_c(*timestamp) : ::gtd_ts_none();
}

[[nodiscard]] inline std::optional<Timestamp> from_c(GtdTimestamp timestamp) noexcept {
    if (::gtd_ts_is_none(timestamp) != 0) {
        return std::nullopt;
    }
    return Timestamp{timestamp.unix_micros};
}

// `gtd_ts_none()` never appears in an event marker or channel sample timestamp:
// the `.gtd` format stores an instant for both.
[[nodiscard]] constexpr Timestamp instant_from_c(GtdTimestamp timestamp) noexcept {
    return Timestamp{timestamp.unix_micros};
}

} // namespace detail

/** Wire name of @p mode, e.g. `"car"` for `TravelMode::Car`. */
[[nodiscard]] inline std::string_view travel_mode_name(TravelMode mode) noexcept {
    return std::string_view{::gtd_travel_mode_name(detail::to_c(mode))};
}

/**
 * Parse a wire name (as produced by travel_mode_name() or read from
 * NavFile::travel_mode()) back into a travel mode.
 *
 * Returns `std::nullopt` for names outside the known set.
 */
[[nodiscard]] inline std::optional<TravelMode>
travel_mode_from_name(const std::string &name) noexcept {
    GtdTravelMode mode{};
    if (::gtd_travel_mode_from_name(name.c_str(), &mode) != GTD_OK) {
        return std::nullopt;
    }
    return detail::from_c(mode);
}

/**
 * A single GPS navigation fix.
 *
 * `lat` is expected in [-90, 90] degrees, `lon` in [-180, 180], `heading` in
 * [0, 360), `speed` and `eph_m` to be non-negative.
 * These are data quality expectations, not parse rules.
 * The SDK writes every value it is given, NaN included: a recorder that
 * captured bad data must be able to write it.
 * An absent `heading`, `speed` or `eph_m` is written as NaN: a NaN given for one
 * of the three reads back as `std::nullopt`.
 */
struct NavFix {
    FixTime time;
    Angle lat = Angle::degrees(0.0);
    Angle lon = Angle::degrees(0.0);
    // Each default keeps `-Wmissing-field-initializers` quiet for a caller that
    // lists only `time`, `lat` and `lon`.
    std::optional<Angle> heading = std::nullopt;
    std::optional<Velocity> speed = std::nullopt;
    std::optional<double> eph_m = std::nullopt;
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
    FixTime time;
    std::vector<Satellite> tracked;
};

/** A legacy map-pin annotation. */
struct Annotation {
    Timestamp time;
    std::string label;
    MarkerIcon icon = MarkerIcon::Pin;
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
    // `std::nullopt` means the application picks the icon.
    std::optional<MarkerIcon> icon = std::nullopt;
    std::string color_hex; // empty = auto (hash-derived). format: "#RRGGBB"
};

/**
 * The unit of a channel's values: a recognized unit or a custom label.
 *
 * A recognized unit comes from the @ref RecognizedUnit catalog, has a physical
 * quantity and a conversion factor, and is what a GeoTrace query compares
 * against unit literals. A custom unit is any other label: it is stored and
 * shown verbatim and its values stay unitless in queries.
 *
 * A unit read back from a file (@ref ChannelView::unit) reports `is_custom()`
 * for any label that is not a catalog unit, including a legacy label an older
 * writer stored. Adding a channel with such a label throws
 * `InvalidChannelError`.
 *
 * ```
 * auto accel = geotrace::ChannelUnit::recognized(geotrace::RecognizedUnit::Mg);
 * auto score = geotrace::ChannelUnit::custom("vendor score");
 * ```
 */
class [[nodiscard]] ChannelUnit {
  public:
    /** A catalog unit, spelled canonically. */
    static ChannelUnit recognized(RecognizedUnit unit) {
        return ChannelUnit{recognized_unit_label(unit), false};
    }

    /**
     * A display-only label for a unit the catalog does not cover.
     *
     * Throws `InvalidChannelError` for an empty label, a label with a control
     * character, or a label that spells a recognized unit: declare that one
     * with `recognized()` or `parse_recognized()`.
     */
    static ChannelUnit custom(const std::string &label) {
        return try_custom(label).value_or_throw();
    }

    /** The non-throwing form of `custom()`, returning a `Result`. */
    static Result<ChannelUnit> try_custom(const std::string &label) {
        return try_parse(label, GTD_CHANNEL_UNIT_CUSTOM, true);
    }

    /**
     * A catalog unit named by its label, resolving aliases: `"kph"` is
     * `"km/h"`, `"degrees"` is `"deg"`, `"m/s²"` is `"m/s2"`.
     *
     * Throws `InvalidChannelError` for a label outside the catalog: store that
     * one with `custom()`.
     */
    static ChannelUnit parse_recognized(std::string_view label) {
        return try_parse_recognized(label).value_or_throw();
    }

    /** The non-throwing form of `parse_recognized()`, returning a `Result`. */
    static Result<ChannelUnit> try_parse_recognized(std::string_view label) {
        return try_parse(std::string{label}, GTD_CHANNEL_UNIT_RECOGNIZED, false);
    }

    /** The canonical label as stored in the file. */
    [[nodiscard]] constexpr const std::string &label() const noexcept { return label_; }

    /** True for a custom label, false for a catalog unit. */
    [[nodiscard]] constexpr bool is_custom() const noexcept { return custom_; }

    friend bool operator==(const ChannelUnit &a, const ChannelUnit &b) noexcept {
        return a.custom_ == b.custom_ && a.label_ == b.label_;
    }

  private:
    friend class NavFile;

    ChannelUnit(std::string label, bool custom) : label_{std::move(label)}, custom_{custom} {}

    static ChannelUnit from_file_label(std::string label, bool custom) {
        return custom ? ChannelUnit{std::move(label), true} : parse_recognized(label);
    }

    static Result<ChannelUnit> try_parse(const std::string &label, GtdChannelUnitMode mode,
                                         bool custom) {
        std::size_t required = 0;
        GtdStatus status = ::gtd_channel_unit_parse(label.c_str(), static_cast<std::uint32_t>(mode),
                                                    nullptr, 0, &required);
        if (status != GTD_OK) {
            return Status::from(status);
        }
        std::vector<char> canonical(required);
        status = ::gtd_channel_unit_parse(label.c_str(), static_cast<std::uint32_t>(mode),
                                          canonical.data(), canonical.size(), &required);
        if (status != GTD_OK) {
            return Status::from(status);
        }
        return ChannelUnit{std::string{canonical.data()}, custom};
    }

    std::string label_;
    bool custom_;
};

/**
 * A scalar or vector sensor channel sampled at its own rate.
 *
 * Leave `components` empty for a scalar channel, or list one label per column
 * for a vector channel (an accelerometer's `{"x", "y", "z"}`). `values` is
 * row-major: `times.size()` rows of one column (scalar) or `components.size()`
 * columns (vector).
 */
struct Channel {
    std::string name;
    std::optional<ChannelUnit> unit;
    // Wrap period, none = linear: a `deg` channel without one is an unbounded angle.
    std::optional<Angle> period;
    std::string description;             // empty = none
    std::vector<std::string> components; // empty = scalar channel
    std::vector<Timestamp> times;
    std::vector<double> values;
};

/** Channel data returned by `NavFile::channel()`. String fields are copies. */
struct ChannelView {
    std::string name;
    std::optional<ChannelUnit> unit;
    // Wrap period, none = linear: a `deg` channel without one is an unbounded angle.
    std::optional<Angle> period;
    std::string description;             // empty = none
    std::vector<std::string> components; // empty = scalar channel
    std::vector<Timestamp> times;
    std::vector<double> values; // row-major, times.size() * max(components.size(), 1)

    /** Whether this is a vector channel (has named components). */
    [[nodiscard]] bool is_vector() const noexcept { return !components.empty(); }
};

/**
 * Data for one navigation fix, returned by `NavFile::nav_point()`.
 *
 * The value ranges of `NavFix` apply here too, as expectations.
 * The SDK returns `lat` and `lon` unchanged, NaN included.
 * A NaN `heading`, `speed` or `eph_m` is returned as `std::nullopt`: NaN is how
 * the write path stores an absent one.
 * Checking a value against its range is the caller's job.
 */
struct NavPointView {
    std::optional<Timestamp> gps_time;
    std::optional<Timestamp> sys_time;
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
 * `enum` values directly - the compiler rejects anything that is not a known
 * event `enum`.
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
 *  - `static constexpr std::string_view base` - the segment for this level.
 *  - `static constexpr std::string_view seg(E)` - the leaf segment per value.
 *
 * `seg()` must return a non-empty segment for **every** enumerator. Write it as
 * a `switch` with no `default` so that adding an enumerator without a matching
 * case is a compile error under `-Wswitch`/`-Werror` (the SDK examples and
 * tests build that way). A value that falls through would compose a malformed
 * path such as `"power/"`, surfacing only as a runtime `InvalidPathError`.
 *
 * The primary template is left undefined so that only specialised `enum` types
 * are accepted by `event_path()` and `FileBuilder::add_event()`.
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
 * proper EventEnum<> specialisation.
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
class [[nodiscard]] EventPath {
  public:
    explicit EventPath(std::string path) noexcept : path_(std::move(path)) {}

    [[nodiscard]] constexpr const std::string &str() const noexcept { return path_; }
    [[nodiscard]] explicit operator std::string_view() const noexcept { return path_; }

  private:
    std::string path_;
};

namespace detail {
template <class E> void append_event_seg(std::string &out, E value, bool with_base) {
    if (with_base) {
        out += EventEnum<E>::base;
    }
    out += '/';
    out += EventEnum<E>::seg(value);
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
template <EventEnumValue E, EventEnumValue... Es> EventPath event_path(E first, Es... rest) {
#else
template <class E, class... Es> EventPath event_path(E first, Es... rest) {
    static_assert(detail::is_event_enum<E>::value,
                  "event_path: no EventEnum<> specialisation for this type");
    static_assert((detail::is_event_enum<Es>::value && ...),
                  "event_path: no EventEnum<> specialisation for a nested type");
#endif
    std::string out;
    detail::append_event_seg(out, first, true);
    (detail::append_event_seg(out, rest, false), ...);
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
        if (!impl_) {
            status_ = Status{GTD_ERR_INTERNAL, "failed to allocate the .gtd builder"};
        }
    }

    FileBuilder(const FileBuilder &) = delete;
    FileBuilder &operator=(const FileBuilder &) = delete;

    FileBuilder(FileBuilder &&) noexcept = default;
    FileBuilder &operator=(FileBuilder &&) noexcept = default;

    ~FileBuilder() = default;

    /** @name Metadata setters (must be called before the first `add_*` call). */
    ///@{

    FileBuilder &title(const std::string &title) {
        record(::gtd_builder_set_title(impl_.get(), title.c_str()));
        return *this;
    }

    FileBuilder &device(const std::string &device) {
        record(::gtd_builder_set_device(impl_.get(), device.c_str()));
        return *this;
    }

    FileBuilder &notes(const std::string &notes) {
        record(::gtd_builder_set_notes(impl_.get(), notes.c_str()));
        return *this;
    }

    FileBuilder &identity(const std::string &identity) {
        record(::gtd_builder_set_identity(impl_.get(), identity.c_str()));
        return *this;
    }

    /** Declare the platform the recording was made on. */
    FileBuilder &travel_mode(TravelMode mode) {
        record(::gtd_builder_set_travel_mode(impl_.get(), detail::to_c(mode)));
        return *this;
    }

    /** Downgrade out-of-range annotation errors to warnings. */
    FileBuilder &lenient() {
        record(::gtd_builder_set_lenient(impl_.get()));
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
        record(::gtd_builder_add_nav_fix(impl_.get(), detail::to_c(fix.time.gps_time()),
                                         detail::to_c(fix.time.sys_time()), fix.lat.as_degrees(),
                                         fix.lon.as_degrees(), detail::to_c(heading_deg),
                                         detail::to_c(speed_mps), detail::to_c(fix.eph_m)));
        return *this;
    }

    FileBuilder &add_satellite_report(const SatelliteReport &report) {
        std::vector<GtdSatellite> sats;
        sats.reserve(report.tracked.size());
        for (const auto &satellite : report.tracked) {
            sats.push_back(GtdSatellite{
                detail::to_c(satellite.constellation),
                satellite.prn,
                static_cast<std::uint8_t>(satellite.in_fix ? 1 : 0),
                detail::to_c(satellite.elevation_deg),
                detail::to_c(satellite.azimuth_deg),
                detail::to_c(satellite.snr_dbhz),
            });
        }
        record(::gtd_builder_add_satellite_report(impl_.get(), detail::to_c(report.time.gps_time()),
                                                  detail::to_c(report.time.sys_time()), sats.data(),
                                                  sats.size()));
        return *this;
    }

    /**
     * Add a legacy map-pin annotation.
     * @throws FieldTooLongError if `label` is longer than 255 bytes.
     */
    FileBuilder &add_annotation(const Annotation &ann) {
        const char *label = ann.label.empty() ? nullptr : ann.label.c_str();
        record(::gtd_builder_add_annotation(impl_.get(), detail::to_c(ann.time), label,
                                            detail::to_c(ann.icon)));
        return *this;
    }

    /**
     * Add a structured event marker.
     * @throws InvalidPathError if `variant_path` is malformed.
     * @throws FieldTooLongError if `variant_path` is longer than 255 bytes, or
     *         `annotation` longer than 511 bytes.
     */
    FileBuilder &add_event_marker(const EventMarker &marker) {
        const char *ann = marker.annotation.empty() ? nullptr : marker.annotation.c_str();
        record(::gtd_builder_add_event_marker(impl_.get(), marker.variant_path.c_str(),
                                              detail::to_c(marker.sys_time), ann));
        return *this;
    }

    /**
     * Register a display style for an event marker variant.
     *
     * The style is checked when the file is written: a `variant_path` past 255
     * bytes or a `color_hex` past 7 bytes fails there with a
     * `FieldTooLongError`.
     */
    FileBuilder &add_event_marker_style(const EventMarkerStyle &style) {
        const char *color = style.color_hex.empty() ? nullptr : style.color_hex.c_str();
        record(::gtd_builder_add_event_marker_style(impl_.get(), style.variant_path.c_str(),
                                                    detail::to_c(style.icon), color));
        return *this;
    }

    /**
     * Add a scalar or vector sensor channel.
     *
     * The unit keeps its recognized/custom interpretation, so a
     * `ChannelUnit::custom` label is stored as a display-only unit.
     * @throws InvalidChannelError if the name/a component is malformed, the
     *         unit is not valid writer input, or `values` is not
     *         `times.size() * max(components.size(), 1)` long.
     */
    FileBuilder &add_channel(const Channel &channel) {
        std::vector<const char *> components;
        components.reserve(channel.components.size());
        for (const auto &label : channel.components) {
            components.push_back(label.c_str());
        }

        std::vector<GtdTimestamp> times;
        times.reserve(channel.times.size());
        for (const auto &time : channel.times) {
            times.push_back(detail::to_c(time));
        }

        const std::optional<double> period_deg =
            channel.period ? std::optional<double>{channel.period->as_degrees()} : std::nullopt;

        GtdChannel c_api_channel{};
        c_api_channel.name = channel.name.c_str();
        c_api_channel.unit = channel.unit ? channel.unit->label().c_str() : nullptr;
        c_api_channel.period_deg = detail::to_c(period_deg);
        c_api_channel.description =
            channel.description.empty() ? nullptr : channel.description.c_str();
        c_api_channel.components = components.empty() ? nullptr : components.data();
        c_api_channel.n_components = components.size();
        c_api_channel.times = times.data();
        c_api_channel.n_times = times.size();
        c_api_channel.values = channel.values.data();
        c_api_channel.n_values = channel.values.size();
        const auto mode = channel.unit && channel.unit->is_custom() ? GTD_CHANNEL_UNIT_CUSTOM
                                                                    : GTD_CHANNEL_UNIT_RECOGNIZED;
        record(::gtd_builder_add_channel_with_unit_mode(impl_.get(), &c_api_channel, mode));
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
    FileBuilder &add_event(E value, Timestamp sys_time, std::string note = {}) {
#else
    template <class E, std::enable_if_t<detail::is_event_enum<E>::value, int> = 0>
    FileBuilder &add_event(E value, Timestamp sys_time, std::string note = {}) {
#endif
        return add_event(event_path(value), sys_time, std::move(note));
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
    FileBuilder &add(const Channel &channel) { return add_channel(channel); }
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
    [[nodiscard]] constexpr const Status &status() const noexcept { return status_; }

  private:
    // Record the first error. With exceptions enabled, throw it immediately so
    // the throwing API still reports at the call site. Without exceptions it
    // stays sticky and is surfaced by status() / try_finish().
    void record(GtdStatus status) {
        if (status_.is_ok() && status != GTD_OK) {
            status_ = Status::from(status);
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
class [[nodiscard]] NavFile {
  public:
    NavFile() = delete;

    NavFile(const NavFile &) = delete;
    NavFile &operator=(const NavFile &) = delete;

    NavFile(NavFile &&) noexcept = default;
    NavFile &operator=(NavFile &&) noexcept = default;

    ~NavFile() = default;

    /** Open and parse a `.gtd` file, or return the error. */
    static Result<NavFile> try_open(const std::filesystem::path &path) {
        GtdNavFile *out = nullptr;
        const GtdStatus status = ::gtd_nav_file_open(detail::path_string(path).c_str(), &out);
        if (status != GTD_OK) {
            return Status::from(status);
        }
        return NavFile(out);
    }

    /**
     * Open and parse a `.gtd` file.
     * @throws IoError, UnsupportedVersionError, Hdf5Error, ParseError on failure.
     */
    static NavFile open(const std::filesystem::path &path) {
        return try_open(path).value_or_throw();
    }

    /** Parse a `.gtd` file from a memory buffer, or return the error. */
    static Result<NavFile> try_from_bytes(span<const std::uint8_t> data) {
        GtdNavFile *out = nullptr;
        const GtdStatus status = ::gtd_nav_file_from_bytes(data.data(), data.size(), &out);
        if (status != GTD_OK) {
            return Status::from(status);
        }
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
    Status try_write_to_file(const std::filesystem::path &path) const {
        return Status::from(
            ::gtd_nav_file_write_to_path(impl_.get(), detail::path_string(path).c_str()));
    }

    /**
     * Write the file to disk. The `.gtd` extension is appended if the path has none.
     * @throws IoError, Hdf5Error on failure.
     * @throws FieldTooLongError if an event marker style holds a variant path or
     *         color longer than its field.
     */
    void write_to_file(const std::filesystem::path &path) const {
        try_write_to_file(path).throw_on_failure();
    }

    /** Serialise to a byte vector, or return the error. */
    Result<std::vector<std::uint8_t>> try_to_bytes() const {
        std::uint8_t *buf = nullptr;
        std::size_t len = 0;
        const GtdStatus status = ::gtd_nav_file_to_bytes(impl_.get(), &buf, &len);
        if (status != GTD_OK) {
            return Status::from(status);
        }
        auto deleter = [len](std::uint8_t *bytes) noexcept { ::gtd_free_bytes(bytes, len); };
        const std::unique_ptr<std::uint8_t, decltype(deleter)> guard(buf, deleter);
        // The C SDK reports the buffer's length, and the vector below reads that
        // many bytes.
        // NOLINTNEXTLINE(cppcoreguidelines-pro-bounds-pointer-arithmetic)
        return std::vector<std::uint8_t>{buf, buf + len};
    }

    /**
     * Serialise to a byte vector.
     * @throws IoError, Hdf5Error on failure.
     * @throws FieldTooLongError if an event marker style holds a variant path or
     *         color longer than its field.
     */
    [[nodiscard]] std::vector<std::uint8_t> to_bytes() const {
        return try_to_bytes().value_or_throw();
    }

    /** @name Metadata (returns empty `std::string_view` when field is absent). */
    ///@{

    [[nodiscard]] std::string_view title() const noexcept {
        const char *title = ::gtd_nav_file_title(impl_.get());
        return title != nullptr ? std::string_view{title} : std::string_view{};
    }
    [[nodiscard]] std::string_view device() const noexcept {
        const char *device = ::gtd_nav_file_device(impl_.get());
        return device != nullptr ? std::string_view{device} : std::string_view{};
    }
    [[nodiscard]] std::string_view notes() const noexcept {
        const char *notes = ::gtd_nav_file_notes(impl_.get());
        return notes != nullptr ? std::string_view{notes} : std::string_view{};
    }
    [[nodiscard]] std::string_view identity() const noexcept {
        const char *identity = ::gtd_nav_file_identity(impl_.get());
        return identity != nullptr ? std::string_view{identity} : std::string_view{};
    }

    /**
     * Travel mode wire name, or an empty view when the field is absent.
     *
     * The value is the raw wire string (e.g. `"car"`). Pass it to
     * travel_mode_from_name() for the typed `enum`. A file written by a newer
     * SDK may carry a wire name outside the known set - such values are still
     * returned here verbatim, never dropped.
     */
    [[nodiscard]] std::string_view travel_mode() const noexcept {
        const char *mode = ::gtd_nav_file_travel_mode(impl_.get());
        return mode != nullptr ? std::string_view{mode} : std::string_view{};
    }

    ///@}

    /** @name The SDK build that wrote the file (an empty view or
     * `std::nullopt` when absent). */
    ///@{

    /** Version of the SDK build that wrote the file. */
    [[nodiscard]] std::string_view sdk_version() const noexcept {
        const char *version = ::gtd_nav_file_sdk_version(impl_.get());
        return version != nullptr ? std::string_view{version} : std::string_view{};
    }

    /** Commit of the geotrace repository the writing SDK was built from. */
    [[nodiscard]] std::string_view sdk_git_commit() const noexcept {
        const char *commit = ::gtd_nav_file_sdk_git_commit(impl_.get());
        return commit != nullptr ? std::string_view{commit} : std::string_view{};
    }

    /** Committer timestamp of sdk_git_commit(). */
    [[nodiscard]] std::optional<Timestamp> sdk_commit_time() const noexcept {
        return detail::from_c(::gtd_nav_file_sdk_commit_time(impl_.get()));
    }

    ///@}

    /** Number of navigation fixes in the file. */
    [[nodiscard]] std::size_t nav_point_count() const noexcept {
        return ::gtd_nav_file_nav_point_count(impl_.get());
    }

    /** Return the navigation fix at @p idx, or an out-of-range error. */
    Result<NavPointView> try_nav_point(std::size_t idx) const {
        GtdNavPointInfo info{};
        const GtdStatus status = ::gtd_nav_file_get_nav_point(impl_.get(), idx, &info);
        if (status != GTD_OK) {
            return Status::from(status);
        }

        NavPointView point{};
        point.gps_time = detail::from_c(info.gps_time);
        point.sys_time = detail::from_c(info.sys_time);
        point.lat = Angle::degrees(info.lat_deg);
        point.lon = Angle::degrees(info.lon_deg);
        point.heading = info.heading_deg.present != 0
                            ? std::optional<Angle>{Angle::degrees(info.heading_deg.value)}
                            : std::nullopt;
        point.speed = info.speed_mps.present != 0
                          ? std::optional<Velocity>{Velocity::mps(info.speed_mps.value)}
                          : std::nullopt;
        point.eph_m =
            info.eph_m.present != 0 ? std::optional<double>{info.eph_m.value} : std::nullopt;
        point.satellite_count = info.sat_count;
        return point;
    }

    /**
     * Return the navigation fix at @p idx.
     * @throws std::out_of_range if `idx >= nav_point_count()`.
     */
    [[nodiscard]] NavPointView nav_point(std::size_t idx) const {
        return try_nav_point(idx).value_or_throw();
    }

    /** Return satellite data for a tracked satellite, or an out-of-range error. */
    Result<SatelliteView> try_satellite(std::size_t nav_idx, std::size_t sat_idx) const {
        GtdSatInfo info{};
        const GtdStatus status = ::gtd_nav_file_get_satellite(impl_.get(), nav_idx, sat_idx, &info);
        if (status != GTD_OK) {
            return Status::from(status);
        }

        SatelliteView satellite{};
        satellite.constellation = detail::from_c(info.constellation);
        satellite.prn = info.prn;
        satellite.in_fix = info.in_fix != 0;
        satellite.elevation_deg = info.elevation_deg.present != 0
                                      ? std::optional<double>{info.elevation_deg.value}
                                      : std::nullopt;
        satellite.azimuth_deg = info.azimuth_deg.present != 0
                                    ? std::optional<double>{info.azimuth_deg.value}
                                    : std::nullopt;
        satellite.snr_dbhz =
            info.snr_dbhz.present != 0 ? std::optional<double>{info.snr_dbhz.value} : std::nullopt;
        return satellite;
    }

    /**
     * Return satellite data for a specific tracked satellite.
     * @throws std::out_of_range if either index is out of range or the fix has no satellite report.
     */
    [[nodiscard]] SatelliteView satellite(std::size_t nav_idx, std::size_t sat_idx) const {
        return try_satellite(nav_idx, sat_idx).value_or_throw();
    }

    /** Number of event markers in the file. */
    [[nodiscard]] std::size_t event_marker_count() const noexcept {
        return ::gtd_nav_file_event_marker_count(impl_.get());
    }

    /** Return the event marker at @p idx, or an out-of-range error. */
    Result<EventMarkerView> try_event_marker(std::size_t idx) const {
        GtdEventMarkerInfo info{};
        const GtdStatus status = ::gtd_nav_file_get_event_marker(impl_.get(), idx, &info);
        if (status != GTD_OK) {
            return Status::from(status);
        }

        // GtdEventMarkerInfo holds its strings in fixed C buffers, and the C SDK
        // terminates each string.
        // NOLINTBEGIN(cppcoreguidelines-pro-bounds-array-to-pointer-decay,hicpp-no-array-decay)
        return EventMarkerView{
            info.variant_path,
            detail::instant_from_c(info.sys_time),
            Angle::degrees(info.lat_deg),
            Angle::degrees(info.lon_deg),
            info.has_annotation != 0 ? std::string{info.annotation} : std::string{},
        };
        // NOLINTEND(cppcoreguidelines-pro-bounds-array-to-pointer-decay,hicpp-no-array-decay)
    }

    /**
     * Return the event marker at @p idx.
     * @throws std::out_of_range if `idx >= event_marker_count()`.
     */
    [[nodiscard]] EventMarkerView event_marker(std::size_t idx) const {
        return try_event_marker(idx).value_or_throw();
    }

    /** Number of channels in the file. */
    [[nodiscard]] std::size_t channel_count() const noexcept {
        return ::gtd_nav_file_channel_count(impl_.get());
    }

    /** Return the channel at @p idx, or an out-of-range error. */
    Result<ChannelView> try_channel(std::size_t idx) const {
        GtdChannelInfo info{};
        const GtdStatus status = ::gtd_nav_file_get_channel(impl_.get(), idx, &info);
        if (status != GTD_OK) {
            return Status::from(status);
        }

        // GtdChannelInfo holds its strings in fixed C buffers, and the component
        // accessor fills a buffer that the caller declares.
        // NOLINTBEGIN(cppcoreguidelines-pro-bounds-array-to-pointer-decay,hicpp-no-array-decay,cppcoreguidelines-avoid-c-arrays,hicpp-avoid-c-arrays,modernize-avoid-c-arrays)
        ChannelView view{};
        view.name = info.name;
        size_t unit_len = 0;
        std::uint8_t unit_is_custom = 0;
        const GtdStatus unit_size_status = ::gtd_nav_file_get_channel_unit(
            impl_.get(), idx, nullptr, 0, &unit_len, &unit_is_custom);
        if (unit_size_status != GTD_OK) {
            return Status::from(unit_size_status);
        }
        if (unit_len > 0) {
            std::vector<char> unit_buffer(unit_len);
            const GtdStatus unit_status =
                ::gtd_nav_file_get_channel_unit(impl_.get(), idx, unit_buffer.data(),
                                                unit_buffer.size(), &unit_len, &unit_is_custom);
            if (unit_status != GTD_OK) {
                return Status::from(unit_status);
            }
            const std::string label{unit_buffer.data()};
            view.unit = ChannelUnit::from_file_label(label, unit_is_custom != 0);
        }
        view.period = info.period_deg.present != 0
                          ? std::optional<Angle>{Angle::degrees(info.period_deg.value)}
                          : std::nullopt;
        view.description =
            info.has_description != 0 ? std::string{info.description} : std::string{};

        view.components.reserve(info.component_count);
        for (std::size_t c = 0; c < info.component_count; ++c) {
            // Matches GtdChannelInfo::name[256]. The C API truncates a longer
            // label and cannot report the untruncated length.
            static constexpr std::size_t kChannelLabelCap = 256;
            char buf[kChannelLabelCap] = {};
            if (::gtd_nav_file_get_channel_component(impl_.get(), idx, c, buf, sizeof(buf)) ==
                GTD_OK) {
                view.components.emplace_back(buf);
            }
        }

        // The buffer sizes come from `info`, which holds the authoritative
        // counts.
        const std::size_t columns = info.component_count > 0 ? info.component_count : 1;
        std::vector<GtdTimestamp> raw_times(info.sample_count);
        if (info.sample_count > 0) {
            ::gtd_nav_file_channel_times(impl_.get(), idx, raw_times.data(), raw_times.size());
        }
        view.times.reserve(info.sample_count);
        for (const auto &time : raw_times) {
            view.times.push_back(detail::instant_from_c(time));
        }

        view.values.resize(info.sample_count * columns);
        if (!view.values.empty()) {
            ::gtd_nav_file_channel_values(impl_.get(), idx, view.values.data(), view.values.size());
        }

        return view;
        // NOLINTEND(cppcoreguidelines-pro-bounds-array-to-pointer-decay,hicpp-no-array-decay,cppcoreguidelines-avoid-c-arrays,hicpp-avoid-c-arrays,modernize-avoid-c-arrays)
    }

    /**
     * Return the channel at @p idx.
     * @throws std::out_of_range if `idx >= channel_count()`.
     */
    [[nodiscard]] ChannelView channel(std::size_t idx) const {
        return try_channel(idx).value_or_throw();
    }

  private:
    friend class FileBuilder;
    explicit NavFile(GtdNavFile *impl) noexcept : impl_(impl) {}
    std::unique_ptr<GtdNavFile, detail::NavFileDeleter> impl_;
};

inline Result<NavFile> FileBuilder::try_finish() {
    if (status_.is_err()) {
        return status_;
    }
    GtdNavFile *out = nullptr;
    const GtdStatus status = ::gtd_builder_finish(impl_.release(), &out);
    if (status != GTD_OK) {
        return Status::from(status);
    }
    return NavFile(out);
}

inline NavFile FileBuilder::finish() {
    return try_finish().value_or_throw();
}

} // namespace geotrace

#endif // GEOTRACE_GEOTRACE_HPP
