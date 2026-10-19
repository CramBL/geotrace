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

## Navigation metadata encoding

New writes use GTD version 3.
Readers accept versions 1, 2 and 3.
Versions 1 and 2 decode the new navigation fields as unreported.
All version 3 navigation columns have one element per navigation point and a one-dimensional shape.
The decoder requires the declared numeric class, width and precision, with either standard byte order.

| `nav_points` column | Numeric type | Encoding |
|---|---|---|
| `state` | `u8` | 0 absent, 1 Fix, 2 NoFix, 3 EstimatedOnly, 4 Unknown |
| `method` | `u8` | 0 absent, 1 Autonomous, 2 Differential, 3 RtkFloat, 4 RtkFixed, 5 DeadReckoning, 6 Hybrid, 7 Other |
| `dimension` | `u8` | 0 absent, 1 TwoD, 2 ThreeD |
| `position_provenance` | `u8` | 0 absent, 1 SourceProvided, 2 SdkSynthesized |
| `used_count` | `u32` | Global source-reported used count, including zero |
| `used_count_present` | `u8` | 0 absent, 1 present |
| `hdop`, `vdop`, `pdop` | `f64` | Dimensionless values, NaN absent |

An absent count has `used_count_present=0` and a zero payload.
Readers ignore that payload when presence is zero.
Optional floating-point values decode NaN as absent, matching heading, speed and accuracy.
Other numeric values retain their recorded scale.
DOP and horizontal accuracy are independent fields.

`NavFix::builder()` defaults `position_provenance` to `Some(PositionProvenance::SourceProvided)`.
Use `.position_provenance(None)` to record unreported coordinate origin.
SDK-generated positions have `Some(PositionProvenance::SdkSynthesized)`, including positions with an interpolated heading.
Heading and timestamp origins are independent of navigation state and coordinate origin.
`SolutionMethod` and `SolutionDimension` describe source facts without changing state.
`SolutionState::EstimatedOnly` means an estimate without measured GNSS contribution.
A hybrid solution with GNSS contribution uses `SolutionState::Fix`.
