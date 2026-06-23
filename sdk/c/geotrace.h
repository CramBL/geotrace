/**
 * @file geotrace.h
 * @brief GeoTrace C SDK - public API for reading and writing `.gtd` navigation data files.
 *
 * Link against `libgeotrace_c` and include this header.
 *
 * **Thread safety:** Handles (`GtdFileBuilder*`, `GtdNavFile*`) are not thread-safe;
 * serialise access to a single handle across threads.
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

#ifdef __cplusplus
extern "C" {
#endif

/** @defgroup version Version macros */
/** @{ */
#define GEOTRACE_C_VERSION       "0.2.0-rc.1" /**< SDK version string. */
#define GEOTRACE_C_VERSION_MAJOR 0
#define GEOTRACE_C_VERSION_MINOR 2
#define GEOTRACE_C_VERSION_PATCH 0
/** @} */

/**
 * @defgroup handles Opaque handle types
 * @{
 */

/** Opaque handle for a file under construction. Created by gtd_builder_create(). */
typedef struct GtdFileBuilder GtdFileBuilder;

/** Opaque handle for a parsed or freshly-built navigation file. */
typedef struct GtdNavFile GtdNavFile;

/** @} */

/**
 * @defgroup status Status codes
 * @{
 */

/**
 * Return code for all fallible SDK functions.
 *
 * On failure, call `gtd_last_error()` for a human-readable description.
 */
typedef enum {
    GTD_OK = 0,                  /**< Success. */
    GTD_ERR_NULL_ARGUMENT = 1,   /**< A required pointer argument was NULL. */
    GTD_ERR_INVALID_PATH = 2,    /**< Malformed event-marker variant path. */
    GTD_ERR_NO_NAV_FIXES = 3,    /**< Builder finished with no nav fixes. */
    GTD_ERR_ANNOTATIONS_OOB = 4, /**< Annotation(s) outside the nav fix time range. */
    GTD_ERR_IO = 5,              /**< I/O error (file not found, permission denied, etc.). */
    GTD_ERR_HDF5 = 6,            /**< HDF5 library error. */
    GTD_ERR_VERSION = 7,         /**< Unsupported file format version. */
    GTD_ERR_UTF8 = 8,            /**< String argument contained invalid UTF-8. */
    GTD_ERR_INTERNAL = 99,       /**< Internal error (bug in the SDK). */
} GtdStatus;

/**
 * Returns the last error message for the current thread, or NULL if none.
 *
 * The pointer is valid until the next SDK call on this thread.
 */
const char *gtd_last_error(void);

/** @} */

/**
 * @defgroup timestamps Timestamps
 * @{
 */

/**
 * UTC Unix epoch timestamp in microseconds.
 *
 * Use `gtd_ts_none()` to represent an absent timestamp.
 */
typedef struct {
    int64_t unix_micros;
} GtdTimestamp;

/** Construct a timestamp from whole seconds since the Unix epoch. */
GtdTimestamp gtd_ts_from_seconds(uint64_t secs);

/** Construct a timestamp from milliseconds since the Unix epoch. */
GtdTimestamp gtd_ts_from_millis(uint64_t ms);

/** Construct a timestamp from microseconds since the Unix epoch. */
GtdTimestamp gtd_ts_from_micros(uint64_t us);

/** Construct a timestamp from nanoseconds since the Unix epoch (truncated to µs). */
GtdTimestamp gtd_ts_from_nanos(uint64_t ns);

/** Sentinel value representing an absent timestamp. */
GtdTimestamp gtd_ts_none(void);

/** Returns non-zero if @p ts is the absent-timestamp sentinel. */
uint8_t gtd_ts_is_none(GtdTimestamp ts);

/** @} */

/**
 * @defgroup optf64 Optional double
 * @{
 */

/** An optional `double` value. Use the macros below to construct values. */
typedef struct {
    double value;
    uint8_t present;
} GtdOptF64;

/** An absent optional double. */
#define GTD_NONE_F64 ((GtdOptF64){.value = 0.0, .present = 0})

