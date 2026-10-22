# Python SDK

`geotrace-sdk` reads and writes `.gtd` files. MIT licensed.
Requires Python 3.12+. Distributed as `abi3` wheels (one `cp312-abi3` wheel per
platform, forward-compatible with newer CPython).

## Install

```sh
uv add geotrace-sdk
```

Or

```sh
pip install geotrace-sdk
```

Prebuilt wheels:

- Linux: `manylinux` and `musllinux`, x86-64 / ARM64 / x86 / armv7 / s390x / ppc64le.
- macOS: x86-64, ARM64.
- Windows: x64, x86, ARM64.

A platform without a matching wheel builds from the source distribution, which
requires a Rust toolchain.

## Examples

In [`sdk/python/geotrace-py/examples/`](../../sdk/python/geotrace-py/examples/).
Run with `python <name>.py`.

- [`write_basic`](../../sdk/python/geotrace-py/examples/write_basic.py): minimal write workflow: add fixes, finish, write to disk.
- [`read_file`](../../sdk/python/geotrace-py/examples/read_file.py): read a `.gtd` file and print a content summary.
- [`with_satellites`](../../sdk/python/geotrace-py/examples/with_satellites.py): pair each fix with a satellite visibility report.
- [`with_satellites_and_markers`](../../sdk/python/geotrace-py/examples/with_satellites_and_markers.py): satellite reports plus custom map annotations placed by interpolation.
- [`event_markers`](../../sdk/python/geotrace-py/examples/event_markers.py): write and read hierarchical event markers (`variant_path`).
- [`channels`](../../sdk/python/geotrace-py/examples/channels.py): attach scalar and vector sensor channels and read them back.
- [`event_markers_flat`](../../sdk/python/geotrace-py/examples/event_markers_flat.py): typed markers via `@event_kind`, single-level paths.
- [`event_markers_nested`](../../sdk/python/geotrace-py/examples/event_markers_nested.py): typed markers via `@event_kind`, nested paths.
- [`event_markers_skip`](../../sdk/python/geotrace-py/examples/event_markers_skip.py): omit variants with `event_kind.skip`.
- [`from_csv`](../../sdk/python/geotrace-py/examples/from_csv.py): convert CSV rows into a `.gtd` file.
- [`from_multiple_sources`](../../sdk/python/geotrace-py/examples/from_multiple_sources.py): merge fixes and events from separate sources, interpolating event positions.
- [`gold_dataset`](../../sdk/python/geotrace-py/examples/gold_dataset.py): build the cross-SDK reference file from shared fixtures and verify the round-trip.

## Navigation and independent polling

These examples use the unreleased SDK APIs.
A position without heading can report Fix, eight used satellites and HDOP 0.9 without an observation list.
Add usage identities when membership is available, then independent observations when geometry or signal measurements are available.
The same API accepts facts from text protocols, binary protocols and asynchronous polling.
Counts and DOP remain independent of identity lists and horizontal accuracy.
The following example records host-clock positions and a later poll at its own timestamp.

```python
from datetime import UTC, datetime, timedelta
from geotrace_sdk import (
    Constellation, NavFileBuilder, NavFix, SatelliteCoverage, SatelliteIdentity,
    SatelliteObservation, SatelliteObservationReport, SatelliteScope, SolutionState,
)

start = datetime(2023, 11, 14, 22, 13, 20, tzinfo=UTC)
builder = NavFileBuilder()
for second in [0, 10]:
    builder.add(NavFix(0, second, sys_time=start + timedelta(seconds=second),
        state=SolutionState.FIX, used_count=8, hdop=0.9))
builder.add(SatelliteObservationReport(
    [SatelliteObservation(SatelliteIdentity(Constellation.GPS, 1), snr=40)],
    sys_time=start + timedelta(seconds=2), scope=SatelliteScope.global_(),
    coverage=SatelliteCoverage.PARTIAL, reported_count=12))
builder.finish().write_to_file("navigation_polling.gtd")
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
