# SDK Changelog

Notable changes to the GeoTrace SDK - the `.gtd` format libraries for Rust, C,
C++, and Python.
The SDK versions independently of the GeoTrace application (see CHANGELOG.md for
the app).

## [unreleased]

### Added

- Rust `geotrace_sdk_units::snr`, holding `NO_DATA_SENTINEL_DB_HZ` (99 dB-Hz), `NO_DATA_SENTINEL_TOLERANCE_DB_HZ` (0.5 dB-Hz) and `is_no_data_sentinel`, the value some receiver firmware sends when it has no measurement, and the band around it. `geotrace_sdk::Satellite::snr_is_no_data_sentinel` reads that band.
- Rust `NavFileBuilder::with_scrubbed_provenance()`. A file written through it holds the new `geotrace_sdk::SCRUBBED_SDK_VERSION` (`<scrubbed>`) as its `sdk_version`, no `sdk_git_commit` and no `sdk_commit_time`, whatever the build that wrote it.
- Rust `NavFile::equals_ignoring_build_provenance()`, which compares two files over everything but their `sdk_version`, `sdk_git_commit` and `sdk_commit_time`.
- C `gtd_nav_file_marker_count`, `gtd_nav_file_get_marker`, `gtd_nav_file_event_marker_style_count` and `gtd_nav_file_get_event_marker_style` read the map markers and the event marker styles a file contains.
- C `gtd_set_log_callback` and `gtd_clear_log_callback` send the SDK's log records to a callback, which receives each record's `GtdLogLevel`, target and message.
- C `gtd_set_log_level` sets the lowest severity the SDK forwards, `GTD_LOG_WARN` until it is called.
- C `gtd_nav_file_satellite_warning_count` and `gtd_nav_file_get_satellite_warning` read the satellite data warnings the builder's checks raise for a file, through the new `GtdSatelliteWarningInfo`.
- C++ `FixTime::from_recorded()`, which takes a `RecordedFixTimestamps` and returns `std::nullopt` when the recorder holds neither timestamp.
- C++ `NavFile::marker_count`, `marker`, `try_marker`, `event_marker_style_count`, `event_marker_style` and `try_event_marker_style` read them through `MarkerView` and `EventMarkerStyleView`.
- C++ `geotrace::set_log_callback`, `try_set_log_callback` and `clear_log_callback` take a `std::function` over the same records, with the level as the new `LogLevel`. `geotrace::set_log_level` sets the lowest severity forwarded.
- C++ `NavFile::satellite_warning_count`, `satellite_warning` and `try_satellite_warning` read them through `SatelliteWarningView`.
- C++ `geotrace::constellation_from_code`, `marker_icon_from_code` and `travel_mode_from_code` convert an integer code to the scoped `enum`, and return `std::nullopt` for a code no enumerator declares.
- Python `NavFileBuilder.with_lenient_errors()` clamps an annotation outside the nav fix time range to the nearest fix, where the build otherwise fails.
- Python `NavFileBuilder.with_satellite_window(timedelta)`, C `gtd_builder_set_satellite_window_us(uint64_t)` and C++ `FileBuilder::satellite_window(std::chrono::microseconds)` set how far a satellite report may be from a nav fix to be associated with it. Python raises `ValueError` and C++ throws `std::invalid_argument` for a negative window.
- Python `NavFile.points` has `latitudes()`, `longitudes()`, `gps_times()`, `sys_times()`, `headings()`, `speeds_mps()` and `eph_m_values()`, each returning that field of every fix as a list.

### Changed

