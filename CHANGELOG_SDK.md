# SDK Changelog

Notable changes to the GeoTrace SDK - the `.gtd` format libraries for Rust, C,
C++, and Python.
The SDK versions independently of the GeoTrace application (see CHANGELOG.md for
the app).

## [unreleased]

### Changed

- Updated `hdf5-pure` to 0.27.0.

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
