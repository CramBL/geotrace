# SDK Changelog

Notable changes to the GeoTrace SDK - the `.gtd` format libraries for Rust, C,
C++, and Python.
The SDK versions independently of the GeoTrace application (see CHANGELOG.md for
the app).

## [unreleased]

## [0.7.0] - 2026-09-15

### Added

- Every SDK reports whether an SNR reading is the 99 dB-Hz some receiver firmware sends when it has no measurement (`snr_is_no_data_sentinel`).
- Rust `NavFileBuilder::with_scrubbed_provenance()` writes a file without the build stamp of the SDK that wrote it, and `NavFile::equals_ignoring_build_provenance()` compares two files apart from that stamp.
- Rust `Velocity::try_from_knots_str` parses a speed in knots, and `geotrace_sdk_units::MPS_PER_KMH` and `MPS_PER_KNOT` are the factors `Velocity` converts with.
- Rust `Constellation::wire_code` and `MarkerIcon::wire_code` return the code the file stores.
- Rust, Python: `#[event_kind(rename = "<segment>")]` and `event_kind.rename("<segment>")` set a variant path segment other than the derived one.
- C `gtd_nav_file_title_with_length` and its siblings return a metadata value with its byte length, nul bytes included.
- C: Angle conversions between degrees and radians, to the same double as the Rust `Angle`.
- C, C++: Readers for a file's map markers and event marker styles.
- C, C++: A callback that receives the SDK's log records, and a setting for the lowest level forwarded.
- C, C++: Readers for the satellite data warnings the builder raises for a file.
- C, C++: A parser for an ISO 8601 timestamp, such as `2026-02-01T15:00:00+00:00`.
- C, C++, Python: A parser for the lower-case wire name of a constellation or a marker icon, such as `navic` or `satellite_lost`.
- C, C++, Python: A setting for how far a satellite report may be from a nav fix to be associated with it.
- C, Python: Speed conversions between m/s and km/h or knots, to the same double as the Rust `Velocity`.
- C++ `FixTime::from_recorded()` returns `std::nullopt` for a recorder with neither timestamp.
- C++ `constellation_from_code`, `marker_icon_from_code` and `travel_mode_from_code` convert an integer code to the scoped `enum`.
- Python `NavFileBuilder.with_lenient_errors()` clamps an annotation outside the nav fix time range to the nearest fix.
- Python `NavFile.points` returns each field of every fix as a list, such as `latitudes()`.

### Changed