- The writer stamps `geotrace_version` 2 for the layout it writes, and the reader accepts 1 and 2.
- The writer takes `sdk_version`, `sdk_git_commit` and `sdk_commit_time` from the `NavFile` it writes: a file read from disk and written back keeps the stamp it was read with, and one read without a stamp is written without one. `NavRecorder::finish` stamps the build it runs in.
- **Breaking:** Reading a file whose nav point, satellite report or event marker has no timestamp fails with an error stating the record. A nav point and a satellite report each have a receiver timestamp and a host timestamp, and the reader accepts a record with either one.
- **Breaking:** A map marker whose `markers/icon` code is outside the `MarkerIcon` set is preserved and written back unchanged: Rust `Annotation::icon()` returns the new `AnnotationIcon` (`Icon(MarkerIcon)` or `Unrecognized(u8)`), and Python `Marker.icon` and `Annotation.icon` read `None` for it with the new `icon_code` holding the code and a `UserWarning` raised by `NavFile.markers`.
- **Breaking:** An annotation's icon is Pin unless set: Rust `Annotation::icon()` returns a `MarkerIcon`, C `gtd_builder_add_annotation` returns `GTD_ERR_INVALID_ARGUMENT` for `GTD_ICON_AUTO`, C++ `Annotation::icon` defaults to `MarkerIcon::Pin` and `MarkerIcon::Auto` is gone (`EventMarkerStyle::icon` is a `std::optional<MarkerIcon>`), and Python `Annotation.icon` and `Marker.icon` are a `MarkerIcon`.
- A satellite report before the first nav fix produces a ghost fix on the first fix. A ghost fix after the last nav fix takes that fix's position when the fix has no heading.
- **Breaking:** An event marker outside the nav fix time range fails the build with Rust `BuildError::EventMarkersOutsideRange`, C `GTD_ERR_EVENT_MARKERS_OOB` (15), C++ `EventMarkersOutOfRangeError` or a Python `ValueError`, where it was placed on the nearest fix. Lenient mode clamps it to the nearest fix and logs a warning.
- **Breaking:** An event marker on a builder with no nav fix fails the build with the no-nav-fixes error, where it was dropped.
- Rust `NavFile::inspect` reports a file's identity, travel mode and build stamp, its event markers and event marker styles, its satellites' elevation, azimuth and no-data SNR readings, each channel's period, description and time range, every marker icon code it holds, and each fixed-width field row that is not UTF-8.
- **Breaking:** Rust `NavFileBuilder::with_satellite_window` takes a `std::time::Duration`, which cannot be negative. A window longer than `i64::MAX` microseconds associates every satellite report with its nearest nav fix.
- **Breaking:** Rust `NavFix` and `SatelliteReport` cannot be built without a timestamp: each has a new required `time` field, a `NavFixTime` (`Receiver`, `Host`, or `Both`). `gps_time()` and `sys_time()` read that field. The builder no longer drops a satellite report without a timestamp.
- **Breaking:** Rust `NavRecorder::finish` fails with the new `BuildError::GhostFixTimeOutOfRange` where the ghost nav fix for an unassociated satellite report is past the range a UTC timestamp covers.
- **Breaking:** Rust `Timestamp::try_from_unix_seconds`, `try_from_unix_millis`, `try_from_unix_micros` and `try_from_unix_nanos` take an `i64` and return a `Result`, replacing `from_unix_seconds` and its three siblings.
- **Breaking:** C `gtd_set_log_level` takes `uint32_t level` in place of `GtdLogLevel` and returns a `GtdStatus` in place of `void`. It returns `GTD_ERR_INVALID_ARGUMENT` for a value no `GtdLogLevel` variant declares, and the level set before the call stays in force.
- **Breaking:** C `gtd_builder_add_annotation` and `gtd_builder_add_event_marker_style` take `uint32_t icon` in place of `GtdMarkerIcon`. Both return `GTD_ERR_INVALID_ARGUMENT` for a value no `GtdMarkerIcon` variant declares, and `gtd_builder_add_annotation` still returns it for `GTD_ICON_AUTO`.
- **Breaking:** C `GtdSatellite::constellation` is a `uint32_t` in place of a `GtdConstellation`. `gtd_builder_add_satellite_report` returns `GTD_ERR_INVALID_ARGUMENT`, and the builder keeps the reports it already has, for a satellite whose constellation is a value no `GtdConstellation` variant declares.
- **Breaking:** C `gtd_builder_set_travel_mode` and `gtd_travel_mode_name` take `uint32_t mode` in place of `GtdTravelMode`. `gtd_builder_set_travel_mode` returns `GTD_ERR_INVALID_ARGUMENT` and `gtd_travel_mode_name` returns `"unknown"` for a value no `GtdTravelMode` variant declares.
- **Breaking:** C `GtdNavPointInfo` has two new `GtdTimestamp` fields, `sat_report_gps_time` and `sat_report_sys_time`, each `gtd_ts_none()` where the nav point has no satellite report and where the report has no such timestamp. C++ `NavPointView` has the two as `std::optional<Timestamp>`.
- **Breaking:** C `GtdSatellite` and `GtdSatInfo` take a satellite's elevation, azimuth and SNR as the new `GtdOptF32` (`GTD_SOME_F32`, `GTD_NONE_F32`), and C++ `Satellite` and `SatelliteView` as `std::optional<float>`, the 32-bit float the file stores.
- **Breaking:** C `gtd_builder_add_channel_with_unit_mode` takes `uint32_t unit_mode`, the parameter type `gtd_channel_unit_parse` already uses. A `GtdChannelUnitMode` value passes unchanged.
- **Breaking:** C `gtd_builder_add_nav_fix` and `gtd_builder_add_satellite_report` return the new `GTD_ERR_INVALID_ARGUMENT` (12) when `gps_time` and `sys_time` are both `gtd_ts_none()`.
- **Breaking:** C `gtd_builder_finish` returns `GTD_ERR_INVALID_ARGUMENT` where a ghost nav fix is past the range a UTC timestamp covers.
- **Breaking:** C `gtd_nav_file_get_nav_point`, `gtd_nav_file_get_satellite`, `gtd_nav_file_get_event_marker`, `gtd_nav_file_get_channel`, `gtd_nav_file_get_channel_component`, `gtd_nav_file_get_channel_unit` and `gtd_channel_unit_parse` return the new `GTD_ERR_OUT_OF_RANGE` (13) for an index past the end or a short output buffer, where they returned `GTD_ERR_NULL_ARGUMENT`.
- **Breaking:** C `gtd_builder_set_title`, `gtd_builder_set_device`, `gtd_builder_set_notes`, `gtd_builder_set_identity`, `gtd_builder_set_travel_mode` and `gtd_builder_set_lenient` return the new `GTD_ERR_CALL_ORDER` (14) when data has already been added, where the first five returned `GTD_ERR_INTERNAL`. `gtd_builder_set_lenient` returns a `GtdStatus` in place of `void`.
- **Breaking:** C `gtd_ts_from_seconds`, `gtd_ts_from_millis`, `gtd_ts_from_micros` and `gtd_ts_from_nanos` take an `int64_t` count and a `GtdTimestamp` out parameter and return a `GtdStatus`, with `GTD_ERR_OUT_OF_RANGE` for a count past the range a timestamp covers.
- **Breaking:** C++ `NavFix` and `SatelliteReport` have a required `FixTime` member, built with `FixTime::receiver`, `FixTime::host` or `FixTime::both`, in place of their two timestamps.
- **Breaking:** C++ `Timestamp` is always an instant: it has no default constructor, `Timestamp::none()` and `Timestamp::is_none()` are gone, and `NavPointView::gps_time`, `NavPointView::sys_time` and `NavFile::sdk_commit_time()` are `std::optional<Timestamp>`.
- **Breaking:** C++ header values are `[[nodiscard]]`, the value types are `constexpr` apart from the `Timestamp` factories, and `NavFile` has no default constructor.
- **Breaking:** C++ `FileBuilder::lenient()` records its status, an out-of-range accessor throws `std::out_of_range` through the new status, and `GTD_ERR_CALL_ORDER` throws the new `geotrace::CallOrderError`.
- **Breaking:** C++ `Timestamp::try_from_seconds`, `try_from_millis`, `try_from_micros` and `try_from_nanos` return a `Result<Timestamp>`, and `Timestamp::from_seconds` and its siblings take a `std::int64_t` and throw `std::out_of_range` for a count past the range a timestamp covers.
- **Breaking:** C++ `FileBuilder::travel_mode`, `add_satellite_report`, `add_annotation` and `add_event_marker_style` throw `std::invalid_argument` for a `TravelMode`, `Constellation` or `MarkerIcon` value no enumerator declares, where the SDK wrote the platform as `car`, the satellite as GPS and the marker as a pin. Built without exceptions, the builder records `GTD_ERR_INVALID_ARGUMENT` with a message stating the rejected value.
- **Breaking:** C++ `travel_mode_name` returns a `std::optional<std::string_view>`, `std::nullopt` for a value no `TravelMode` enumerator declares.
- **Breaking:** Python `NavFile.points`, `markers`, `event_markers`, `channels` and `event_marker_styles` return a sequence supporting `len()`, indexing, slicing and iteration, in place of a list rebuilt on every attribute access.
- **Breaking:** Python `NavFix` and `SatelliteReport` raise `ValueError` when `gps_time` and `sys_time` are both `None`.
- **Breaking:** Python `EventMarker` raises `TypeError` for a `variant_path` that is neither a `str`, `None` nor `event_kind.skip`, where it read any other value as `None`.
- **Breaking:** Python `Constellation`, `MarkerIcon` and `TravelMode` are `enum.Enum` classes: each member has `.name` and `.value` and works as a `set` element and a `dict` key, and `list()` and `len()` over the class give the members and their count.