/** An optional double with value @p v. */
#define GTD_SOME_F64(v) ((GtdOptF64){.value = (v), .present = 1})

/** @} */

/**
 * @defgroup constellation GNSS constellation
 * @{
 */

/** GNSS constellation identifier. */
typedef enum {
    GTD_CONSTELLATION_GPS = 0,     /**< GPS (USA). */
    GTD_CONSTELLATION_GLONASS = 1, /**< GLONASS (Russia). */
    GTD_CONSTELLATION_GALILEO = 2, /**< Galileo (EU). */
    GTD_CONSTELLATION_BEIDOU = 3,  /**< BeiDou (China). */
} GtdConstellation;

/** @} */

/**
 * @defgroup icon Map marker icons
 * @{
 */

/** Icon for map markers. Use `GTD_ICON_AUTO` to let the application choose. */
typedef enum {
    GTD_ICON_PIN = 0,            /**< Map pin. */
    GTD_ICON_CROSS = 1,          /**< Cross / X mark. */
    GTD_ICON_CIRCLE = 2,         /**< Circle. */
    GTD_ICON_LIGHTNING = 3,      /**< Lightning bolt. */
    GTD_ICON_WARNING = 4,        /**< Warning triangle. */
    GTD_ICON_ERROR = 5,          /**< Error indicator. */
    GTD_ICON_CHECK = 6,          /**< Check mark. */
    GTD_ICON_SATELLITE = 7,      /**< Satellite with signal. */
    GTD_ICON_SATELLITE_LOST = 8, /**< Satellite without signal. */
    GTD_ICON_GEAR = 9,           /**< Gear / settings. */
    GTD_ICON_REFRESH = 10,       /**< Refresh / reload. */
    GTD_ICON_DOWNLOAD = 11,      /**< Download arrow. */
    GTD_ICON_UPLOAD = 12,        /**< Upload arrow. */
    GTD_ICON_WRENCH = 13,        /**< Wrench / tool. */
    GTD_ICON_AUTO = 255,         /**< Use the application default for this variant. */
} GtdMarkerIcon;

/** @} */

/**
 * @defgroup satellite Satellite data
 * @{
 */

/**
 * One tracked satellite within a satellite report (write path).
 *
 * Pass an array of these to `gtd_builder_add_satellite_report()`.
 */
typedef struct {
    GtdConstellation constellation; /**< GNSS constellation. */
    uint32_t prn;                   /**< Pseudo-random noise number (satellite ID). */
    uint8_t in_fix;          /**< Non-zero if this satellite contributed to the position fix. */
    GtdOptF64 elevation_deg; /**< Elevation above the horizon in degrees [0, 90]. */
    GtdOptF64 azimuth_deg;   /**< Azimuth from true north in degrees [0, 360). */
    GtdOptF64 snr_dbhz;      /**< Signal-to-noise ratio in dB·Hz. */
} GtdSatellite;

/** @} */

/**
 * @defgroup navpoint Nav point
 * @{
 */

/**
 * Navigation fix data returned by `gtd_nav_file_get_nav_point()`.
 *
 * All fields are caller-owned (no pointers to SDK memory).
 */
typedef struct {
    GtdTimestamp gps_time; /**< GPS time of the fix. Use `gtd_ts_is_none()` to check. */
    GtdTimestamp sys_time; /**< System (wall-clock) time of the fix. */
    double lat_deg;        /**< WGS-84 latitude in degrees. */
    double lon_deg;        /**< WGS-84 longitude in degrees. */
    GtdOptF64 heading_deg; /**< Compass heading in degrees [0, 360), if known. */
    GtdOptF64 speed_mps;   /**< Ground speed in m/s, if known. */
    GtdOptF64 eph_m;       /**< Estimated horizontal position error in metres, if known. */
    size_t sat_count; /**< Number of tracked satellites (0 when no satellite report present). */
} GtdNavPointInfo;

/** @} */

/**
 * @defgroup satinfo Satellite info (read path)
 * @{
 */

/**
 * Satellite data returned by `gtd_nav_file_get_satellite()`.
 */
