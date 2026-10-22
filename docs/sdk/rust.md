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

## Satellite evidence values

`SatelliteUsageReport` records used, unused or unknown membership for each constellation/PRN identity.
`SatelliteObservationReport` records identities with optional elevation, azimuth and SNR.
Both builders require a timestamp, scope and coverage.
Each report has an optional `reported_count` that is exact for its declared scope, including zero.
The aggregate and identity rows remain independent when they conflict.

`SatelliteScope::Global` declares global scope explicitly.
`SatelliteScope::Constellations` requires a nonempty `ConstellationSet`.
A set of all supported constellations remains distinct from global scope.
Complete usage coverage makes omitted identities known-unused within that scope.
An explicit unknown row remains unknown.
Partial coverage leaves omitted identities unknown.
A complete empty observation report establishes zero seen identities within its scope.
An absent report is unreported evidence.

`NavPoint::satellite_evidence()` returns its canonical collection of independent usage and observation samples.
`NavPoint::satellites()` returns a borrowed optional combined report through the conservative projection.
`NavPoint::new(fix, evidence)` constructs a point with canonical evidence.
`NavPoint::from_legacy(fix, report)` converts an optional combined report.
`NavPoint::set_satellite_evidence(evidence)` replaces the samples and updates the legacy projection.
`NavFile::nav_points_mut()` permits point edits before writing.
`SatelliteEvidence::from(&report)` creates one complete global report of each kind at the original timestamps.
The conversion preserves duplicate rows, measurements and each row's Boolean membership.
Legacy `in_fix=false` becomes known-unused membership.

`SatelliteEvidence::legacy_report()` returns a combined report for one complete global pair with identical recorded timestamps.
Normalized times must also match when placement metadata is present.
Every observed identity requires known membership, and every used identity must be observed.
Reported aggregates must equal the distinct identity counts.
Conflicting duplicate memberships require matching row order in both reports.
The projection preserves the original rows for a converted legacy report.
The method returns `None` for other evidence combinations.

## Independent report ingestion

`NavRecorder::add()` accepts navigation fixes, usage reports and observation reports.
The typed methods are `add_satellite_usage_report()` and `add_satellite_observation_report()`.
The legacy `add_satellite_report()` converts each combined report into complete global usage and observation samples.

Each report associates independently with the nearest submitted fix inside the inclusive association window, which defaults to 500ms.
An equal-distance report associates with the earlier fix.
Several reports can associate with one fix.
Their source timestamps and input order remain available after writing and reading.
Host-only reports compare host timestamps when the candidate fix has a host timestamp.
A receiver-only candidate uses a clock delta from the nearest dual-clock fix, or the raw host time when no delta exists.

`report.placement` records the normalized report time and its zero-based input order across report kinds.
A report at 10.4s associated with a fix at 10s retains its 10.4s normalized time.
Unmatched reports with equal normalized times share one synthesized position.
Their source timestamps remain unchanged.
Interpolation and endpoint placement use the existing coordinate rules.
An observation-only synthesized position has absent navigation state.

## Satellite evidence encoding

Version 3 writers store independent samples in `satellite_usage_reports` and `satellite_observation_reports`.
Each report table has the following one-dimensional columns.

| Column | Numeric type | Encoding |
|---|---|---|
| `nav_point_idx` | `u64` | Zero-based navigation point reference, repeated references permitted |
| `gps_time_us`, `sys_time_us` | `u64` | Original microseconds since Unix epoch, two's-complement signed count, `u64::MAX` absent |
| `scope` | `u32` | 0 global, otherwise constellation-set bit mask |
| `coverage` | `u8` | 0 complete, 1 partial |
| `reported_count` | `u32` | Exact aggregate within the declared scope |
| `reported_count_present` | `u8` | 0 absent, 1 present |
| `normalized_time_us` | `u64` | Common navigation clock, signed microseconds, `u64::MAX` absent |
| `input_order` | `u64` | Zero-based recorder insertion order across report kinds |

The placement columns form an optional pair for earlier version 3 files.
Both absent columns decode as absent placement metadata.
When normalized time is absent, readers ignore the input-order payload.
A converted legacy pair has the same input order in both report tables.

Scope bits use the constellation wire codes: GPS bit 0, GLONASS bit 1, Galileo bit 2, BeiDou bit 3, NavIC bit 4 and QZSS bit 5.
The decoder rejects other bits.
A mask of 63 remains a constellation set.
At least one original timestamp must be present.
An absent count has a zero payload when written.
Readers ignore the payload when presence is zero.

Identity tables are `satellite_usage` and `satellite_observations`.
Both have `report_idx: u64`, `constellation: u8` and `prn: u32` columns.
`satellite_usage/report_idx` references `satellite_usage_reports`.
`satellite_observations/report_idx` references `satellite_observation_reports`.
Usage rows also have `membership: u8`, encoded as 0 unknown, 1 unused and 2 used.
Observation rows have `elevation`, `azimuth` and `snr` as `f32`, with NaN for absence.
Elevation and azimuth are degrees, and SNR is dB-Hz.
Measured zero and other non-NaN values remain present.

Every report and identity column requires the exact numeric class, width and precision, with either standard byte order.
Row order and repeated identities remain unchanged.
A report table and its identity table must both exist, including an empty identity table for empty reports.
An absent report kind has neither table.
Readers also accept the combined `sat_reports` and `tracked_sats` layout from versions 1, 2 and earlier version 3 writers.
Files with both satellite layouts produce an error.

## Navigation and independent polling

These examples use the unreleased SDK APIs.
A position without heading can report Fix, eight used satellites and HDOP 0.9 without an observation list.
Add usage identities when membership is available, then independent observations when geometry or signal measurements are available.
The same API accepts facts from text protocols, binary protocols and asynchronous polling.
Counts and DOP remain independent of identity lists and horizontal accuracy.
The following example records host-clock positions and a later poll at its own timestamp.

```rust
use geotrace_sdk::{Angle, DateTime, Duration, NavFileBuilder, NavFix, NavFixTime,
    SatelliteCoverage, SatelliteObservation, SatelliteIdentity, Constellation,
    SatelliteObservationReport, SatelliteScope, SolutionState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = DateTime::from_timestamp(1_700_000_000, 0).ok_or("timestamp")?;
    let mut recorder = NavFileBuilder::new().open();
    for second in [0, 10] {
        recorder.add(NavFix::builder()
            .time(NavFixTime::Host(start + Duration::seconds(second)))
            .lat(Angle::degrees(0.0)).lon(Angle::degrees(second as f64))
            .state(SolutionState::Fix).used_count(8).hdop(0.9).build());
    }
    recorder.add(SatelliteObservationReport::builder()
        .time(NavFixTime::Host(start + Duration::seconds(2)))
        .scope(SatelliteScope::Global).coverage(SatelliteCoverage::Partial)
        .reported_count(12).satellites(vec![SatelliteObservation::builder()
            .identity(SatelliteIdentity::new(Constellation::Gps, 1)).snr(40.0).build()])
        .build());
    recorder.finish()?.write_to_file("navigation_polling.gtd")?;
    Ok(())
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
