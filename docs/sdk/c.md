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

## Navigation and independent polling

These examples use the unreleased SDK APIs.
A position without heading can report Fix, eight used satellites and HDOP 0.9 without an observation list.
Add usage identities when membership is available, then independent observations when geometry or signal measurements are available.
The same API accepts facts from text protocols, binary protocols and asynchronous polling.
Counts and DOP remain independent of identity lists and horizontal accuracy.
The following example records host-clock positions and a later poll at its own timestamp.

```c
#include <geotrace.h>
#include <stdio.h>

int main(void) {
    GtdFileBuilder *builder = gtd_builder_create();
    GtdNavFile *file = NULL;
    GtdNavFixV2 fix = gtd_nav_fix_v2_init();
    fix.state = GTD_SOLUTION_STATE_FIX;
    fix.used_count = GTD_SOME_U32(8);
    fix.hdop = GTD_SOME_F64(0.9);
    if (gtd_ts_from_seconds(1700000000, &fix.sys_time) != GTD_OK ||
        gtd_builder_add_nav_fix_v2(builder, &fix) != GTD_OK) {
        goto fail;
    }
    fix.lon_deg = 10;
    if (gtd_ts_from_seconds(1700000010, &fix.sys_time) != GTD_OK ||
        gtd_builder_add_nav_fix_v2(builder, &fix) != GTD_OK) {
        goto fail;
    }
    GtdSatelliteReportInput poll = {0};
    poll.gps_time = gtd_ts_none();
    poll.scope.kind = GTD_SATELLITE_SCOPE_KIND_GLOBAL;
    poll.coverage = GTD_SATELLITE_COVERAGE_PARTIAL;
    poll.reported_count = GTD_SOME_U32(12);
    GtdSatelliteObservation row = {GTD_CONSTELLATION_GPS, 1, GTD_NONE_F32, GTD_NONE_F32,
                                   GTD_SOME_F32(40)};
    if (gtd_ts_from_seconds(1700000002, &poll.sys_time) != GTD_OK ||
        gtd_builder_add_satellite_observation_report(builder, &poll, &row, 1) != GTD_OK) {
        goto fail;
    }
    GtdStatus status = gtd_builder_finish(builder, &file);
    builder = NULL;
    if (status != GTD_OK || gtd_nav_file_write_to_path(file, "navigation_polling.gtd") != GTD_OK) {
        goto fail;
    }
    gtd_nav_file_destroy(file);
    return 0;
fail:
    fprintf(stderr, "%s\n", gtd_last_error());
    gtd_builder_destroy(builder);
    gtd_nav_file_destroy(file);
    return 1;
}
```

Explicit state, including Unknown, takes precedence over counts and membership.
An absent state permits fallback from eligible used aggregates or known membership.
A known-used identity can establish Fix in a partial report.
Only global evidence can establish NoFix from zero usage.
An omitted aggregate remains unreported.
A present aggregate, including zero, is exact for its declared scope.
Complete coverage describes the identity list within that scope.
A partial list retains omitted identities as unknown.
Each report preserves its original clocks and normalized placement time.
The poll at second 2 becomes a synthesized carrier at longitude 2° between these host-clock positions.

Legacy combined-report accessors return a report only when the independent evidence can be represented without losing membership, scope or timestamps.
Observation-only and partial independent reports remain available through the new report collections.
See the [Rust model and format reference](rust.md#satellite-evidence-values) for the complete coverage and projection rules.

New writers use GTD v3.
The compatible application reader is currently in the Unreleased app.
App 0.19.0 and earlier require GTD v1/v2 files.
Publish the compatible app reader before the synchronized SDK releases, and record its release version in the [release handoff](../../RELEASES.md#gtd-v3-release-order).