typedef struct {
    GtdConstellation constellation; /**< GNSS constellation. */
    uint32_t prn;                   /**< Pseudo-random noise number. */
    uint8_t in_fix;                 /**< Non-zero if this satellite contributed to the fix. */
    GtdOptF64 elevation_deg;        /**< Elevation in degrees, if available. */
    GtdOptF64 azimuth_deg;          /**< Azimuth in degrees, if available. */
    GtdOptF64 snr_dbhz;             /**< SNR in dB·Hz, if available. */
} GtdSatInfo;

/** @} */

/**
 * @defgroup eventmarker Event marker info (read path)
 * @{
 */

/**
 * Event marker data returned by `gtd_nav_file_get_event_marker()`.
 *
 * All string fields are null-terminated.
 */
typedef struct {
    char variant_path[257]; /**< Hierarchical event type path, e.g. `"system/startup"`. */
    GtdTimestamp sys_time;  /**< System time when the event occurred. */
    double lat_deg;         /**< WGS-84 latitude of the event. */
    double lon_deg;         /**< WGS-84 longitude of the event. */
    uint8_t has_annotation; /**< Non-zero if @ref annotation is set. */
    char annotation[1024];  /**< Human-readable annotation text, when @ref has_annotation. */
} GtdEventMarkerInfo;

/** @} */

/**
 * @defgroup builder Builder (write path)
 * @{
 */

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
 * Do **not** call this after a successful `gtd_builder_finish()` - that call
 * already consumes the builder.
 *
 * @param b Builder to destroy.  No-op if NULL.
 */
void gtd_builder_destroy(GtdFileBuilder *b);

/**
 * Set the file title (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 * @return `GTD_ERR_INTERNAL` if data has already been added.
 */
GtdStatus gtd_builder_set_title(GtdFileBuilder *b, const char *title);

/**
 * Set the recording device name (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 * @return `GTD_ERR_INTERNAL` if data has already been added.
 */
GtdStatus gtd_builder_set_device(GtdFileBuilder *b, const char *device);

/**
 * Set free-form notes (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 * @return `GTD_ERR_INTERNAL` if data has already been added.
 */
GtdStatus gtd_builder_set_notes(GtdFileBuilder *b, const char *notes);

/**
 * Set a device/session identity string (optional).
 *
 * Must be called before the first `gtd_builder_add_*` call.
 * @return `GTD_ERR_INTERNAL` if data has already been added.
 */
GtdStatus gtd_builder_set_identity(GtdFileBuilder *b, const char *identity);

/**
 * Enable lenient mode.
 *
 * By default `gtd_builder_finish()` returns `GTD_ERR_ANNOTATIONS_OOB` when
 * any annotation falls outside the nav fix time range.  Calling this
 * function downgrades that error to a warning and lets the build succeed.
 *
 * Must be called before the first `gtd_builder_add_*` call.
 *
 * @param b Builder handle.  No-op if NULL.
 */
void gtd_builder_set_lenient(GtdFileBuilder *b);

/**
 * Add a GPS navigation fix.
 *
 * At least one nav fix is required before `gtd_builder_finish()`.
 * Fixes must be added in ascending time order.
 *
 * @param b           Builder handle.
 * @param gps_time    GPS time of the fix.  Use `gtd_ts_none()` when unavailable.
 * @param sys_time    System (wall-clock) time.  Use `gtd_ts_none()` when unavailable.
 * @param lat_deg     WGS-84 latitude in degrees.
 * @param lon_deg     WGS-84 longitude in degrees.
 * @param heading_deg Compass heading in degrees [0, 360), or `GTD_NONE_F64`.
 * @param speed_mps   Ground speed in m/s, or `GTD_NONE_F64`.
 * @param eph_m       Estimated horizontal position error in metres, or `GTD_NONE_F64`.
 */
GtdStatus gtd_builder_add_nav_fix(GtdFileBuilder *b, GtdTimestamp gps_time, GtdTimestamp sys_time,
                                  double lat_deg, double lon_deg, GtdOptF64 heading_deg,
                                  GtdOptF64 speed_mps, GtdOptF64 eph_m);

