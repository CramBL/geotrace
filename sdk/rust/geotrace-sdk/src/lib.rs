//! SDK for producing and reading `.gtd` GeoTrace data files.
//!
//! # Quick start
//!
//! ```no_run
//! use geotrace_sdk::{Angle, NavFileBuilder, NavFix};
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut sink = NavFileBuilder::new().open();
//!
//! sink.add_nav_fix(
//!     NavFix::builder()
//!         .gps_time(chrono::Utc::now())
//!         .lat(Angle::degrees(51.5))
//!         .lon(Angle::degrees(-0.1))
//!         .heading(Angle::degrees(270.0))
//!         .build(),
//! );
//!
//! let nav_file = sink.finish()?;
//! nav_file.write_to_file("output")?;
//! # Ok(())
//! # }
//! ```

/// The version of this SDK, e.g. `"0.1.0"` (the crate version). Consumers can
/// surface it, for example `println!("geotrace-sdk {}", geotrace_sdk::VERSION)`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod builder;
mod error;
mod read;
mod time_types;
mod types;
mod units;
mod variant_path;
mod write;

// Re-export public API
pub use builder::{NavFileBuilder, NavFileSink, SatelliteWarning, collect_satellite_warnings};
pub use error::{BuildError, Error, EventMarkerError};
pub use types::{
    Annotation, Constellation, EventMarker, EventMarkerColor, EventMarkerIconChoice,
    EventMarkerPoint, EventMarkerStyle, Marker, MarkerIcon, Meta, NavFile, NavFix, NavPoint,
    Satellite, SatelliteReport,
};
pub use units::{Angle, Timestamp, Velocity};
#[doc(hidden)]
pub use variant_path::__private;
pub use variant_path::EventKind;

// Re-export the derive macro
pub use geotrace_sdk_macros::EventKind;

// Re-export commonly needed external types so users don't need extra deps
pub use chrono::{DateTime, Duration, Utc};