### Fixed

- **Breaking:** Fixed the reader dropping an event marker or event marker style with an empty variant path: it now fails with an error stating the dataset and the record.
- **Breaking:** Fixed the reader dropping a tracked satellite or a satellite report whose index points past the table it addresses: it now fails with an error stating the dataset and the record.
- **Breaking:** Fixed the reader reading a timestamp outside the range a UTC timestamp covers as 1970-01-01: it now fails with an error stating the dataset and the record.
- **Breaking:** Fixed the reader treating a dataset it cannot read or a group it cannot open as one the file does not hold: it now fails with an error stating the dataset and the record, or stating the group.
- **Breaking:** Fixed the reader dropping the rows past the end of a dataset shorter than its table: it now fails with an error stating the dataset and the row counts.
- **Breaking:** Fixed a timestamp of exactly 1969-12-31T23:59:59.999999Z being written as absent: writing it fails with an error stating the dataset and the record.
- Fixed an annotation timestamped exactly at the last nav fix being placed outside the nav fix time range: it is placed on that fix.
- Fixed an annotation or event marker inside the nav fix time range being reported as outside it, where a recording's receiver and host timestamps put its fixes in different orders: it is placed between the two fixes whose host timestamps surround its time.
- Fixed a marker, event marker or ghost fix interpolated between two fixes on either side of the antimeridian being placed near longitude 0: it is placed on the short arc between the two fixes.
- **Breaking:** Fixed the reader accepting any `geotrace_version` beginning with a 1 or a 2, such as `10` or `1abc`: it reads the attribute as an integer and accepts 1 and 2 alone.
- Fixed the `encoding` attribute of the `markers/icon` and `tracked_sats/constellation` datasets listing 7 of the 14 marker icons and 4 of the 6 constellations: the writer builds each attribute from the full set of codes.

