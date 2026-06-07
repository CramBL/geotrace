# GeoTrace

> [!WARNING]
> **Pre-Alpha Status:** This project is in early development. It is highly experimental, prone to bugs or breaking changes, and **not yet ready for general usage**.

GeoTrace is a desktop application for visualizing GPS/GNSS navigation data.
It reads `.gtd` recording files and renders Time-Position-Velocity traces on an interactive map with satellite signal quality overlays, event markers, and filtering tools.

## Features

- Drag-and-drop `.gtd` file loading
- Interactive map with OpenStreetMap and satellite basemaps
- TPV trace visualization with dynamic label density based on zoom level
- Per-satellite SNR and constellation quality display
- Event marker overlay with typed variant paths
- Track filtering by time range, duration, and position
- Recording history with import and re-load support

## SDKs

GeoTrace defines an open binary format (`.gtd`) for storing navigation recordings.
SDKs are available so you can produce `.gtd` files from any data source and open them in GeoTrace.

All SDKs are MIT licensed and can be embedded in proprietary applications.

### Rust SDK

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

### C++ SDK

A header-only C++17 wrapper around the C SDK with RAII types and range-based iteration.
See [`sdk/cpp/include/`](sdk/cpp/include/).

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