/**
 * Add a satellite visibility report.
 *
 * The report is associated with the nearest preceding nav fix.
 * Passing @p n_sats as zero with a NULL @p sats pointer records an empty report.
 *
 * @param b        Builder handle.
 * @param gps_time GPS time of the report.  Use `gtd_ts_none()` when unavailable.
 * @param sys_time System (wall-clock) time of the report.
 * @param sats     Array of @p n_sats satellite entries.
 * @param n_sats   Number of elements in @p sats.
 */
GtdStatus gtd_builder_add_satellite_report(GtdFileBuilder *b, GtdTimestamp gps_time,
                                           GtdTimestamp sys_time, const GtdSatellite *sats,
                                           size_t n_sats);

/**
 * Add a legacy map-pin annotation (optional label + icon).
 *
 * @p time must lie within the nav fix time range unless lenient mode is enabled.
 *
 * @param b     Builder handle.
 * @param time  Timestamp of the annotation.  Must not be `gtd_ts_none()`.
 * @param label Human-readable label, or NULL for no label.
 * @param icon  Icon to display.  `GTD_ICON_AUTO` uses the application default (Pin).
 */
GtdStatus gtd_builder_add_annotation(GtdFileBuilder *b, GtdTimestamp time, const char *label,
                                     GtdMarkerIcon icon);

/**
 * Add a structured event marker.
 *
 * Event markers use a hierarchical variant path (e.g. `"system/startup"`)
 * to identify the event type.  Paths must be non-empty, consist of
 * alphanumeric segments separated by `/`, and not exceed 256 bytes.
 *
 * @param b            Builder handle.
 * @param variant_path Hierarchical event type path.
 * @param sys_time     Time of the event.  Must not be `gtd_ts_none()`.
 * @param annotation   Optional human-readable text.  Pass NULL for none.
 *
 * @return `GTD_ERR_INVALID_PATH` if @p variant_path is malformed.
 */
GtdStatus gtd_builder_add_event_marker(GtdFileBuilder *b, const char *variant_path,
                                       GtdTimestamp sys_time, const char *annotation);

/**
 * Register a display style for an event marker variant.
 *
 * Styles are per-variant, not per-event.  Calling this multiple times for
 * the same path overwrites the previous style.
 *
 * @param b            Builder handle.
 * @param variant_path Hierarchical event type path (same format as in
 * `gtd_builder_add_event_marker()`).
 * @param icon         Icon to display.  `GTD_ICON_AUTO` uses the application default.
 * @param color_hex    Color as an `"#RRGGBB"` string, or NULL for automatic.
 */
GtdStatus gtd_builder_add_event_marker_style(GtdFileBuilder *b, const char *variant_path,
                                             GtdMarkerIcon icon, const char *color_hex);

/**
 * Finalise the builder and produce a `GtdNavFile` handle.
 *
 * The builder is **consumed** by this call regardless of success or failure.
 * Do not call `gtd_builder_destroy()` afterwards.
 *
 * On success, `*out` is set to the new handle.
 * On failure, `*out` is set to NULL and `gtd_last_error()` describes the error.
 *
 * @param b   Builder to finalise.
 * @param out Output parameter for the resulting file handle.
 *
 * @return `GTD_ERR_NO_NAV_FIXES` if no nav fixes were added.
 * @return `GTD_ERR_ANNOTATIONS_OOB` if annotations fall outside the time range (unless lenient).
 */
GtdStatus gtd_builder_finish(GtdFileBuilder *b, GtdNavFile **out);

/** @} */

/**
 * @defgroup navfile_write NavFile output
 * @{
 */

/**
 * Write the navigation file to disk.
 *
 * The `.gtd` extension is appended automatically if @p path has no extension.
 *
 * @param f    File handle (not consumed - caller must still call `gtd_nav_file_destroy()`).
 * @param path Destination file path.
 */
GtdStatus gtd_nav_file_write_to_path(const GtdNavFile *f, const char *path);

