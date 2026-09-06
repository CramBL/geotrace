/**
 * @file geotrace.h
 * @brief GeoTrace C SDK - public API for reading and writing `.gtd` navigation data files.
 *
 * Generated from the Rust FFI crate `geotrace-c`. Do not edit.
 *
 * Link against `libgeotrace_c` and include this header.
 *
 * **Thread safety:** Handles (`GtdFileBuilder*`, `GtdNavFile*`) are not thread-safe.
 * Serialise access to a single handle across threads.
 * `gtd_last_error()` is thread-local and always safe to call.
 *
 * **Ownership:** Every function that returns a handle allocates heap memory.
 * Destroy handles exactly once with the matching `gtd_*_destroy()` function.
 * Passing a destroyed handle is undefined behaviour (same contract as `FILE*`).
 */

#ifndef GEOTRACE_H
#define GEOTRACE_H

#include <stddef.h>
#include <stdint.h>

/** SDK version string. */
#define GEOTRACE_C_VERSION       "0.6.0"
#define GEOTRACE_C_VERSION_MAJOR 0
#define GEOTRACE_C_VERSION_MINOR 6
#define GEOTRACE_C_VERSION_PATCH 0

/** An absent optional double. */
#define GTD_NONE_F64 ((GtdOptF64){.value = 0.0, .present = 0})

/** An optional double with value @p v. */
#define GTD_SOME_F64(v) ((GtdOptF64){.value = (v), .present = 1})

/** An absent optional float. */
#define GTD_NONE_F32 ((GtdOptF32){.value = 0.0f, .present = 0})

/** An optional float with value @p v. */
#define GTD_SOME_F32(v) ((GtdOptF32){.value = (v), .present = 1})

/**
 * Return code for all fallible SDK functions.
 *
 * On failure, call `gtd_last_error()` for a human-readable description.
 */
typedef enum {
    /**
     * Success.
     */
    GTD_OK = 0,
    /**
     * A required pointer argument was NULL.
     */
    GTD_ERR_NULL_ARGUMENT = 1,
    /**
     * Malformed event-marker variant path.
     */
    GTD_ERR_INVALID_PATH = 2,
    /**
     * Builder finished with no nav fixes.
     */
    GTD_ERR_NO_NAV_FIXES = 3,
    /**
     * Annotation(s) outside the nav fix time range.
     */
    GTD_ERR_ANNOTATIONS_OOB = 4,
    /**
     * I/O error (file not found, permission denied, etc.).
     */
    GTD_ERR_IO = 5,
    /**
     * HDF5 library error.
     */
    GTD_ERR_HDF5 = 6,
    /**
     * Unsupported file format version.
     */
    GTD_ERR_VERSION = 7,
    /**
     * String argument contained invalid UTF-8.
     */
    GTD_ERR_UTF8 = 8,
    /**
     * Malformed or corrupt .gtd file (decode failed).
     */
    GTD_ERR_PARSE = 9,
    /**
     * Malformed channel (bad name/component or length mismatch).
     */
    GTD_ERR_INVALID_CHANNEL = 10,
    /**
     * A string is longer than the `.gtd` field that holds it.
     */
    GTD_ERR_FIELD_TOO_LONG = 11,
    /**
     * An argument's value is not allowed.
     */
    GTD_ERR_INVALID_ARGUMENT = 12,
    /**
     * An index is past the end of what it addresses, or an output buffer is
     * too small.
     */
    GTD_ERR_OUT_OF_RANGE = 13,
    /**
     * A call was made in an order the API does not allow.
     */
    GTD_ERR_CALL_ORDER = 14,
    /**
     * Internal error (bug in the SDK).
     */
    GTD_ERR_INTERNAL = 99,
} GtdStatus;

/**
 * GNSS constellation identifier.
 */
typedef enum {
    /**
     * GPS (USA).
     */
    GTD_CONSTELLATION_GPS = 0,
    /**
     * GLONASS (Russia).
     */
    GTD_CONSTELLATION_GLONASS = 1,
    /**
     * Galileo (EU).
     */
    GTD_CONSTELLATION_GALILEO = 2,
    /**
     * BeiDou (China).
     */
    GTD_CONSTELLATION_BEIDOU = 3,
    /**
     * NavIC / IRNSS (India).
     */
    GTD_CONSTELLATION_NAVIC = 4,
    /**
     * QZSS (Japan).
     */
    GTD_CONSTELLATION_QZSS = 5,
} GtdConstellation;

/**
 * Icon for map markers. `GTD_ICON_AUTO` is accepted only by
 * `gtd_builder_add_event_marker_style()`, where the application picks the icon.
 */
