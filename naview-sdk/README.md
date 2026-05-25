# naview-sdk

The official Rust SDK for generating and reading `.nvd` data files compatible with the Naview ecosystem.

```rust
use naview_sdk::{Angle, NavFileBuilder, NavFix, Utc, degree};

let mut builder = NavFileBuilder::new();
builder.add_nav_fix(
    NavFix::builder()
        .time("2024-01-15T09:00:00Z".parse::<DateTime<Utc>>()?)
        .lat(Angle::new::<degree>(51.5074))
        .lon(Angle::new::<degree>(-0.1278))
        .heading(Angle::new::<degree>(90.0))
        .build(),
);
let nav_file = builder.finish()?;
nav_file.write_to_file("track.nvd")?;
```

## Examples

- [**from_csv.rs**](examples/from_csv.rs) — Convert GPS data exported as CSV rows into a `.nvd` file.
- [**from_multiple_sources.rs**](examples/from_multiple_sources.rs) — Aggregate a GPS track and event annotations from separate sources into a single `.nvd` file.

## License

This SDK is licensed under the **MIT License**.

You are free to use, modify, and distribute this SDK in both open-source and closed-source commercial projects.
Using this crate to generate data files does not subject your application to the AGPL license used by the main Naview application.

See the [LICENSE](./LICENSE) file for details.