/**
 * Serialise the navigation file into a heap-allocated byte buffer.
 *
 * On success, `*buf` points to a buffer of `*len` bytes that the caller must
 * free with `gtd_free_bytes(*buf, *len)`.
 *
 * @param f   File handle (not consumed).
 * @param buf Output: pointer to the allocated buffer.
 * @param len Output: number of bytes in the buffer.
 */
GtdStatus gtd_nav_file_to_bytes(const GtdNavFile *f, uint8_t **buf, size_t *len);

/**
 * Free a byte buffer returned by `gtd_nav_file_to_bytes()`.
 *
 * @p buf and @p len must match the values written by `gtd_nav_file_to_bytes()`.
 * No-op if @p buf is NULL.
 *
 * @param buf Pointer to the buffer.
 * @param len Number of bytes in the buffer.
 */
void gtd_free_bytes(uint8_t *buf, size_t len);

/**
 * Destroy a navigation file handle and free all associated memory.
 *
 * @param f Handle to destroy.  No-op if NULL.
 */
void gtd_nav_file_destroy(GtdNavFile *f);

/** @} */

/**
 * @defgroup navfile_read NavFile read path
 * @{
 */

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
 * The caller retains ownership of @p data - it may be freed after this call returns.
 *
 * @param data Pointer to the serialised file data.
 * @param len  Length of the data in bytes.
 * @param out  Output parameter for the file handle.
 */
GtdStatus gtd_nav_file_from_bytes(const uint8_t *data, size_t len, GtdNavFile **out);

/**
 * Return the number of navigation fixes in the file.
 *
 * @param f File handle.  Returns 0 if NULL.
 */
size_t gtd_nav_file_nav_point_count(const GtdNavFile *f);

/**
 * Fill @p out with data for the navigation fix at @p idx.
 *
 * @param f   File handle.
 * @param idx Zero-based index.  Must be less than `gtd_nav_file_nav_point_count(f)`.
 * @param out Caller-allocated struct to fill.
 *
 * @return `GTD_ERR_NULL_ARGUMENT` if @p idx is out of range.
 */
GtdStatus gtd_nav_file_get_nav_point(const GtdNavFile *f, size_t idx, GtdNavPointInfo *out);

/**
 * Fill @p out with satellite data for a specific satellite within a nav fix.
 *
 * @param f       File handle.
 * @param nav_idx Nav fix index.
 * @param sat_idx Satellite index within that fix.  Must be less than `GtdNavPointInfo::sat_count`.
 * @param out     Caller-allocated struct to fill.
 *
 * @return `GTD_ERR_NULL_ARGUMENT` if either index is out of range, or the nav fix has no satellite
 * report.
 */
GtdStatus gtd_nav_file_get_satellite(const GtdNavFile *f, size_t nav_idx, size_t sat_idx,
                                     GtdSatInfo *out);

/**
 * Return the file title, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p f.
 */
const char *gtd_nav_file_title(const GtdNavFile *f);

/**
 * Return the recording device name, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p f.
 */
const char *gtd_nav_file_device(const GtdNavFile *f);

/**
 * Return the notes string, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p f.
 */
const char *gtd_nav_file_notes(const GtdNavFile *f);

/**
 * Return the identity string, or NULL if not set.
 *
 * The returned pointer is valid for the lifetime of @p f.
 */
const char *gtd_nav_file_identity(const GtdNavFile *f);

/**
 * Return the number of event markers in the file.
 *
 * @param f File handle.  Returns 0 if NULL.
 */
size_t gtd_nav_file_event_marker_count(const GtdNavFile *f);

/**
 * Fill @p out with data for the event marker at @p idx.
 *
 * @param f   File handle.
 * @param idx Zero-based index.  Must be less than `gtd_nav_file_event_marker_count(f)`.
 * @param out Caller-allocated struct to fill.
 *
 * @return `GTD_ERR_NULL_ARGUMENT` if @p idx is out of range.
 */
GtdStatus gtd_nav_file_get_event_marker(const GtdNavFile *f, size_t idx, GtdEventMarkerInfo *out);

/** @} */

#ifdef __cplusplus
}
#endif

#endif /* GEOTRACE_H */
