//! SDK for producing and reading `.nvd` naview data files.
//!
//! # Quick start
//!
//! ```no_run
//! use naview_sdk::{NavFileBuilder, NavFix, Angle, degree};
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut builder = NavFileBuilder::new();
//!
//! builder.add_nav_fix(
//!     NavFix::builder()
//!         .gps_time(chrono::Utc::now())
//!         .lat(Angle::new::<degree>(51.5))
//!         .lon(Angle::new::<degree>(-0.1))
//!         .heading(Angle::new::<degree>(270.0))
//!         .build(),
//! );
//!
//! let nav_file = builder.finish()?;
//! nav_file.write_to_file("output")?;
//! # Ok(())
//! # }
//! ```

mod builder;
mod error;
mod read;
mod time_types;
mod types;
mod write;

// Re-export public API
pub use builder::NavFileBuilder;
pub use error::{BuildError, Error};
pub use types::{
    Annotation, Constellation, Marker, MarkerIcon, Meta, NavFile, NavFix, NavPoint, Satellite,
    SatelliteReport,
};

// Re-export commonly needed external types so users don't need extra deps
pub use chrono::{DateTime, Duration, Utc};
pub use uom::si::angle::degree;
pub use uom::si::f64::{Angle, Velocity};
pub use uom::si::velocity::meter_per_second;