typedef enum {
    /**
     * Map pin.
     */
    GTD_ICON_PIN = 0,
    /**
     * Cross / X mark.
     */
    GTD_ICON_CROSS = 1,
    /**
     * Circle.
     */
    GTD_ICON_CIRCLE = 2,
    /**
     * Lightning bolt.
     */
    GTD_ICON_LIGHTNING = 3,
    /**
     * Warning triangle.
     */
    GTD_ICON_WARNING = 4,
    /**
     * Error indicator.
     */
    GTD_ICON_ERROR = 5,
    /**
     * Check mark.
     */
    GTD_ICON_CHECK = 6,
    /**
     * Satellite with signal.
     */
    GTD_ICON_SATELLITE = 7,
    /**
     * Satellite without signal.
     */
    GTD_ICON_SATELLITE_LOST = 8,
    /**
     * Gear / settings.
     */
    GTD_ICON_GEAR = 9,
    /**
     * Refresh / reload.
     */
    GTD_ICON_REFRESH = 10,
    /**
     * Download arrow.
     */
    GTD_ICON_DOWNLOAD = 11,
    /**
     * Upload arrow.
     */
    GTD_ICON_UPLOAD = 12,
    /**
     * Wrench / tool.
     */
    GTD_ICON_WRENCH = 13,
    /**
     * Let the application pick the icon for an event marker variant.
     */
    GTD_ICON_AUTO = 255,
} GtdMarkerIcon;

/**
 * Platform a recording was made on, declared by the recorder.
 */
typedef enum {
    /**
     * Passenger car.
     */
    GTD_TRAVEL_MODE_CAR = 0,
    /**
     * Motorcycle.
     */
    GTD_TRAVEL_MODE_MOTORCYCLE = 1,
    /**
     * Bicycle.
     */
    GTD_TRAVEL_MODE_BICYCLE = 2,
    /**
     * On foot.
     */
    GTD_TRAVEL_MODE_PEDESTRIAN = 3,
    /**
     * Boat or ship.
     */
    GTD_TRAVEL_MODE_BOAT = 4,
    /**
     * Train or tram.
     */
    GTD_TRAVEL_MODE_RAIL = 5,
    /**
     * Aircraft.
     */
    GTD_TRAVEL_MODE_AIRCRAFT = 6,
} GtdTravelMode;

/**
 * How a channel unit label should be interpreted on the write path.
 *
 * A recognized unit has a physical quantity and a conversion factor, so a
 * GeoTrace query compares it against literals in any unit of the same
 * quantity. A custom unit is a label the catalog does not cover. It is stored
 * and shown verbatim, and its values stay unitless in queries. A file may also
 * hold a legacy label that is neither (see @ref gtd_nav_file_get_channel_unit):
 * it is readable but not writable, so neither mode accepts it.
 */
typedef enum {
    /**
     * Validate as a recognized, convertible unit.
     */
    GTD_CHANNEL_UNIT_RECOGNIZED = 0,
    /**
     * Preserve as display-only: queries treat values as unitless.
     */
    GTD_CHANNEL_UNIT_CUSTOM = 1,
} GtdChannelUnitMode;

/**
 * Opaque handle for a file-under-construction.
 *
 * Created by `gtd_builder_create()`. Freed either by `gtd_builder_destroy()`
 * (on error paths) or consumed by `gtd_builder_finish()` (on success).
 */
typedef struct GtdFileBuilder GtdFileBuilder;

/**
 * Opaque handle for a parsed or freshly-built navigation file.
 */
typedef struct GtdNavFile GtdNavFile;

/**
 * UTC Unix epoch timestamp in microseconds.
 *
 * Use `gtd_ts_none()` to represent an absent timestamp.
 */
typedef struct {
    int64_t unix_micros;
} GtdTimestamp;

/**
 * An optional `double` value. Use the `GTD_SOME_F64` and `GTD_NONE_F64`
 * macros to construct values.
 */
typedef struct {
    double value;
    uint8_t present;
} GtdOptF64;

/**
 * An optional `float` value. Use the `GTD_SOME_F32` and `GTD_NONE_F32` macros
 * to construct values.
 */
typedef struct {
    float value;
    uint8_t present;
} GtdOptF32;

/**
 * A satellite entry within a report (write path, input from C).
 *
 * Pass an array of these to `gtd_builder_add_satellite_report()`.
 */
typedef struct {
    /**
     * GNSS constellation.
     */
    GtdConstellation constellation;
    /**
     * Pseudo-random noise number (satellite ID).
     */
    uint32_t prn;
    /**
     * Non-zero if this satellite contributed to the position fix.
     */
    uint8_t in_fix;
    /**
     * Elevation above the horizon in degrees [0, 90].
     */
    GtdOptF32 elevation_deg;
    /**
     * Azimuth from true north in degrees [0, 360).
     */
    GtdOptF32 azimuth_deg;
    /**
     * Signal-to-noise ratio in dB·Hz.
     */
    GtdOptF32 snr_dbhz;
} GtdSatellite;

