# geotrace-sdk-units

Canonical channel units shared by the GeoTrace file SDK and query engine.

A channel declares an optional `ChannelUnit`, which is one of three kinds:

- **Recognized** - a `Unit` from the catalog in this crate.
  It has a physical quantity and a factor to that quantity's base unit, so a query can compare the channel against a literal written in any unit of the same quantity.
- **Custom** - a label GeoTrace stores and displays verbatim.
  Its values stay dimensionless in queries, because a conversion cannot be inferred from a label alone.
- **Legacy** - a label read from a file that is neither of those, kept byte for byte and never accepted as writer input.

## Attaching a unit to a channel you write

Name a catalog unit through one of the `Unit` constants, or call `ChannelUnit::custom` for a label the catalog does not cover.

```rust
use geotrace_sdk_units::{ChannelUnit, Unit};

let acceleration = ChannelUnit::recognized(Unit::MG);
assert_eq!(acceleration.to_string(), "mg");

let score = ChannelUnit::custom("vendor score")?;
assert!(score.as_recognized().is_none());
```

Parsing a string yields a recognized unit and nothing else: `"rpm".parse::<ChannelUnit>()` fails with `UnitParseError::Unrecognized`, the signal to call `ChannelUnit::custom` when a display-only label is what you meant.

## Reading a unit back

`ChannelUnit::from_file_label` accepts every label a file can hold.
It resolves aliases such as `degrees`, `kph`, `m/s²` and `µg` to catalog units, keeps any other single-line label as custom, and preserves the rest as legacy.
`ChannelUnit::is_writable` is false for exactly the legacy case, so a tool that rewrites a file learns which labels it cannot declare again.

## What queries do with each kind

Values stay in their declared scale when stored and plotted.
A query converts a recognized channel to its quantity's base unit, compares it there, and formats each result back to the declared scale.
The base units are degrees, meters, meters per second, meters per second squared, seconds, the unit fraction for a ratio (`100 %` is `1.0`), and per minute for a rate.
Custom and legacy values are read as plain numbers, so a unit literal cannot be compared against them.

The catalog also covers prefixed sensor scales (`mm`, `ug`, `cm/s2`) that a query parses but the editor leaves out of its suggestions.
`Unit::CANONICAL` is the suggested subset, `Unit::recognized()` is everything.

This crate is MIT licensed independently of the AGPL GeoTrace application.
