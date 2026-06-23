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
- [`event_markers_flat`](../../sdk/python/geotrace-py/examples/event_markers_flat.py): typed markers via `@event_kind`, single-level paths.
- [`event_markers_nested`](../../sdk/python/geotrace-py/examples/event_markers_nested.py): typed markers via `@event_kind`, nested paths.
- [`event_markers_skip`](../../sdk/python/geotrace-py/examples/event_markers_skip.py): omit variants with `event_kind.skip`.
- [`from_csv`](../../sdk/python/geotrace-py/examples/from_csv.py): convert CSV rows into a `.gtd` file.
- [`from_multiple_sources`](../../sdk/python/geotrace-py/examples/from_multiple_sources.py): merge fixes and events from separate sources, interpolating event positions.
- [`gold_dataset`](../../sdk/python/geotrace-py/examples/gold_dataset.py): build the cross-SDK reference file from shared fixtures and verify the round-trip.