/**
 * A scalar or vector channel to add via `gtd_builder_add_channel()`.
 *
 * A scalar channel leaves @ref components NULL and @ref n_components zero. A
 * vector channel points @ref components at @ref n_components label strings.
 * @ref values is row-major: @ref n_times rows of one column (scalar) or
 * @ref n_components columns (vector), so @ref n_values must equal
 * `n_times * (n_components > 0 ? n_components : 1)`.
 *
 * Only a channel with @ref period_deg set wraps: a `deg` channel without it
 * holds an unbounded angle.
 */
typedef struct {
    /**
     * Channel name (a lowercase identifier).
     */
    const char *name;
    /**
     * Unit of the values, or NULL. See @ref GtdChannelUnitMode.
     */
    const char *unit;
    /**
     * Wrap period in degrees for an angular channel, or `GTD_NONE_F64`.
     */
    GtdOptF64 period_deg;
    /**
     * Human-readable description, or NULL.
     */
    const char *description;
    /**
     * Component labels for a vector channel, or NULL for scalar.
     */
    const char *const *components;
    /**
     * Number of component labels (0 = scalar channel).
     */
    size_t n_components;
    /**
     * Sample timestamps, one per row.
     */
    const GtdTimestamp *times;
    /**
     * Number of timestamps.
     */
    size_t n_times;
    /**
     * Row-major values, `n_times * max(n_components, 1)` of them.
     */
    const double *values;
    /**
     * Number of values.
     */
    size_t n_values;
} GtdChannel;

/**
 * Channel metadata returned by `gtd_nav_file_get_channel()`.
 *
 * Sample timestamps, values, and component labels are fetched separately with
 * `gtd_nav_file_channel_times()`, `gtd_nav_file_channel_values()`, and
 * `gtd_nav_file_get_channel_component()`. A @ref component_count of zero marks
 * a scalar channel. All string fields are null-terminated and truncated to
 * their buffer size if longer. `gtd_nav_file_get_channel_unit()` reads the unit
 * without that limit and reports whether it is a recognized unit.
 *
 * Only a channel with @ref period_deg set wraps: a `deg` channel without it
 * holds an unbounded angle.
 */
typedef struct {
    /**
     * Channel name.
     */
    char name[256];
    /**
     * Non-zero if @ref unit is set.
     */
    uint8_t has_unit;
    /**
     * Unit of the values, when @ref has_unit.
     */
    char unit[64];
    /**
     * Wrap period in degrees, or absent for a linear channel.
     */
    GtdOptF64 period_deg;
    /**
     * Non-zero if @ref description is set.
     */
    uint8_t has_description;
    /**
     * Description, when @ref has_description.
     */
    char description[1024];
    /**
     * Number of vector components (0 = scalar channel).
     */
    size_t component_count;
    /**
     * Number of sample timestamps (value rows).
     */
    size_t sample_count;
} GtdChannelInfo;

/**
 * Event marker data returned by `gtd_nav_file_get_event_marker()`.
 *
 * All string fields are null-terminated.
 */
typedef struct {
    /**
     * Hierarchical event type path, e.g. `"system/startup"`.
     */
    char variant_path[257];
    /**
     * System time when the event occurred.
     */
    GtdTimestamp sys_time;
    /**
     * WGS-84 latitude of the event.
     */
    double lat_deg;
    /**
     * WGS-84 longitude of the event.
     */
    double lon_deg;
    /**
     * Non-zero if @ref annotation is set.
     */
    uint8_t has_annotation;
    /**
     * Human-readable annotation text, when @ref has_annotation.
     */
    char annotation[1024];
} GtdEventMarkerInfo;

/**
 * Navigation fix data returned by `gtd_nav_file_get_nav_point()`.
 *
 * All fields are caller-owned (no pointers to SDK memory).
 *
 * The ranges on @ref lat_deg, @ref lon_deg and @ref heading_deg, and
 * non-negative @ref speed_mps and @ref eph_m, are data quality expectations,
 * not parse rules.
 * The SDK returns @ref lat_deg and @ref lon_deg unchanged, NaN included.
 * A NaN @ref heading_deg, @ref speed_mps or @ref eph_m is returned as
 * `GTD_NONE_F64`: NaN is how the write path stores an absent one.
 * Checking a value against its range is the caller's job.
 */
typedef struct {
    /**
     * GPS time of the fix. Use `gtd_ts_is_none()` to check.
     */
    GtdTimestamp gps_time;
    /**
     * System (wall-clock) time of the fix.
     */
    GtdTimestamp sys_time;
    /**
     * WGS-84 latitude in degrees, expected in [-90, 90].
     */
    double lat_deg;
    /**
     * WGS-84 longitude in degrees, expected in [-180, 180].
     */
    double lon_deg;
    /**
     * Compass heading in degrees, expected in [0, 360), if known.
     */
    GtdOptF64 heading_deg;
    /**
     * Ground speed in m/s, expected to be non-negative, if known.
     */
    GtdOptF64 speed_mps;
    /**
     * Estimated horizontal position error in metres, expected to be
     * non-negative, if known.
     */
    GtdOptF64 eph_m;
    /**
     * Number of tracked satellites (0 when no satellite report present).
     */
    size_t sat_count;
} GtdNavPointInfo;

