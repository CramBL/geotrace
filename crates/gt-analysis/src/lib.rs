//! Derived satellite-analysis algorithms over loaded GNSS data.
//!
//! This crate is deliberately isolated from the UI, plotting, and rendering
//! crates so each analysis can be audited on its own.  It depends only on the
//! shared domain types in `gt-types` and produces plain data (point series,
//! anomaly lists, slip events) that consumers - the time-series plot and the
//! generated-marker pipeline - turn into mipmaps or markers.
//!
//! - [`satellite_utilization`] - in-fix share of in-view satellites.
//! - [`loss_of_lock`] - cycle-slip detection and slip-rate-per-minute.

pub mod loss_of_lock;
pub mod satellite_utilization;
