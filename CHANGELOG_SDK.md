# SDK Changelog

Notable changes to the GeoTrace SDK - the `.gtd` format libraries for Rust, C,
C++, and Python.
The SDK versions independently of the GeoTrace application (see CHANGELOG.md for
the app).

## [unreleased]

### Added

- Rust `NavFileBuilder::with_scrubbed_provenance()`. A file written through it holds the new `geotrace_sdk::SCRUBBED_SDK_VERSION` (`<scrubbed>`) as its `sdk_version`, no `sdk_git_commit` and no `sdk_commit_time`, whatever the build that wrote it.
- Rust `NavFile::equals_ignoring_build_provenance()`, which compares two files over everything but their `sdk_version`, `sdk_git_commit` and `sdk_commit_time`.

### Changed

- The writer takes `sdk_version`, `sdk_git_commit` and `sdk_commit_time` from the `NavFile` it writes: a file read from disk and written back keeps the stamp it was read with, and one read without a stamp is written without one. `NavRecorder::finish` stamps the build it runs in.
- C `gtd_builder_add_channel_with_unit_mode` takes `uint32_t unit_mode`, the parameter type `gtd_channel_unit_parse` already uses. A `GtdChannelUnitMode` value passes unchanged.

## [0.6.0] - 2026-09-03

### Added

- The `sdk_version`, `sdk_git_commit` and `sdk_commit_time` file attributes, which a released SDK build stamps on the files it writes. They read back through Rust `Meta::sdk_version()`, `Meta::sdk_git_commit()` and `Meta::sdk_commit_time()`, C `gtd_nav_file_sdk_version()`, `gtd_nav_file_sdk_git_commit()` and `gtd_nav_file_sdk_commit_time()`, the same three as C++ `NavFile` methods, and the Python `Meta.sdk_version`, `Meta.sdk_git_commit` and `Meta.sdk_commit_time` properties.
- The `nav_points/gps_time_us` dataset, holding each fix's GPS-receiver timestamp in microseconds since the Unix epoch, `u64::MAX` where the fix has none, which Rust `NavFix::gps_time` reads as `None`. A file written before this dataset existed reads its `time` axis as the receiver's timestamp.
- Python `logging` receives the SDK's diagnostics, such as a satellite report dropped for having no timestamp, on the `geotrace_sdk` logger and its per-module children.

### Changed

- A string longer than the `.gtd` field that holds it is rejected where it is built or written: an event marker variant path past 255 bytes or annotation past 511 bytes, a marker label past 255 bytes, and an event marker style variant path or color past its field. Rust `EventMarker::builder().build()` and `Annotation::builder().build()` return `Result`, C returns the new `GTD_ERR_FIELD_TOO_LONG` (11) from `gtd_builder_add_event_marker`, `gtd_builder_add_annotation`, `gtd_nav_file_write_to_path` and `gtd_nav_file_to_bytes`, C++ throws the new `geotrace::FieldTooLongError`, and Python raises `ValueError`. `GTD_ERR_INVALID_PATH` covers only a malformed variant path now.
- Rust `Annotation` has the accessors `label()`, `icon()` and `time()` in place of its public fields, so `Annotation::builder().build()` is the only way to construct one. The `markers/label` dataset no longer has a `truncated` attribute.
- Rust `EventMarkerIconChoice` and `EventMarkerColor` each have a new `Unrecognized(String)` variant, which the reader produces for an `icon_name` outside the `MarkerIcon` set and for a `color_hex` that is not `#RRGGBB`, and which the writer writes back verbatim. `EventMarkerIconChoice::wire_name` returns the `icon_name` wire value the choice writes, and `EventMarkerIconChoice` is no longer `Copy`.
- Python `NavFile.event_marker_styles` raises a `UserWarning` for a style naming an icon outside the `MarkerIcon` set, whose `icon` reads as `None` and whose new read-only `EventMarkerStyle.icon_name` holds the stored name. `EventMarkerStyle.color` reads back a color that is not `#RRGGBB` verbatim. `NavFileBuilder.add_event_marker_style` writes such a name back unchanged and raises `ValueError` for such a color.
- Rust `Unit::to_base` bases a rate on per second: `Unit::PER_S.to_base()` is `1.0`, `Unit::PER_MIN.to_base()` is `1/60`, and `Unit::PER_H.to_base()` is `1/3600`.
- Updated the Python bindings' `pyo3` to 0.29, which fixes RUSTSEC-2026-0176 and RUSTSEC-2026-0177.

### Fixed

- Fixed the reader allocating for a dataset's declared size before reading it: a file declaring more data than its own byte length can hold is rejected with an error naming the dataset.
- Fixed the reader replacing the invalid bytes of a marker label, event marker variant path or annotation, or event marker style icon name or color hex with U+FFFD: reading a file whose field is not UTF-8 now fails with an error naming the group and dataset, in all four SDKs.

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