/**
 * Satellite data returned by `gtd_nav_file_get_satellite()`.
 */
typedef struct {
    /**
     * GNSS constellation.
     */
    GtdConstellation constellation;
    /**
     * Pseudo-random noise number.
     */
    uint32_t prn;
    /**
     * Non-zero if this satellite contributed to the fix.
     */
    uint8_t in_fix;
    /**
     * Elevation in degrees, if available.
     */
    GtdOptF32 elevation_deg;
    /**
     * Azimuth in degrees, if available.
     */
    GtdOptF32 azimuth_deg;
    /**
     * SNR in dB·Hz, if available.
     */
    GtdOptF32 snr_dbhz;
} GtdSatInfo;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Finalise the builder and produce a `GtdNavFile` handle.
 *
 * The builder is **consumed** by this call regardless of success or failure.
 * Do not call `gtd_builder_destroy()` afterwards.
 *
 * On success, `*out` is set to the new handle.
 * On failure, `*out` is set to NULL and `gtd_last_error()` describes the error.
 *
 * @param builder Builder to finalise.
 * @param out     Output parameter for the resulting file handle.
 *
 * @return `GTD_ERR_NO_NAV_FIXES` if no nav fixes were added.
 * @return `GTD_ERR_ANNOTATIONS_OOB` if annotations fall outside the time range (unless lenient).
 */
GtdStatus gtd_builder_finish(GtdFileBuilder *builder, GtdNavFile **out);

/**
 * Add a GPS navigation fix.
 *
 * At least one nav fix is required before `gtd_builder_finish()`.
 * Fixes must be added in ascending time order.
 *
 * The ranges named below are data quality expectations, not parse rules.
 * The SDK records a value outside its range, NaN included, as given: a recorder
 * that captured bad data must be able to write it.
 * A NaN @p heading_deg, @p speed_mps or @p eph_m reads back as absent: the SDK
 * stores `GTD_NONE_F64` as NaN.
 *
 * @param builder     Builder handle.
 * @param gps_time    GPS time of the fix. Use `gtd_ts_none()` when unavailable.
 * @param sys_time    System (wall-clock) time. Use `gtd_ts_none()` when unavailable.
 * @param lat_deg     WGS-84 latitude in degrees, expected in [-90, 90].
 * @param lon_deg     WGS-84 longitude in degrees, expected in [-180, 180].
 * @param heading_deg Compass heading in degrees, expected in [0, 360), or `GTD_NONE_F64`.
 * @param speed_mps   Ground speed in m/s, expected to be non-negative, or `GTD_NONE_F64`.
 * @param eph_m       Estimated horizontal position error in metres, expected to be
 *                    non-negative, or `GTD_NONE_F64`.
 *
 * @return `GTD_ERR_INVALID_ARGUMENT` if @p gps_time and @p sys_time are both
 *         `gtd_ts_none()`.
 */
GtdStatus gtd_builder_add_nav_fix(GtdFileBuilder *builder,
                                  GtdTimestamp gps_time,
                                  GtdTimestamp sys_time,
                                  double lat_deg,
                                  double lon_deg,
                                  GtdOptF64 heading_deg,
                                  GtdOptF64 speed_mps,
                                  GtdOptF64 eph_m);

/**
 * Add a satellite visibility report.
 *
 * The report is associated with the nearest preceding nav fix.
 * Passing @p n_sats as zero with a NULL @p sats pointer records an empty report.
 *
 * @param builder  Builder handle.
 * @param gps_time GPS time of the report. Use `gtd_ts_none()` when unavailable.
 * @param sys_time System (wall-clock) time of the report.
 * @param sats     Array of @p n_sats satellite entries.
 * @param n_sats   Number of elements in @p sats.
 *
 * @return `GTD_ERR_INVALID_ARGUMENT` if @p gps_time and @p sys_time are both
 *         `gtd_ts_none()`.
 */
GtdStatus gtd_builder_add_satellite_report(GtdFileBuilder *builder,
                                           GtdTimestamp gps_time,
                                           GtdTimestamp sys_time,
                                           const GtdSatellite *sats,
                                           size_t n_sats);

