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

## Examples

- [**from_csv.rs**](examples/from_csv.rs) - Convert GPS data exported as CSV rows into a `.gtd` file.
- [**from_multiple_sources.rs**](examples/from_multiple_sources.rs) - Aggregate a GPS track and event annotations from separate sources into a single `.gtd` file.

## License

This SDK is licensed under the **MIT License**.

You are free to use, modify, and distribute this SDK in both open-source and closed-source commercial projects.
Using this crate to generate data files does not subject your application to the AGPL license used by the main GeoTrace application.

See the [LICENSE](./LICENSE) file for details.