- The writer stamps `geotrace_version` 2, and the reader accepts 1 and 2.
- The writer writes the build stamp the `NavFile` has: a file read and written back keeps its stamp, and `NavRecorder::finish` stamps the build it runs in.
- **Breaking:** Reading a nav point, satellite report or event marker without a timestamp fails with an error stating the record. A nav point or satellite report with either a receiver or a host timestamp reads.
- **Breaking:** A nav fix and a satellite report need a receiver or a host timestamp: Rust and C++ take a required `NavFixTime` and `FixTime`, C returns `GTD_ERR_INVALID_ARGUMENT` and Python raises `ValueError` without either. The builder no longer drops a satellite report without a timestamp.
- **Breaking:** A map marker icon code outside the `MarkerIcon` set is preserved and written back unchanged. Rust reads it as `AnnotationIcon::Unrecognized`, and Python as `Marker.icon_code` with a `UserWarning`.
- **Breaking:** An annotation's icon is Pin unless set, and no SDK takes the automatic icon for an annotation. C++ `MarkerIcon` has no `Auto`, and C++ `EventMarkerStyle::icon` is a `std::optional<MarkerIcon>`.
- A satellite report before the first nav fix produces a ghost fix on the first fix. A ghost fix after the last nav fix takes that fix's position when the fix has no heading.
- **Breaking:** An event marker outside the nav fix time range fails the build. Lenient mode clamps it to the nearest fix and logs a warning.
- **Breaking:** A satellite report or an event marker on a builder with no nav fix fails the build, with an error stating the number of records of each kind.
- **Breaking:** The build fails where a ghost nav fix is past the range a UTC timestamp covers.
- **Breaking:** The builder stores an empty event marker annotation as none.
- Rust `NavFile::inspect` reports a file's metadata and build stamp, event markers and styles, satellite readings, channel details, marker icon codes and fixed-width field rows that are not UTF-8.
- **Breaking:** Rust `NavFileBuilder::with_satellite_window` takes a `std::time::Duration`. A window past `i64::MAX` microseconds associates every satellite report with its nearest nav fix.
- **Breaking:** Rust `Meta` and `EventMarkerStyle` have private fields. `Meta::builder()` and the `NavFileBuilder` metadata setters return a `Result`, and `EventMarkerStyle::builder()` returns the new `EventMarkerStyleError`.
- **Breaking:** Rust `EventMarkerError` reports a malformed variant path through the new `VariantPathError`.
- **Breaking:** Rust `EventMarkerColor` no longer implements `TryFrom<String>`.
- **Breaking:** Rust `EventMarkerStyle::builder()` builds an `Unrecognized` icon with the name of a `MarkerIcon` as that `Icon`, and one with an empty name as `Auto`.
- **Breaking:** Rust, C, C++: A timestamp built from a count of seconds, milliseconds, microseconds or nanoseconds takes a signed 64-bit count and reports an error for a count past the range a timestamp covers.
- **Breaking:** Rust, Python: `#[derive(EventKind)]` and `@event_kind` reject a variant name with a non-ASCII character, a segment past 255 bytes and two variants with the same segment, and the derive rejects an attribute without effect. Both derive `gps3_lock` for `GPS3Lock`, and the derive `type` for `r#type`, where files written by an earlier SDK contain `gp_s3_lock` and `r#type`.
- **Breaking:** C takes an enumeration value as a `uint32_t`, the unit mode of `gtd_builder_add_channel_with_unit_mode` included, and returns `GTD_ERR_INVALID_ARGUMENT`, or `"unknown"` from `gtd_travel_mode_name`, for a value no variant declares. The builder keeps the records it already has, and `gtd_set_log_level` returns a `GtdStatus`.
- **Breaking:** C returns the new `GTD_ERR_OUT_OF_RANGE` for an index past the end, a short output buffer and a timestamp past the range a timestamp covers, and the new `GTD_ERR_CALL_ORDER` for a builder setting made after data. `gtd_builder_set_lenient` returns a `GtdStatus`.
- **Breaking:** C `GtdChannelInfo` points to the channel's strings, and `gtd_nav_file_get_channel` returns each whole, or `GTD_ERR_INVALID_CHANNEL` for one with a nul byte. `gtd_nav_file_get_channel_component` is removed.
- **Breaking:** C, C++: A nav point read from a file states the timestamps of its satellite report.
- **Breaking:** C, C++: A satellite's elevation, azimuth and SNR are optional 32-bit floats, as the file stores them.
- **Breaking:** C++ `Timestamp` is always an instant, with no default constructor, no public constructor from a count and no public field. An absent timestamp is a `std::optional<Timestamp>`.
- **Breaking:** C++ header values are `[[nodiscard]]`, and the value types are `constexpr` apart from the `Timestamp` factories and the unit conversions. `NavFile` has no default constructor.
- **Breaking:** C++ `FileBuilder::lenient()` records its status, an out-of-range accessor throws `std::out_of_range`, and `GTD_ERR_CALL_ORDER` throws the new `CallOrderError`.
- **Breaking:** C++ `FileBuilder` throws `std::invalid_argument` for an enumeration value no enumerator declares, or records `GTD_ERR_INVALID_ARGUMENT` when built without exceptions, and `travel_mode_name` returns `std::nullopt` for one.
- **Breaking:** C++ `NavFile::title` and the other metadata getters return a `std::optional<std::string_view>`: `std::nullopt` for an absent field and an empty view for an empty value.
- **Breaking:** Python `NavFile.points`, `markers`, `event_markers`, `channels` and `event_marker_styles` return a sequence.
- **Breaking:** Python `EventMarker` raises `TypeError` for a `variant_path` that is neither a `str`, `None` nor `event_kind.skip`.
- **Breaking:** Python `Constellation`, `MarkerIcon` and `TravelMode` are `enum.Enum` classes, and a member's `.value` is the code or the name the file stores.
- Updated `hdf5-pure` to 0.46.0.

