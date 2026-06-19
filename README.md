# GeoTrace

> [!WARNING]
> **Pre-Alpha Status:** This project is in early development. It is highly experimental, prone to bugs or breaking changes, and **not yet ready for general usage**.

GeoTrace is a desktop application for visualizing GPS/GNSS navigation data.
It reads `.gtd` recording files and renders Time-Position-Velocity traces on an interactive map with satellite signal quality overlays, event markers, and filtering tools.

![GeoTrace showing the demo trip: a ride along the Paris quays with per-fix satellite counts, custom and event markers, a tunnel fix-loss rendered in red, and the satellite metrics plot below](tests/snapshots/snap_app_demo_trip.png)

*(offline map) A short ride with multi-constellation satellite data, custom and event markers, and a 59-second tunnel fix-loss followed by gradual signal reacquisition.
The screenshot is the UI snapshot test baseline (with map tiles off), so it always matches the current build.*

## Features

- Drag-and-drop `.gtd` file loading
- Interactive map with OpenStreetMap and satellite basemaps
- TPV trace visualization with dynamic label density based on zoom level
- Per-satellite SNR and constellation quality display
- Event marker overlay with typed variant paths
- Track filtering by time range, duration, and position
- Recording history with import and re-load support

## Install

> [!NOTE]
> Installers and packages are published with each tagged release.
> Until the first release is tagged, build from source with `just run`.

GeoTrace is a desktop app for Linux, macOS, and Windows (x86-64 and ARM64).

Linux / macOS (shell):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/CramBL/geotrace/releases/latest/download/geotrace-installer.sh | sh
```

Windows (PowerShell):

```powershell
irm https://github.com/CramBL/geotrace/releases/latest/download/geotrace-installer.ps1 | iex
```

Homebrew (Linux / macOS):

```sh
brew install CramBL/homebrew-tap/geotrace
```

Windows installer: download the `.msi` for your architecture from the [latest release](https://github.com/CramBL/geotrace/releases/latest).

GeoTrace checks for a newer release on startup and offers to update.
Installs from the shell/PowerShell installer update in place; any other install is pointed at the downloads page.
The check can be turned off under Settings.

> [!NOTE]
> Releases are currently **unsigned**.
> On macOS, the first launch needs right-click &rarr; **Open** (or `xattr -d com.apple.quarantine /path/to/geotrace`).
> On Windows, SmartScreen shows **More info &rarr; Run anyway**.

## SDKs

> [!NOTE]
> SDK packages are published on their own release cadence (separate from the app), starting with the first SDK release.

GeoTrace defines an open binary format (`.gtd`) for storing navigation recordings.
SDKs are available so you can produce `.gtd` files from any data source and open them in GeoTrace.

All SDKs are MIT licensed and can be embedded in proprietary applications.

### Rust SDK

```sh
cargo add geotrace-sdk
```

```rust
use geotrace_sdk::{Angle, DateTime, NavFileBuilder, NavFix, Utc};

let t = "2024-01-15T09:00:00Z".parse::<DateTime<Utc>>()?;
let mut sink = NavFileBuilder::new().open();
sink.add_nav_fix(
    NavFix::builder()
        .gps_time(t)
        .lat(Angle::degrees(51.5074))
        .lon(Angle::degrees(-0.1278))
        .build(),
);
let nav_file = sink.finish()?;
nav_file.write_to_file("track.gtd")?;
```

Examples: [`sdk/rust/geotrace-sdk/examples/`](sdk/rust/geotrace-sdk/examples/)

### C SDK

A C API wrapping the core encoder/decoder, suitable for embedding in any language with a C FFI.
See [`sdk/c/geotrace.h`](sdk/c/geotrace.h) for the full API surface.
Building or consuming it via CMake requires CMake 3.21+.
On Windows the released library is built with MSVC.

Homebrew (Linux / macOS), installs headers, the static library, and the CMake package config:

```sh
brew install CramBL/homebrew-tap/geotrace-c
```

**find_package** (from a local build, a Homebrew install, or a prebuilt release archive):

```cmake
find_package(GeoTraceC REQUIRED)
target_link_libraries(my_target PRIVATE GeoTrace::C)
```

**FetchContent** (from a release archive URL — no Rust toolchain required):

```cmake
include(FetchContent)
FetchContent_Declare(geotrace_c
    URL     https://github.com/CramBL/geotrace/releases/download/geotrace-sdk-v0.1.0/geotrace-sdk-x86_64-unknown-linux-gnu.tar.gz
    URL_HASH SHA256=<hash>)
FetchContent_MakeAvailable(geotrace_c)
list(APPEND CMAKE_PREFIX_PATH "${geotrace_c_SOURCE_DIR}")
find_package(GeoTraceC REQUIRED)
target_link_libraries(my_target PRIVATE GeoTrace::C)
```

Replace the URL and hash with those from the [Releases](../../releases) page.
The archive contains the same relocatable install tree produced by `cmake --install`,
so `find_package` resolves it correctly after adding the extracted root to `CMAKE_PREFIX_PATH`.

### C++ SDK

A header-only C++17 wrapper around the C SDK with RAII types and range-based iteration.
See [`sdk/cpp/include/`](sdk/cpp/include/).
Building or consuming it via CMake requires CMake 3.21+.

### Python SDK

```python
from datetime import UTC, datetime
from geotrace_sdk import NavFileBuilder, NavFix

sink = NavFileBuilder()
sink.add(NavFix(lat=51.5074, lon=-0.1278, gps_time=datetime(2024, 1, 15, 9, 0, 0, tzinfo=UTC)))
sink.finish().write_to_file("track.gtd")
```

Install with uv: `uv add geotrace-sdk`

Or with pip: `pip install geotrace-sdk`

## Development

Use [`just`](https://just.systems) to run common tasks:

```
just --list
```

Before committing, run `just ci` and fix any errors or warnings.

See [`CODE_STYLE.md`](CODE_STYLE.md) and [`DESIGN.md`](DESIGN.md) for project conventions.

## License

GeoTrace uses a split-licensing model.

- **Core application and internal crates** — AGPL-3.0.
  See [`LICENSE`](./LICENSE).
- **SDKs** (`geotrace-sdk`, C SDK, C++ SDK, Python SDK) — MIT.
  See [`sdk/rust/geotrace-sdk/LICENSE`](sdk/rust/geotrace-sdk/LICENSE).
  The SDKs can be embedded in proprietary applications without triggering the AGPL terms of the core application.