/**
 * Add a legacy map-pin annotation (optional label + icon).
 *
 * @p time must lie within the nav fix time range unless lenient mode is enabled.
 *
 * @param builder Builder handle.
 * @param time    Timestamp of the annotation. Must not be `gtd_ts_none()`.
 * @param label   Human-readable label, or NULL for no label.
 * @param icon    Icon to display.
 *
 * @return `GTD_ERR_FIELD_TOO_LONG` if @p label is longer than 255 bytes.
 * @return `GTD_ERR_INVALID_ARGUMENT` if @p icon is `GTD_ICON_AUTO`, which only
 *         `gtd_builder_add_event_marker_style()` accepts.
 */
GtdStatus gtd_builder_add_annotation(GtdFileBuilder *builder,
                                     GtdTimestamp time,
                                     const char *label,
                                     GtdMarkerIcon icon);

/**
 * Add a structured event marker.
 *
 * Event markers use a hierarchical variant path (e.g. `"system/startup"`) to
 * identify the event type. Paths must be non-empty, consist of alphanumeric
 * segments separated by `/`, and not exceed 255 bytes.
 *
 * @param builder      Builder handle.
 * @param variant_path Hierarchical event type path.
 * @param sys_time     Time of the event. Must not be `gtd_ts_none()`.
 * @param annotation   Optional human-readable text. Pass NULL for none.
 *
 * @return `GTD_ERR_INVALID_PATH` if @p variant_path is malformed.
 * @return `GTD_ERR_FIELD_TOO_LONG` if @p variant_path is longer than 255 bytes,
 *         or @p annotation longer than 511 bytes.
 */
GtdStatus gtd_builder_add_event_marker(GtdFileBuilder *builder,
                                       const char *variant_path,
                                       GtdTimestamp sys_time,
                                       const char *annotation);

/**
 * Register a display style for an event marker variant.
 *
 * Styles are per-variant, not per-event. Calling this multiple times for the
 * same path overwrites the previous style.
 *
 * @param builder      Builder handle.
 * @param variant_path Hierarchical event type path (same format as in
 *                     `gtd_builder_add_event_marker()`).
 * @param icon         Icon to display. `GTD_ICON_AUTO` uses the application default.
 * @param color_hex    Color as an `"#RRGGBB"` string, or NULL for automatic.
 *
 * @note The style is checked when the file is written: a @p variant_path past
 *       255 bytes or a @p color_hex past 7 bytes fails there with
 *       `GTD_ERR_FIELD_TOO_LONG`.
 */
GtdStatus gtd_builder_add_event_marker_style(GtdFileBuilder *builder,
                                             const char *variant_path,
                                             GtdMarkerIcon icon,
                                             const char *color_hex);

/**
 * Add a scalar or vector sensor channel.
 *
 * The channel keeps its own sample timestamps. It is correlated with the nav
 * track by time at query time, not resampled here. See @ref GtdChannel for the
 * field layout, including the row-major `values` convention.
 *
 * @param builder Builder handle.
 * @param channel Channel description. Not retained after the call returns.
 *
 * @return `GTD_ERR_INVALID_CHANNEL` if the unit is unrecognized, the name or a
 *         component label is malformed, or `values` is not
 *         `n_times * max(n_components, 1)` long.
 */
GtdStatus gtd_builder_add_channel(GtdFileBuilder *builder, const GtdChannel *channel);

/**
 * Add a channel with an explicit recognized/custom interpretation for its unit.
 *
 * This entry point preserves the layout of @ref GtdChannel while allowing a
 * display-only custom label through @ref GTD_CHANNEL_UNIT_CUSTOM.
 * `gtd_builder_add_channel()` is this call with
 * @ref GTD_CHANNEL_UNIT_RECOGNIZED. The label is validated and canonicalized as
 * in @ref gtd_channel_unit_parse, so the file stores the canonical spelling. A
 * NULL @ref GtdChannel::unit adds a channel without a unit, whatever
 * @p unit_mode says.
 *
 * @param builder   Builder handle.
 * @param channel   Channel description. Not retained after the call returns.
 * @param unit_mode A @ref GtdChannelUnitMode value.
 *
 * @return `GTD_ERR_INVALID_CHANNEL` for an invalid unit/mode combination or
 *         malformed channel metadata.
 */
GtdStatus gtd_builder_add_channel_with_unit_mode(GtdFileBuilder *builder,
                                                 const GtdChannel *channel,
                                                 uint32_t unit_mode);

/**
 * Create a new file builder.
 *
 * @return A new builder handle, or NULL on allocation failure.
 *         Destroy with `gtd_builder_destroy()` on error, or consume with `gtd_builder_finish()`.
 */
GtdFileBuilder *gtd_builder_create(void);

/**
 * Free a builder without writing a file.
 *
 * Do **not** call this after a successful `gtd_builder_finish()`: that call
 * already consumes the builder.
 *
 * @param builder Builder to destroy. No-op if NULL.
 */
void gtd_builder_destroy(GtdFileBuilder *builder);

/**
 * Set the file title (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 *
 * @return `GTD_ERR_CALL_ORDER` if data has already been added.
 */
