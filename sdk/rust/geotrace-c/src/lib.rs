//! C FFI layer for the GeoTrace SDK.
//!
//! This crate is a `cdylib`/`staticlib` - its public surface is the C header
//! `sdk/c/geotrace.h`. Do not add Rust public API here.

#![expect(
    unsafe_code,
    reason = "FFI crate - all extern C functions require unsafe"
)]

#[macro_use]
mod macros;

mod builder;
mod channel;
mod constellation;
pub(crate) mod error;
mod icon;
mod nav_file;
mod optf64;
mod satellite;
mod satinfo;
mod timestamp;
mod travel_mode;

pub use builder::GtdFileBuilder;
pub use channel::GtdChannel;
pub use constellation::GtdConstellation;
pub use error::GtdStatus;
pub use icon::GtdMarkerIcon;
pub use nav_file::{GtdChannelInfo, GtdEventMarkerInfo, GtdNavFile, GtdNavPointInfo};
pub use optf64::GtdOptF64;
pub use satellite::GtdSatellite;
pub use satinfo::GtdSatInfo;
pub use timestamp::GtdTimestamp;
pub use travel_mode::GtdTravelMode;