### Fixed

- **Breaking:** Fixed the reader dropping or misreading a record with an empty variant path, an index past its table or a timestamp out of range, a dataset shorter than its table or unreadable, and a group it cannot open: it fails with an error stating the dataset and the record, or the group.
- **Breaking:** Fixed the reader accepting any `geotrace_version` beginning with a 1 or a 2, such as `10`: it accepts 1 and 2 alone.
- **Breaking:** Fixed a timestamp of exactly 1969-12-31T23:59:59.999999Z being written as absent: writing it fails.
- Fixed an annotation at the last nav fix, and an annotation or event marker between fixes whose receiver and host timestamps disagree on their order, being reported as outside the nav fix time range: the first is placed on that fix, the second between the two fixes whose host timestamps surround its time.
- Fixed a marker, event marker or ghost fix between two fixes on either side of the antimeridian being placed near longitude 0: it is placed on the short arc between them.
- **Breaking:** Fixed the SDK writing a metadata or channel string with a nul byte: it rejects the string with an error stating the byte offset.
- **Breaking:** Fixed the SDK storing an empty or whitespace-only title, device, notes, identity, channel description or marker label as absent: it stores each as given.
- **Breaking:** Fixed the SDK accepting an event marker style with a malformed variant path or a color outside the `#RRGGBB` form, a whitespace-only color included: it rejects the style where it is built. An empty color still gets the hash color.
- **Breaking:** Fixed the builder writing several styles for one event marker variant path: it writes the style of the last call for the path, and writes the styles in variant path order.
- Fixed the `encoding` attribute of the `markers/icon` and `tracked_sats/constellation` datasets listing part of the codes: it lists every code.
- **Breaking:** Fixed Rust `NavRecorder::add_event` writing a variant path that `EventMarker::builder()` rejects: the build fails in strict mode, and lenient mode drops the event and logs an error.
- **Breaking:** Rust: Fixed `NavRecorder` writing an `#[event_kind(icon = ...)]` icon beside a style from `add_event_marker_style` for the same path: it writes the style from `add_event_marker_style` alone, whatever the order of the calls.
- **Breaking:** Rust: Fixed `EventMarkerStyle::builder()` accepting an icon name longer than 31 bytes or with a nul byte: it rejects the name where the style is built.
- **Breaking:** C: Fixed `gtd_builder_finish` leaving `*out` unchanged or the builder allocated on a failure: it sets `*out` to NULL and frees the builder.
- C: Fixed `gtd_nav_file_open` leaving `*out` unchanged on a failure: it sets `*out` to NULL.
- C, C++: Fixed the examples taking a CSV number's decimal separator from `LC_NUMERIC`: they read '.' under every locale and reject a field with trailing characters.
- C++: Fixed a channel string read shortening a value longer than its buffer: it returns the whole value.
- C++: Fixed the metadata getters returning an empty view for a value with a nul byte: they return the whole value.
- **Breaking:** C++: Fixed `FileBuilder` and `ChannelUnit::custom` cutting a string with a nul byte at that byte: they throw an error stating the byte offset.
- **Breaking:** Python: Fixed `EventMarkerStyle` and `EventMarker` raising `ValueError` only when added to the builder: the constructor raises it.
- **Breaking:** Python: Fixed `NavFileBuilder.add_event_marker_style` rejecting a style read from a file with a color outside the `#RRGGBB` form: it writes the style back unchanged.

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