GtdStatus gtd_builder_set_title(GtdFileBuilder *builder, const char *title);

/**
 * Set the recording device name (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 *
 * @return `GTD_ERR_CALL_ORDER` if data has already been added.
 */
GtdStatus gtd_builder_set_device(GtdFileBuilder *builder, const char *device);

/**
 * Set free-form notes (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 *
 * @return `GTD_ERR_CALL_ORDER` if data has already been added.
 */
GtdStatus gtd_builder_set_notes(GtdFileBuilder *builder, const char *notes);

/**
 * Set a device/session identity string (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 *
 * @return `GTD_ERR_CALL_ORDER` if data has already been added.
 */
GtdStatus gtd_builder_set_identity(GtdFileBuilder *builder, const char *identity);

/**
 * Declare the platform the recording was made on (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 *
 * @return `GTD_ERR_CALL_ORDER` if data has already been added.
 */
GtdStatus gtd_builder_set_travel_mode(GtdFileBuilder *builder, GtdTravelMode mode);

/**
 * Enable lenient mode.
 *
 * By default `gtd_builder_finish()` returns `GTD_ERR_ANNOTATIONS_OOB` when any
 * annotation falls outside the nav fix time range. Calling this function
 * downgrades that error to a warning and lets the build succeed.
 *
 * Must be called before the first `gtd_builder_add_*` call.
 *
 * @return `GTD_ERR_CALL_ORDER` if data has already been added.
 */
GtdStatus gtd_builder_set_lenient(GtdFileBuilder *builder);

/**
 * Validate and canonicalize a channel unit label.
 *
 * Call with @p out NULL and @p out_capacity zero to query the required byte
 * length, including the terminating NUL, then call again with a large enough
 * buffer. Validation and Unicode handling are identical to the Rust SDK.
 *
 * Under `GTD_CHANNEL_UNIT_RECOGNIZED` the label is trimmed and aliases are
 * resolved, so `"kph"`, `"degrees"` and `"m/s²"` come back as `"km/h"`,
 * `"deg"` and `"m/s2"`. Under `GTD_CHANNEL_UNIT_CUSTOM` the label is only
 * trimmed, and a label matching a recognized unit is rejected: it belongs in
 * `GTD_CHANNEL_UNIT_RECOGNIZED`, which keeps its conversion factor.
 *
 * @param label           Unit label to validate, NUL-terminated UTF-8.
 * @param unit_mode       A @ref GtdChannelUnitMode value.
 * @param out             Buffer for the canonical label, or NULL to size it.
 * @param out_capacity    Bytes writable at @p out.
 * @param required_length Receives the canonical byte length including the NUL.
 *
 * @return `GTD_ERR_INVALID_CHANNEL` if the label is invalid for @p unit_mode or
 *         @p unit_mode is not a @ref GtdChannelUnitMode, `GTD_ERR_UTF8` if
 *         @p label is not UTF-8, `GTD_ERR_OUT_OF_RANGE` if @p out is too small
 *         for the canonical label.
 */
GtdStatus gtd_channel_unit_parse(const char *label,
                                 uint32_t unit_mode,
                                 char *out,
                                 size_t out_capacity,
                                 size_t *required_length);

/**
 * Returns the last error message for the current thread, or NULL if none.
 *
 * The pointer is valid until the next SDK call on this thread.
 */
const char *gtd_last_error(void);

/**
 * Return the number of channels in the file.
 *
 * @param file File handle. Returns 0 if NULL.
 */
size_t gtd_nav_file_channel_count(const GtdNavFile *file);

/**
 * Fill @p out with metadata for the channel at @p index.
 *
 * @param file  File handle.
 * @param index Zero-based index. Must be less than `gtd_nav_file_channel_count(file)`.
 * @param out   Caller-allocated struct to fill.
 *
 * @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last channel.
 */
GtdStatus gtd_nav_file_get_channel(const GtdNavFile *file, size_t index, GtdChannelInfo *out);

/**
 * Read a channel unit without the fixed-size @ref GtdChannelInfo buffer limit.
 *
 * Pass NULL @p out and zero @p out_capacity to query the required byte length,
 * including the trailing null byte. A channel without a unit reports zero.
 * @p is_custom may be NULL when the recognized/custom distinction is not needed.
 *
 * @p is_custom is non-zero for any label that is not a recognized unit. That
 * covers both a custom label and a legacy label an older writer stored, which
 * this SDK reports verbatim and rejects on the write path: passing such a label
 * to @ref gtd_builder_add_channel_with_unit_mode returns
 * `GTD_ERR_INVALID_CHANNEL`.
 *
 * @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last channel.
 */
GtdStatus gtd_nav_file_get_channel_unit(const GtdNavFile *file,
                                        size_t index,
                                        char *out,
                                        size_t out_capacity,
                                        size_t *required_length,
                                        uint8_t *is_custom);

