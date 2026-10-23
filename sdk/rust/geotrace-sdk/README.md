# geotrace-sdk

The official Rust SDK for generating and reading `.gtd` data files compatible with GeoTrace.

```rust
use geotrace_sdk::{Angle, NavFileBuilder, NavFix, NavFixTime, Utc};

let mut recorder = NavFileBuilder::new().open();
// add() dispatches by type: pass a NavFix, SatelliteReport, Annotation, or EventMarker.
recorder.add(
    NavFix::builder()
        .time(NavFixTime::Receiver(Utc::now()))
        .lat(Angle::degrees(51.5074))
        .lon(Angle::degrees(-0.1278))
        .heading(Angle::degrees(90.0))
        .build(),
);
let nav_file = recorder.finish()?;
nav_file.write_to_file("track.gtd")?;
```

## Channel units

Use a recognized `Unit` so GeoTrace can dimension-check and convert channel values in queries.
Values stay in their declared scale for storage and plotting, so a channel declared as `Unit::MG` should contain and display milli-g values.

```rust
use geotrace_sdk::{Channel, ChannelUnit, Unit};

let acceleration = Channel::builder()
    .name("accel")
    .unit(Unit::MG)
    .times(times.clone())
    .values(milli_g_values)
    .build()?;

let vendor_measurement = Channel::builder()
    .name("quality")
    .unit(ChannelUnit::custom("vendor score")?)
    .times(times)
    .values(scores)
    .build()?;
```

Custom units are preserved for display but treated as dimensionless because GeoTrace has no safe conversion rule for them.
Unknown labels in existing files are matched against recognized aliases first and otherwise preserved as custom units.

## NMEA ingestion

Enable the optional `nmea` feature to convert NMEA input with `NmeaIngestor`.
Configure sentence periods and optional UTC timestamp prefixes, then inspect the diagnostic summary alongside the recording.
For incremental input, use `push_line()` or `push_line_at()` and submit the remaining records from `finish()`.

```rust
use std::io::Cursor;
use std::time::Duration;

use geotrace_sdk::{NavFileBuilder, NmeaIngestor, SentenceType};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let source = Cursor::new(concat!(
    "2026-10-23 120000.000 ",
    "$GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A*52\n",
));
let result = NmeaIngestor::builder()
    .timestamp_prefix_utc("%Y-%m-%d %H%M%S%.f")
    .sentence_period(SentenceType::RMC, Duration::from_secs(1))
    .build()?
    .ingest_reader(source, NavFileBuilder::new())?;
let mut gtd = Vec::new();
result.recording.write(&mut gtd)?;
assert_eq!(result.recording.nav_points().len(), 1);
assert!(result.diagnostics.samples().is_empty());
# Ok(())
# }
```

## Examples

Runnable examples are in
[`sdk/rust/geotrace-sdk/examples`](https://github.com/CramBL/geotrace/tree/trunk/sdk/rust/geotrace-sdk/examples).
Run one with `cargo run -p geotrace-sdk --example <name>`.

The list, with a line on each:
[`docs/sdk/rust.md`](https://github.com/CramBL/geotrace/blob/trunk/docs/sdk/rust.md).

## License

This SDK is licensed under the **MIT License**.

You are free to use, modify, and distribute this SDK in both open-source and closed-source commercial projects.
Using this crate to generate data files does not subject your application to the AGPL license used by the main GeoTrace application.

See the [LICENSE](./LICENSE) file for details.
