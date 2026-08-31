# SDK Changelog

Notable changes to the GeoTrace SDK - the `.gtd` format libraries for Rust, C,
C++, and Python.
The SDK versions independently of the GeoTrace application (see CHANGELOG.md for
the app).

## [unreleased]

### Added

- Rust `EventMarkerIconChoice::wire_name` returns the `icon_name` wire value the choice writes: the empty name for `Auto`, the icon's name for `Icon`, and the stored name for `Unrecognized`.
- Python `EventMarkerStyle.icon_name`, a read-only property holding the stored name of `icon`: `None` where the style leaves the icon to the application, and the name verbatim where it is outside the `MarkerIcon` set. `NavFileBuilder.add_event_marker_style` writes such a name back unchanged.

### Changed

- `geotrace/geotrace.hpp` uses an include guard instead of `#pragma once`, matching `geotrace/unit_catalog.hpp`.
- Rust `EventMarkerIconChoice` and `EventMarkerColor` each have a new `Unrecognized(String)` variant, which the reader produces for an `icon_name` outside the `MarkerIcon` set and for a `color_hex` that is not `#RRGGBB`, and which the writer writes back verbatim. `EventMarkerIconChoice` is no longer `Copy`.
- Python `NavFile.event_marker_styles` raises a `UserWarning` for a style naming an icon outside the `MarkerIcon` set, whose `icon` reads as `None`. `EventMarkerStyle.color` reads back a color that is not `#RRGGBB` verbatim, and `NavFileBuilder.add_event_marker_style` raises `ValueError` for such a color.
- An event marker whose variant path is longer than the 255 bytes that field holds is rejected where it is built: Rust `EventMarker::builder().build()`, C `gtd_builder_add_event_marker`, C++ `FileBuilder::add_event_marker`, and the Python `EventMarker` constructor.
- Rust `EventMarker::builder().build()`, C `gtd_builder_add_event_marker`, and C++ `FileBuilder::add_event_marker` reject an annotation longer than the 511 bytes that field holds.
- A marker label longer than the 255 bytes that field holds is rejected where it is built: Rust `Annotation::builder().build()` returns `Result`, C `gtd_builder_add_annotation` returns `GTD_ERR_FIELD_TOO_LONG`, C++ `FileBuilder::add_annotation` throws `geotrace::FieldTooLongError`, and the Python `Annotation` constructor raises `ValueError`.
- Rust `Annotation` has the accessors `label()`, `icon()` and `time()` in place of its public fields, so `Annotation::builder().build()` is the only way to construct one.
- The `markers/label` dataset no longer has a `truncated` attribute.
- C returns the new `GTD_ERR_FIELD_TOO_LONG` (11) where a string is longer than the `.gtd` field that holds it: an event marker variant path past 255 bytes or annotation past 511 bytes at `gtd_builder_add_event_marker`, a marker label past 255 bytes at `gtd_builder_add_annotation`, and an event marker style variant path or color past its field at `gtd_nav_file_write_to_path` and `gtd_nav_file_to_bytes`. `GTD_ERR_INVALID_PATH` covers only a malformed variant path now. C++ throws the matching new `geotrace::FieldTooLongError`, which derives `geotrace::Error`.

### Fixed

- The C gold example rejects a date or time number too large for an `int` instead of reading it with `sscanf`, which cannot report the overflow.
- Fixed the reader reading an event marker style's icon name outside the `MarkerIcon` set, and a color that is not `#RRGGBB`, as the automatic style: both values now survive the read.
- Fixed the writer cutting a marker label, event marker variant path, annotation, icon name, or color hex mid-character to fit its field: writing a file that holds such a value now fails with an error naming the field, in all four SDKs.
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