/**
 * Copy the label of a vector channel's component into @p out (null-terminated,
 * truncated to @p out_capacity bytes).
 *
 * @param file            File handle.
 * @param channel_index   Channel index.
 * @param component_index Component index. Must be less than `GtdChannelInfo::component_count`.
 * @param out             Caller-allocated buffer of @p out_capacity bytes.
 * @param out_capacity    Capacity of @p out in bytes.
 *
 * @return `GTD_ERR_OUT_OF_RANGE` if an index is past the end or @p out_capacity
 *         is zero, `GTD_ERR_NULL_ARGUMENT` if @p out is NULL.
 */
GtdStatus gtd_nav_file_get_channel_component(const GtdNavFile *file,
                                             size_t channel_index,
                                             size_t component_index,
                                             char *out,
                                             size_t out_capacity);

/**
 * Copy up to @p out_capacity sample timestamps of the channel at @p channel_index into @p out.
 *
 * @return The channel's total sample count (independent of @p out_capacity). Pass a NULL
 *         @p out or zero @p out_capacity to query the count without copying.
 */
size_t gtd_nav_file_channel_times(const GtdNavFile *file,
                                  size_t channel_index,
                                  GtdTimestamp *out,
                                  size_t out_capacity);

/**
 * Copy up to @p out_capacity values of the channel at @p channel_index into @p out (row-major).
 *
 * @return The channel's total value count, `sample_count * max(component_count, 1)`
 *         (independent of @p out_capacity). Pass a NULL @p out or zero
 *         @p out_capacity to query the count without copying.
 */
size_t gtd_nav_file_channel_values(const GtdNavFile *file,
                                   size_t channel_index,
                                   double *out,
                                   size_t out_capacity);

/**
 * Return the number of event markers in the file.
 *
 * @param file File handle. Returns 0 if NULL.
 */
size_t gtd_nav_file_event_marker_count(const GtdNavFile *file);

/**
 * Fill @p out with data for the event marker at @p index.
 *
 * @param file  File handle.
 * @param index Zero-based index. Must be less than `gtd_nav_file_event_marker_count(file)`.
 * @param out   Caller-allocated struct to fill.
 *
 * @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last event marker.
 */
GtdStatus gtd_nav_file_get_event_marker(const GtdNavFile *file,
                                        size_t index,
                                        GtdEventMarkerInfo *out);

/**
 * Return the file title, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p file.
 */
const char *gtd_nav_file_title(const GtdNavFile *file);

/**
 * Return the recording device name, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p file.
 */
const char *gtd_nav_file_device(const GtdNavFile *file);

/**
 * Return the notes string, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p file.
 */
const char *gtd_nav_file_notes(const GtdNavFile *file);

/**
 * Return the identity string, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p file.
 */
const char *gtd_nav_file_identity(const GtdNavFile *file);

/**
 * Return the travel mode wire name, or NULL if not set.
 *
 * The value is the raw wire string (e.g. `"car"`). Pass it to
 * `gtd_travel_mode_from_name()` for the typed `GtdTravelMode`. A file written
 * by a newer SDK may carry a wire name that fails to parse - such values are
 * still returned here verbatim, never dropped.
 *
 * The returned pointer is valid for the lifetime of @p file.
 */
const char *gtd_nav_file_travel_mode(const GtdNavFile *file);

/**
 * Return the version of the SDK build that wrote the file, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p file.
 */
const char *gtd_nav_file_sdk_version(const GtdNavFile *file);

/**
 * Return the commit of the `geotrace` repository the writing SDK was built from,
 * or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p file.
 */
const char *gtd_nav_file_sdk_git_commit(const GtdNavFile *file);

/**
 * Return the committer timestamp of `gtd_nav_file_sdk_git_commit()`.
 *
 * `gtd_ts_none()` if not set. Use `gtd_ts_is_none()` to check.
 */
GtdTimestamp gtd_nav_file_sdk_commit_time(const GtdNavFile *file);

/**
 * Return the number of navigation fixes in the file.
 *
 * @param file File handle. Returns 0 if NULL.
 */
size_t gtd_nav_file_nav_point_count(const GtdNavFile *file);

/**
 * Fill @p out with data for the navigation fix at @p index.
 *
 * @param file  File handle.
 * @param index Zero-based index. Must be less than `gtd_nav_file_nav_point_count(file)`.
 * @param out   Caller-allocated struct to fill.
 *
 * @return `GTD_ERR_OUT_OF_RANGE` if @p index is past the last nav fix.
 */
GtdStatus gtd_nav_file_get_nav_point(const GtdNavFile *file, size_t index, GtdNavPointInfo *out);

