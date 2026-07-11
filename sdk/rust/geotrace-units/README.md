# geotrace-units

Canonical channel units shared by the GeoTrace file SDK and query engine.

Recognized units carry physical dimensions and conversion factors.
Custom units are an explicit display-only escape hatch and remain dimensionless in queries.

```rust
use geotrace_units::{ChannelUnit, Unit};

let acceleration = ChannelUnit::recognized(Unit::MG);
assert_eq!(acceleration.to_string(), "mg");

let score = ChannelUnit::custom("vendor score")?;
assert!(score.as_recognized().is_none());
```

Values remain in their declared scale when stored and plotted.
Query evaluation converts recognized units to physical base units internally and converts displayed channel results back to the declared scale.
Unknown labels from existing files are parsed as recognized aliases where possible and otherwise preserved losslessly.

This crate is MIT licensed independently of the AGPL GeoTrace application.
