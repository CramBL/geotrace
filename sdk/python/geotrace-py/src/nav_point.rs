use chrono::{DateTime, FixedOffset};
use geotrace_sdk::NavPoint;
use pyo3::prelude::*;

use crate::nav_fix::PyNavFix;
use crate::navigation::{
    PyPositionProvenance, PySatelliteObservationReport, PySatelliteUsageReport,
    PySolutionDimension, PySolutionMethod, PySolutionState,
};
use crate::satellite::PySatelliteReport;

/// A navigation fix with all associated independent satellite reports.
///
/// The value ranges of `NavFix` apply here too, as expectations.
/// The SDK returns `lat` and `lon` unchanged, NaN included.
/// Optional floating-point measurements, including DOP, return `None` for an on-disk NaN.
/// Checking a value against its range is the caller's job.
/// Equality compares all fix and satellite facts, treating intentional missing positions as absent.
/// Two NaN coordinates require explicit state and unreported origin for this comparison.
/// Other NaN values retain ordinary inequality.
#[pyclass(frozen, skip_from_py_object, name = "NavPoint")]
#[derive(Clone, Debug)]
pub struct PyNavPoint {
    inner: NavPoint,
}

#[pymethods]
impl PyNavPoint {
    /// Latitude in degrees, expected in [-90, 90].
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.fix.lat.as_degrees()
    }

    /// Longitude in degrees, expected in [-180, 180].
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.fix.lon.as_degrees()
    }

    /// GPS-receiver timestamp (timezone-aware UTC), or `None` when unreported.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.fix.gps_time().map(|time| time.fixed_offset())
    }

    /// System-clock timestamp (timezone-aware UTC), or `None` when the recorder
    /// did not supply one.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.fix.sys_time().map(|time| time.fixed_offset())
    }

    /// Heading in degrees, expected in [0, 360), or `None`.
    #[getter]
    fn heading(&self) -> Option<f64> {
        self.inner.fix.heading.map(|h| h.as_degrees())
    }

    /// Speed in m/s, expected to be non-negative, or `None`.
    #[getter]
    fn speed_mps(&self) -> Option<f64> {
        self.inner.fix.speed.map(|s| s.as_meters_per_second())
    }

    /// Estimated horizontal accuracy radius in metres, expected to be
    /// non-negative, or `None`.
    #[getter]
    fn eph_m(&self) -> Option<f64> {
        self.inner.fix.eph_m
    }

    /// Reported state, or `None` when unreported.
    /// Explicit `SolutionState.UNKNOWN` blocks fallback.
    #[getter]
    fn state(&self) -> PyResult<Option<PySolutionState>> {
        self.inner.fix.state.map(TryInto::try_into).transpose()
    }

    /// Reported solution method, independent of state, or `None` when unreported.
    #[getter]
    fn method(&self) -> PyResult<Option<PySolutionMethod>> {
        self.inner.fix.method.map(TryInto::try_into).transpose()
    }

    /// Reported solution dimension, independent of state, or `None` when unreported.
    #[getter]
    fn dimension(&self) -> PyResult<Option<PySolutionDimension>> {
        self.inner.fix.dimension.map(TryInto::try_into).transpose()
    }

    /// Reported global used count, independent of identity rows, or `None` when unreported.
    #[getter]
    fn used_count(&self) -> Option<u32> {
        self.inner.fix.used_count
    }

    /// Dimensionless horizontal dilution of precision, or `None` when unreported.
    #[getter]
    fn hdop(&self) -> Option<f64> {
        self.inner.fix.hdop
    }

    /// Dimensionless vertical dilution of precision, or `None` when unreported.
    #[getter]
    fn vdop(&self) -> Option<f64> {
        self.inner.fix.vdop
    }

    /// Dimensionless position dilution of precision, or `None` when unreported.
    #[getter]
    fn pdop(&self) -> Option<f64> {
        self.inner.fix.pdop
    }

    /// Coordinate origin, or `None` when unreported.
    #[getter]
    fn position_provenance(&self) -> PyResult<Option<PyPositionProvenance>> {
        self.inner
            .fix
            .position_provenance
            .map(TryInto::try_into)
            .transpose()
    }

    /// Owned usage samples in input order, including every associated report.
    #[getter]
    fn usage_reports(&self) -> Vec<PySatelliteUsageReport> {
        self.inner
            .satellite_evidence()
            .usage
            .iter()
            .cloned()
            .map(|inner| PySatelliteUsageReport { inner })
            .collect()
    }

    /// Owned observation samples in input order, including every associated report.
    #[getter]
    fn observation_reports(&self) -> Vec<PySatelliteObservationReport> {
        self.inner
            .satellite_evidence()
            .observations
            .iter()
            .cloned()
            .map(|inner| PySatelliteObservationReport { inner })
            .collect()
    }

    /// Lossless combined projection, or `None` for independent evidence without that projection.
    #[getter]
    fn satellites(&self) -> Option<PySatelliteReport> {
        self.inner
            .satellites()
            .clone()
            .map(|s| PySatelliteReport { inner: s })
    }

    fn __eq__(&self, other: &Self) -> bool {
        PyNavFix {
            inner: self.inner.fix,
        }
        .equals_recorded_fix(other.inner.fix)
            && self.inner.satellite_evidence() == other.inner.satellite_evidence()
            && self.inner.satellites() == other.inner.satellites()
    }

    fn __repr__(&self) -> String {
        format!(
            "NavPoint(lat={:.6}, lon={:.6})",
            self.inner.fix.lat.as_degrees(),
            self.inner.fix.lon.as_degrees(),
        )
    }
}

impl From<&NavPoint> for PyNavPoint {
    fn from(point: &NavPoint) -> Self {
        Self {
            inner: point.clone(),
        }
    }
}