/**
 * Fill @p out with satellite data for a specific satellite within a nav fix.
 *
 * @param file            File handle.
 * @param nav_point_index Nav fix index.
 * @param satellite_index Satellite index within that fix. Must be less than
 *                        `GtdNavPointInfo::sat_count`.
 * @param out             Caller-allocated struct to fill.
 *
 * @return `GTD_ERR_OUT_OF_RANGE` if either index is past the end, or the nav
 *         fix has no satellite report.
 */
GtdStatus gtd_nav_file_get_satellite(const GtdNavFile *file,
                                     size_t nav_point_index,
                                     size_t satellite_index,
                                     GtdSatInfo *out);

/**
 * Open and parse a `.gtd` navigation file.
 *
 * On success, `*out` is set to a new handle.
 * On failure, `*out` is NULL and `gtd_last_error()` describes the error.
 *
 * @param path File path to open.
 * @param out  Output parameter for the file handle.
 *
 * @return `GTD_ERR_VERSION` if the file uses an unsupported format version.
 */
GtdStatus gtd_nav_file_open(const char *path, GtdNavFile **out);

/**
 * Parse a navigation file from an in-memory buffer.
 *
 * The caller retains ownership of @p data. It may be freed after this call returns.
 *
 * @param data   Pointer to the serialised file data.
 * @param length Length of the data in bytes.
 * @param out    Output parameter for the file handle.
 */
GtdStatus gtd_nav_file_from_bytes(const uint8_t *data, size_t length, GtdNavFile **out);

/**
 * Write the navigation file to disk.
 *
 * The `.gtd` extension is appended automatically if @p path has no extension.
 *
 * @param file File handle (not consumed, the caller must still call `gtd_nav_file_destroy()`).
 * @param path Destination file path.
 *
 * @return `GTD_ERR_FIELD_TOO_LONG` if an event marker style holds a variant path
 *         or color longer than its field.
 */
GtdStatus gtd_nav_file_write_to_path(const GtdNavFile *file, const char *path);

/**
 * Serialise the navigation file into a heap-allocated byte buffer.
 *
 * On success, `*buffer` points to a buffer of `*length` bytes that the caller must
 * free with `gtd_free_bytes(*buffer, *length)`.
 *
 * @param file   File handle (not consumed).
 * @param buffer Output: pointer to the allocated buffer.
 * @param length Output: number of bytes in the buffer.
 *
 * @return `GTD_ERR_FIELD_TOO_LONG` if an event marker style holds a variant path
 *         or color longer than its field.
 */
GtdStatus gtd_nav_file_to_bytes(const GtdNavFile *file, uint8_t **buffer, size_t *length);

/**
 * Free a byte buffer returned by `gtd_nav_file_to_bytes()`.
 *
 * @p buffer and @p length must match the values written by `gtd_nav_file_to_bytes()`.
 * No-op if @p buffer is NULL.
 *
 * @param buffer Pointer to the buffer.
 * @param length Number of bytes in the buffer.
 */
void gtd_free_bytes(uint8_t *buffer, size_t length);

/**
 * Destroy a navigation file handle and free all associated memory.
 *
 * @param file Handle to destroy. No-op if NULL.
 */
void gtd_nav_file_destroy(GtdNavFile *file);

/**
 * Construct a timestamp from whole seconds since the Unix epoch.
 */
GtdTimestamp gtd_ts_from_seconds(uint64_t seconds);

/**
 * Construct a timestamp from milliseconds since the Unix epoch.
 */
GtdTimestamp gtd_ts_from_millis(uint64_t millis);

/**
 * Construct a timestamp from microseconds since the Unix epoch.
 */
GtdTimestamp gtd_ts_from_micros(uint64_t micros);

/**
 * Construct a timestamp from nanoseconds since the Unix epoch (truncated to µs).
 */
GtdTimestamp gtd_ts_from_nanos(uint64_t nanos);

/**
 * The timestamp value that represents an absent timestamp.
 */
GtdTimestamp gtd_ts_none(void);

/**
 * Returns non-zero if @p timestamp is the absent timestamp.
 */
uint8_t gtd_ts_is_none(GtdTimestamp timestamp);

/**
 * Wire name of a travel mode, e.g. `GTD_TRAVEL_MODE_CAR` -> `"car"`.
 *
 * The returned pointer is a static string and always valid.
 */
const char *gtd_travel_mode_name(GtdTravelMode mode);

/**
 * Parse a wire name (as produced by `gtd_travel_mode_name()` or read from
 * `gtd_nav_file_travel_mode()`) back into a travel mode.
 *
 * @param name Wire name, e.g. `"bicycle"`, NUL-terminated.
 * @param out  Caller-allocated result, written on success.
 *
 * @return `GTD_ERR_PARSE` if @p name is not a known travel mode.
 */
GtdStatus gtd_travel_mode_from_name(const char *name, GtdTravelMode *out);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* GEOTRACE_H */
