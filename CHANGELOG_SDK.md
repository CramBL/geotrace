# SDK Changelog

Notable changes to the GeoTrace SDK - the `.gtd` format libraries for Rust, C,
C++, and Python.
The SDK versions independently of the GeoTrace application (see CHANGELOG.md for
the app).

## [unreleased]

### Added

- Add support for NavIC and QZSS constellations.

## [0.3.0] - 2026-06-24

### Added

- C++ SDK: a non-throwing `try_*` API returning `Result<T>`/`Status` by value
  alongside the throwing methods, so the SDK works with exceptions disabled
  (`-fno-exceptions`, or `GEOTRACE_CPP_NO_EXCEPTIONS`).
- C SDK: a distinct `GTD_ERR_PARSE` status for malformed or corrupt `.gtd` file
  content, and a matching C++ `ParseError` exception.
- Python SDK: `Meta.identity` is now settable and readable.
- Cross-SDK conformance check: every SDK must decode the gold-dataset fixtures to
  the same `NavFile` (run by `just test-gold-all`).
- Fuzzing of the `.gtd` decoder.

### Changed

- Decode failures are no longer reported as internal/I/O errors: the C SDK maps
  them to `GTD_ERR_PARSE`, and the Python SDK raises `ValueError`.
- Updated `hdf5-pure` to 0.19.0.

### Fixed

- C and C++ gold examples now parse fractional-second timestamps correctly.

## [0.2.0] - 2026-06-23

### Added

- C and C++ SDKs distributed as prebuilt, relocatable CMake install archives for
  Linux, macOS, and Windows (MSVC), plus a Homebrew formula (`geotrace-c`).
- Python SDK published to PyPI as abi3 wheels - a single `cp312-abi3` wheel per
  platform, covering CPython 3.12 and later - alongside a source distribution.

## [0.1.0] - 2026-06-21

Initial release: the Rust `geotrace-sdk` and `geotrace-sdk-macros` crates on
crates.io.
