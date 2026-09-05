# geotrace-sdk

Python bindings for the GeoTrace SDK: read and write `.gtd` GNSS/GPS navigation
data files.

`.gtd` is the open binary recording format used by
[GeoTrace](https://github.com/CramBL/geotrace), a desktop application for
visualizing GPS/GNSS navigation data: position traces on a map, satellite signal
quality, event markers, and filtering.

MIT licensed. Requires Python 3.12+.

## Install

```sh
uv add geotrace-sdk
```

Or

```sh
pip install geotrace-sdk
```

Distributed as `abi3` wheels (one `cp312-abi3` wheel per platform,
forward-compatible with newer CPython) for Linux (manylinux and musllinux),
macOS, and Windows. A platform without a matching wheel builds from the source
distribution, which requires a Rust toolchain.

## Write a track

```python
from datetime import datetime, timezone
from geotrace_sdk import NavFileBuilder, NavFix

builder = NavFileBuilder()
builder.add(
    NavFix(
        lat=51.5074,
        lon=-0.1278,
        gps_time=datetime(2024, 1, 15, 9, 0, 0, tzinfo=timezone.utc),
        heading=90.0,
    )
)
builder.finish().write_to_file("track.gtd")
```

`add()` also accepts satellite reports, map annotations, and event markers.

## Read a file

```python
from geotrace_sdk import NavFile

nav_file = NavFile.open("track.gtd")
print(len(nav_file.points), "points,", len(nav_file.markers), "markers")
for point in nav_file.points:
    print(point.lat, point.lon, point.gps_time)
```

## Logging

The SDK does not raise when it drops or substitutes data.
It drops a satellite report that has no timestamp, and it keeps an unknown `travel_mode` as it read it.
In both cases the call succeeds and the SDK writes a `logging` record saying what it did.

Each record goes to a logger named after the module that wrote it, such as `geotrace_sdk.builder`.
All of them sit under `geotrace_sdk`, so a single handler there receives every one.

```python
import logging

logging.basicConfig()
logging.getLogger("geotrace_sdk").setLevel(logging.DEBUG)
```

## Examples

Runnable examples are in
[`sdk/python/geotrace-py/examples`](https://github.com/CramBL/geotrace/tree/trunk/sdk/python/geotrace-py/examples):

- [`write_basic`](https://github.com/CramBL/geotrace/blob/trunk/sdk/python/geotrace-py/examples/write_basic.py): minimal write workflow.
- [`read_file`](https://github.com/CramBL/geotrace/blob/trunk/sdk/python/geotrace-py/examples/read_file.py): read a `.gtd` file and print a content summary.
- [`with_satellites`](https://github.com/CramBL/geotrace/blob/trunk/sdk/python/geotrace-py/examples/with_satellites.py): pair each fix with a satellite visibility report.
- [`event_markers`](https://github.com/CramBL/geotrace/blob/trunk/sdk/python/geotrace-py/examples/event_markers.py): write and read hierarchical event markers.
- [`from_csv`](https://github.com/CramBL/geotrace/blob/trunk/sdk/python/geotrace-py/examples/from_csv.py): convert CSV rows into a `.gtd` file.
- [`from_multiple_sources`](https://github.com/CramBL/geotrace/blob/trunk/sdk/python/geotrace-py/examples/from_multiple_sources.py): merge fixes and events from separate sources.
- [`gold_dataset`](https://github.com/CramBL/geotrace/blob/trunk/sdk/python/geotrace-py/examples/gold_dataset.py): build the cross-SDK reference file and verify the round-trip.

Full list and per-platform notes:
[`docs/sdk/python.md`](https://github.com/CramBL/geotrace/blob/trunk/docs/sdk/python.md).

## Links

- Source and issues: <https://github.com/CramBL/geotrace>
- Changelog: <https://github.com/CramBL/geotrace/blob/trunk/CHANGELOG_SDK.md>