## [0.6.0] - 2026-09-03

### Added

- The `sdk_version`, `sdk_git_commit` and `sdk_commit_time` file attributes, which a released SDK build stamps on the files it writes. They read back through Rust `Meta::sdk_version()`, `Meta::sdk_git_commit()` and `Meta::sdk_commit_time()`, C `gtd_nav_file_sdk_version()`, `gtd_nav_file_sdk_git_commit()` and `gtd_nav_file_sdk_commit_time()`, the same three as C++ `NavFile` methods, and the Python `Meta.sdk_version`, `Meta.sdk_git_commit` and `Meta.sdk_commit_time` properties.
- The `nav_points/gps_time_us` dataset, holding each fix's GPS-receiver timestamp in microseconds since the Unix epoch, `u64::MAX` where the fix has none, which Rust `NavFix::gps_time` reads as `None`. A file written before this dataset existed reads its `time` axis as the receiver's timestamp.
- Python `logging` receives the SDK's diagnostics, such as a satellite report dropped for having no timestamp, on the `geotrace_sdk` logger and its per-module children.

### Changed

- A string longer than the `.gtd` field that holds it is rejected where it is built or written: an event marker variant path past 255 bytes or annotation past 511 bytes, a marker label past 255 bytes, and an event marker style variant path or color past its field. Rust `EventMarker::builder().build()` and `Annotation::builder().build()` return `Result`, C returns the new `GTD_ERR_FIELD_TOO_LONG` (11) from `gtd_builder_add_event_marker`, `gtd_builder_add_annotation`, `gtd_nav_file_write_to_path` and `gtd_nav_file_to_bytes`, C++ throws the new `geotrace::FieldTooLongError`, and Python raises `ValueError`. `GTD_ERR_INVALID_PATH` covers only a malformed variant path now.
- Rust `Annotation` has the accessors `label()`, `icon()` and `time()` in place of its public fields, so `Annotation::builder().build()` is the only way to construct one. The `markers/label` dataset no longer has a `truncated` attribute.
- Rust `EventMarkerIconChoice` and `EventMarkerColor` each have a new `Unrecognized(String)` variant, which the reader produces for an `icon_name` outside the `MarkerIcon` set and for a `color_hex` that is not `#RRGGBB`, and which the writer writes back verbatim. `EventMarkerIconChoice::wire_name` returns the `icon_name` wire value the choice writes, and `EventMarkerIconChoice` is no longer `Copy`.
- Python `NavFile.event_marker_styles` raises a `UserWarning` for a style with an icon outside the `MarkerIcon` set, whose `icon` reads as `None` and whose new read-only `EventMarkerStyle.icon_name` holds the stored name. `EventMarkerStyle.color` reads back a color that is not `#RRGGBB` verbatim. `NavFileBuilder.add_event_marker_style` writes such a name back unchanged and raises `ValueError` for such a color.
- Rust `Unit::to_base` bases a rate on per second: `Unit::PER_S.to_base()` is `1.0`, `Unit::PER_MIN.to_base()` is `1/60`, and `Unit::PER_H.to_base()` is `1/3600`.
- Updated the Python bindings' `pyo3` to 0.29, which fixes RUSTSEC-2026-0176 and RUSTSEC-2026-0177.

