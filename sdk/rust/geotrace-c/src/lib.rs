//! C FFI layer for the GeoTrace SDK.
//!
//! This crate is a `cdylib`/`staticlib` - its public surface is the C header
//! `sdk/c/geotrace.h`. Do not add Rust public API here.
//!
//! `cbindgen` generates that header from the declarations below, under
//! `cbindgen.toml` beside this crate's manifest, and the declarations reach it
//! in the order the modules are declared here. `just sdk-c-header` rewrites the
//! committed header and `just check-c-header` fails on drift. Every `///` line
//! is Doxygen text for a C reader, `@param` and `@return` included.

#![expect(
    unsafe_code,
    reason = "FFI crate - all extern C functions require unsafe"
)]
// `GtdStatus`, `GtdMarkerIcon`, `GtdChannelUnitMode` and `GtdLogLevel` spell
// their variants as the C constants (`GTD_OK`, `GTD_ICON_PIN`,
// `GTD_CHANNEL_UNIT_CUSTOM`, `GTD_LOG_WARN`), whose prefix differs from the type
// name. `GtdConstellation` and `GtdTravelMode` keep idiomatic variants: their C
// prefix is the type name, which the
// `cbindgen:rename-all=QualifiedScreamingSnakeCase` annotation derives.
#![expect(
    non_camel_case_types,
    reason = "four enums are named for the C constants they declare"
)]

#[macro_use]
mod macros;

mod builder;
mod channel;
mod constellation;
pub(crate) mod error;
mod icon;
mod log_callback;
mod nav_file;
mod optf32;
mod optf64;
mod satellite;
mod satinfo;
mod timestamp;
mod travel_mode;

pub use builder::GtdFileBuilder;
pub use channel::{GtdChannel, GtdChannelUnitMode};
pub use constellation::GtdConstellation;
pub use error::GtdStatus;
pub use icon::GtdMarkerIcon;
pub use log_callback::{GtdLogCallback, GtdLogLevel};
pub use nav_file::{
    GtdChannelInfo, GtdEventMarkerInfo, GtdEventMarkerStyleInfo, GtdMarkerInfo, GtdNavFile,
    GtdNavPointInfo, GtdSatelliteWarningInfo,
};
pub use optf32::GtdOptF32;
pub use optf64::GtdOptF64;
pub use satellite::GtdSatellite;
pub use satinfo::GtdSatInfo;
pub use timestamp::GtdTimestamp;
pub use travel_mode::GtdTravelMode;
