# C++ SDK

A header-only C++17 wrapper over the `.gtd` C SDK, with RAII types and
range-based iteration. MIT licensed.
Headers: [`sdk/cpp/include/`](../../sdk/cpp/include/).
Consuming via CMake requires CMake 3.21+. On Windows the released library is
built with MSVC.

## Install

The release archive and the Homebrew formula install the C++ headers and the
`GeoTraceCpp` CMake package config alongside the C library they wrap. The
released libraries are static.

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
find_package(GeoTraceCpp REQUIRED)
target_link_libraries(my_target PRIVATE GeoTrace::Cpp)
```

On Windows, configure with `-A x64` or `-A ARM64`.

### Static linking

`GeoTrace::Cpp` links the C library transitively, so its linkage follows the C
SDK: static in the released archives and the Homebrew formula, and always static
on Windows. The static C library links the system libraries it needs (`ntdll`,
`userenv`, `ws2_32`, `advapi32`, `bcrypt`) through the package config. For a
source build, select static with `-DGEOTRACE_C_STATIC=ON` (see below).

## Build from source

Requires a Rust toolchain. Install the C SDK, then the C++ SDK into the same
prefix. `-DGEOTRACE_C_STATIC=ON` builds the static library (the default on
Windows). Omit it for a shared library on Linux and macOS.

```sh
cargo build -p geotrace-c --release
cmake -S sdk/c -B build/c -DGEOTRACE_C_STATIC=ON -DCMAKE_INSTALL_PREFIX=prefix
cmake --install build/c
cmake -S sdk/cpp -B build/cpp -DCMAKE_PREFIX_PATH=prefix -DCMAKE_INSTALL_PREFIX=prefix
cmake --install build/cpp
```

## Examples

In [`sdk/cpp/examples/`](../../sdk/cpp/examples/). Configure the C++ SDK with
`-DGEOTRACE_CPP_BUILD_EXAMPLES=ON` to build them.

- [`write_basic`](../../sdk/cpp/examples/write_basic.cpp): minimal write workflow: add fixes, finish, write to disk.
- [`read_file`](../../sdk/cpp/examples/read_file.cpp): open a `.gtd` file and print a content summary.
- [`with_satellites`](../../sdk/cpp/examples/with_satellites.cpp): pair each fix with a per-satellite signal report.
- [`event_markers`](../../sdk/cpp/examples/event_markers.cpp): write and read hierarchical event markers (`variant_path`).
- [`channels`](../../sdk/cpp/examples/channels.cpp): attach scalar and vector sensor channels and read them back.
- [`typed_events`](../../sdk/cpp/examples/typed_events.cpp): type-safe markers via `enum class` and `EventEnum<>` specialization.
- [`from_csv`](../../sdk/cpp/examples/from_csv.cpp): convert CSV rows into a `.gtd` file.
- [`from_multiple_sources`](../../sdk/cpp/examples/from_multiple_sources.cpp): merge fixes and events from separate sources, interpolating event positions.
- [`gold_dataset`](../../sdk/cpp/examples/gold_dataset.cpp): build the cross-SDK reference file from shared fixtures and verify the round-trip.
