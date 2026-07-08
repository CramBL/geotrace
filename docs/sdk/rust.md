# Rust SDK

`geotrace-sdk` reads and writes `.gtd` files. MIT licensed. Requires Rust 1.89+.

## Install

```sh
cargo add geotrace-sdk
```

## Examples

In [`sdk/rust/geotrace-sdk/examples/`](../../sdk/rust/geotrace-sdk/examples/).
Run with `cargo run -p geotrace-sdk --example <name>`.

- [`write_basic`](../../sdk/rust/geotrace-sdk/examples/write_basic.rs): minimal write workflow: add fixes, finish, write to disk.
- [`read_file`](../../sdk/rust/geotrace-sdk/examples/read_file.rs): open a `.gtd` file and print a content summary.
- [`with_satellites`](../../sdk/rust/geotrace-sdk/examples/with_satellites.rs): pair each fix with a per-satellite signal report.
- [`event_markers`](../../sdk/rust/geotrace-sdk/examples/event_markers.rs): write and read hierarchical event markers (`variant_path`).
- [`event_markers_typed`](../../sdk/rust/geotrace-sdk/examples/event_markers_typed.rs): type-safe markers via `#[derive(EventKind)]`.
- [`channels`](../../sdk/rust/geotrace-sdk/examples/channels.rs): attach scalar and vector sensor channels and read them back.
- [`from_csv`](../../sdk/rust/geotrace-sdk/examples/from_csv.rs): convert CSV rows into a `.gtd` file.
- [`from_domain_types`](../../sdk/rust/geotrace-sdk/examples/from_domain_types.rs): feed your own structs in through `From` impls.
- [`from_multiple_sources`](../../sdk/rust/geotrace-sdk/examples/from_multiple_sources.rs): merge fixes and events from separate sources, interpolating event positions.
- [`gold_dataset`](../../sdk/rust/geotrace-sdk/examples/gold_dataset.rs): build the cross-SDK reference file from shared fixtures and verify the round-trip.
- [`demo_trip`](../../sdk/rust/geotrace-sdk/examples/demo_trip.rs): generate the demo-trip fixture with a 59 s tunnel fix-loss and reacquisition.
