//! Python bindings for `geotrace-sdk`.
//!
//! Exposes the core read/write API as a Python extension module named
//! `geotrace_sdk._geotrace_sdk`.  The public `geotrace_sdk` package re-exports
//! everything from this module via `python/geotrace_sdk/__init__.py`.

use pyo3::prelude::*;
use pyo3_log::{Caching, Logger};

use crate::navigation::{
    PySatelliteIdentity, PySatelliteObservation, PySatelliteObservationReport, PySatelliteScope,
    PySatelliteUsage, PySatelliteUsageReport,
};

pub use builder::PyNavFileBuilder;
pub use channel::{PyChannel, PyChannelUnit, PyUnit};
pub use collections::{
    PyChannelIterator, PyChannelSequence, PyEventMarkerPointIterator, PyEventMarkerPointSequence,
    PyEventMarkerStyleIterator, PyEventMarkerStyleSequence, PyMarkerIterator, PyMarkerSequence,
    PyNavPointIterator, PyNavPointSequence,
};
pub use enums::{PyConstellation, PyMarkerIcon, PyTravelMode};
pub use markers::{PyAnnotation, PyEventMarker, PyEventMarkerPoint, PyEventMarkerStyle, PyMarker};
pub use metadata::PyMeta;
pub use nav_file::PyNavFile;
pub use nav_fix::PyNavFix;
pub use nav_point::PyNavPoint;
pub use satellite::{PySatellite, PySatelliteReport};

mod builder;
mod channel;
mod collections;
mod enums;
mod errors;
mod markers;
mod metadata;
mod nav_file;
mod nav_fix;
mod nav_point;
mod navigation;
mod python_enum;
mod satellite;
mod timestamps;

#[pymodule]
fn _geotrace_sdk(m: &Bound<'_, PyModule>) -> PyResult<()> {
    install_python_logging_bridge(m.py());

    m.add_class::<PySatellite>()?;
    m.add_class::<PySatelliteReport>()?;
    m.add_class::<PyUnit>()?;
    m.add_class::<PyChannelUnit>()?;
    m.add_class::<PyChannel>()?;
    m.add_class::<PyNavFix>()?;
    m.add_class::<PySatelliteIdentity>()?;
    m.add_class::<PySatelliteScope>()?;
    m.add_class::<PySatelliteUsage>()?;
    m.add_class::<PySatelliteObservation>()?;
    m.add_class::<PySatelliteUsageReport>()?;
    m.add_class::<PySatelliteObservationReport>()?;

    m.add_class::<PyAnnotation>()?;
    m.add_class::<PyMeta>()?;
    m.add_class::<PyNavPoint>()?;
    m.add_class::<PyMarker>()?;
    m.add_class::<PyEventMarker>()?;
    m.add_class::<PyEventMarkerStyle>()?;
    m.add_class::<PyEventMarkerPoint>()?;
    m.add_class::<PyNavPointSequence>()?;
    m.add_class::<PyNavPointIterator>()?;
    m.add_class::<PyMarkerSequence>()?;
    m.add_class::<PyMarkerIterator>()?;
    m.add_class::<PyEventMarkerPointSequence>()?;
    m.add_class::<PyEventMarkerPointIterator>()?;
    m.add_class::<PyChannelSequence>()?;
    m.add_class::<PyChannelIterator>()?;
    m.add_class::<PyEventMarkerStyleSequence>()?;
    m.add_class::<PyEventMarkerStyleIterator>()?;
    m.add_class::<PyNavFile>()?;
    m.add_class::<PyNavFileBuilder>()?;
    m.add_function(wrap_pyfunction!(enums::constellation_from_name, m)?)?;
    m.add_function(wrap_pyfunction!(enums::marker_icon_from_name, m)?)?;
    m.add_function(wrap_pyfunction!(channel::mps_from_kmh, m)?)?;
    m.add_function(wrap_pyfunction!(channel::mps_from_knots, m)?)?;
    m.add_function(wrap_pyfunction!(channel::kmh_from_mps, m)?)?;
    m.add_function(wrap_pyfunction!(channel::knots_from_mps, m)?)?;
    m.add_function(wrap_pyfunction!(satellite::snr_is_no_data_sentinel, m)?)?;
    m.add_function(wrap_pyfunction!(enums::rust_sdk_enum_members, m)?)?;
    Ok(())
}

fn install_python_logging_bridge(py: Python<'_>) {
    let installed = Logger::new(py, Caching::Loggers)
        .map_err(|e| e.to_string())
        .and_then(|logger| logger.install().map_err(|e| e.to_string()));
    if let Err(reason) = installed {
        log::warn!("the sdk's log records stay off python's logging: {reason}");
    }
}
