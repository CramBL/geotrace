# C SDK

A C99 API over the `.gtd` encoder/decoder. MIT licensed.
Full API surface: [`sdk/c/geotrace.h`](../../sdk/c/geotrace.h).
Consuming via CMake requires CMake 3.21+. On Windows the released library is
built with MSVC.

## Install

The release archive and the Homebrew formula install the same relocatable tree:
headers, the static library, and the `GeoTraceC` CMake package config. The
[C++ SDK](cpp.md) ships in the same archive. The released libraries are static.

### Homebrew (Linux, macOS)

```sh
brew install CramBL/homebrew-tap/geotrace-c
```

### Release archive (Linux, macOS, Windows)

Download the archive for your target from the [releases page](https://github.com/CramBL/geotrace/releases),
extract it, and add the extracted root to `CMAKE_PREFIX_PATH`.

| Target | Archive |
| --- | --- |
| Linux x86-64 | `geotrace-sdk-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `geotrace-sdk-aarch64-unknown-linux-gnu.tar.gz` |
| macOS x86-64 | `geotrace-sdk-x86_64-apple-darwin.tar.gz` |
| macOS ARM64 | `geotrace-sdk-aarch64-apple-darwin.tar.gz` |
| Windows x64 | `geotrace-sdk-x86_64-pc-windows-msvc.zip` |
| Windows ARM64 | `geotrace-sdk-aarch64-pc-windows-msvc.zip` |

## Consume

```cmake
find_package(GeoTraceC REQUIRED)
target_link_libraries(my_target PRIVATE GeoTrace::C)
```

From an archive URL with FetchContent (no Rust toolchain). Replace the URL and
hash with those from the [releases page](https://github.com/CramBL/geotrace/releases):

```cmake
include(FetchContent)
FetchContent_Declare(geotrace_c
    URL      https://github.com/CramBL/geotrace/releases/download/geotrace-sdk-v0.7.0/geotrace-sdk-x86_64-unknown-linux-gnu.tar.gz
    URL_HASH SHA256=<hash>)
FetchContent_MakeAvailable(geotrace_c)
list(APPEND CMAKE_PREFIX_PATH "${geotrace_c_SOURCE_DIR}")
find_package(GeoTraceC REQUIRED)
target_link_libraries(my_target PRIVATE GeoTrace::C)
```

On Windows, configure with `-A x64` or `-A ARM64`.

### Static linking

The released archives and the Homebrew formula are static, so `find_package`
yields a static `GeoTrace::C`. On Windows it is always static. The static library
links the system libraries it needs (`ntdll`, `userenv`, `ws2_32`, `advapi32`,
`bcrypt`) through the package config. For a source build, select static with
`-DGEOTRACE_C_STATIC=ON` (see below).

## Build from source

Requires a Rust toolchain.
`cargo build` writes both the static and the shared library.
`-DGEOTRACE_C_STATIC=ON` selects the static one for the `GeoTrace::C` target and for `cmake --install`.
Without it, Linux and macOS use the shared library.
Windows always uses the static library.

```sh
cargo build -p geotrace-c --release
cmake -S sdk/c -B build -DGEOTRACE_C_STATIC=ON -DCMAKE_INSTALL_PREFIX=prefix
cmake --install build
```

## Logging

The SDK logs a record for input data that it cannot use as written, such as a satellite SNR of 99 dB-Hz, an unknown travel mode in a file, or an annotation that lenient mode clamps.
The function that logs the record returns `GTD_OK`.
`gtd_set_log_callback()` registers one callback for the whole process, and the SDK passes it every record at `GTD_LOG_WARN` and above.
`gtd_set_log_level()` sets that level, and `gtd_clear_log_callback()` removes the callback.

```c
static void print_log_record(GtdLogLevel level, const char *target, const char *message,
                             void *user_data) {
    (void)level;
    (void)user_data;
    fprintf(stderr, "%s: %s\n", target, message);
}

GtdStatus status = gtd_set_log_callback(print_log_record, NULL);
if (status != GTD_OK) {
    fprintf(stderr, "set_log_callback: %s\n", gtd_last_error());
}
```

The callback runs on the thread that wrote the record.
The `target` and `message` pointers are valid only for the duration of the call.

## Examples

In [`sdk/c/examples/`](../../sdk/c/examples/). Configure the C SDK with
`-DGEOTRACE_C_BUILD_EXAMPLES=ON` to build them.

- [`write_basic`](../../sdk/c/examples/write_basic.c): minimal write workflow: add fixes, finish, write to disk.
- [`read_file`](../../sdk/c/examples/read_file.c): open a `.gtd` file and print a content summary.
- [`with_satellites`](../../sdk/c/examples/with_satellites.c): pair each fix with a per-satellite signal report.
- [`event_markers`](../../sdk/c/examples/event_markers.c): write and read hierarchical event markers (`variant_path`).
- [`channels`](../../sdk/c/examples/channels.c): attach scalar and vector sensor channels and read them back.
- [`from_csv`](../../sdk/c/examples/from_csv.c): convert CSV rows into a `.gtd` file.
- [`from_multiple_sources`](../../sdk/c/examples/from_multiple_sources.c): merge fixes and events from separate sources, interpolating event positions.
- [`gold_dataset`](../../sdk/c/examples/gold_dataset.c): build the cross-SDK reference file from shared fixtures, write it, and check the metadata and the counts of the built file handle.