### Fixed

- Fixed the reader allocating for a dataset's declared size before reading it: a file declaring more data than its own byte length can hold is rejected with an error stating the dataset.
- Fixed the reader replacing the invalid bytes of a marker label, event marker variant path or annotation, or event marker style icon name or color hex with U+FFFD: reading a file whose field is not UTF-8 now fails with an error stating the group and dataset, in all four SDKs.

## [0.5.1] - 2026-08-05

### Changed

- Updated `hdf5-pure` to 0.33.0.

## [0.5.0] - 2026-07-16

### Added

- An optional `travel_mode` metadata field declaring the recording platform: `car`, `motorcycle`, `bicycle`, `pedestrian`, `boat`, `rail`, or `aircraft`. Unknown values are preserved on read, never dropped.
- Typed channel units across all four SDKs, including native sensor scales such as milli-g, with a display-only custom-unit escape hatch.

### Changed

- Channel builders take typed units now: Rust `Unit`/`ChannelUnit`, C++ `RecognizedUnit`/`ChannelUnit`, and Python `Unit` constants (recognized strings stay accepted). C keeps the frozen 0.4 struct layouts: `gtd_builder_add_channel` defaults to recognized units, `gtd_builder_add_channel_with_unit_mode` covers custom labels.
- Updated `hdf5-pure` to 0.21.2.

## [0.4.0] - 2026-07-08

### Added

- Support for the NavIC and QZSS constellations.
- Rust channels: attach an ad-hoc sensor time series to a recording with `NavRecorder::add_channel` - own sample timestamps, unit, optional wrap period, and description, scalar or vector with named components stored clock-locked. `NavFile::inspect` lists them.

## [0.3.0] - 2026-06-24

### Added

- C++: a non-throwing `try_*` API returning `Result<T>`/`Status`, so the SDK works with exceptions disabled.
- C: a distinct `GTD_ERR_PARSE` status for malformed `.gtd` content, with a matching C++ `ParseError` exception.
- Python: `Meta.identity` is settable and readable.

### Changed

- Decode failures are no longer reported as internal/I/O errors: C maps them to `GTD_ERR_PARSE`, Python raises `ValueError`.
- Updated `hdf5-pure` to 0.19.0.

### Fixed

- C and C++ gold examples parse fractional-second timestamps correctly.

## [0.2.0] - 2026-06-23

### Added

- C and C++ SDKs distributed as prebuilt, relocatable CMake install archives for Linux, macOS, and Windows (MSVC), plus a Homebrew formula (`geotrace-c`).
- Python SDK published to PyPI as abi3 wheels (one `cp312-abi3` wheel per platform, CPython 3.12 and later) alongside a source distribution.

## [0.1.0] - 2026-06-21

Initial release: the Rust `geotrace-sdk` and `geotrace-sdk-macros` crates on crates.io.
