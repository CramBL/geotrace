use chrono::{DateTime, FixedOffset};
use geotrace_sdk::{Angle, NavFix, Velocity};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::navigation::{
    PyPositionProvenance, PySolutionDimension, PySolutionMethod, PySolutionState,
};
use crate::timestamps::{self, TimestampArguments};

/// A single GPS/GNSS fix: position, optional heading, and optional speed.
///
/// Raises `ValueError` when `gps_time` and `sys_time` are both `None`.
/// All `datetime` arguments must be timezone-aware.
/// `lat` is expected in [-90, 90] degrees, `lon` in [-180, 180], `heading` in
/// [0, 360), `speed_mps` in m/s and `eph_m` in metres, both non-negative.
/// These are data quality expectations, not parse rules.
/// The SDK writes every value it is given, NaN included: a recorder that
/// captured bad data must be able to write it.
/// Optional floating-point fields, including dimensionless DOP, use NaN for absence on disk.
/// A reported NaN in those fields reads back as `None`.
/// Omitted `position_provenance` defaults to source-provided. Explicit `None` means unreported.
/// State, method, dimension and the source used count remain independent facts.
/// Both coordinates may be `None` with an explicit state. Coordinate origin is then unreported.
/// Equality treats two NaN coordinates as absent only with explicit state and unreported origin.
/// Other NaN values retain ordinary inequality.
#[pyclass(frozen, skip_from_py_object, name = "NavFix")]
#[derive(Clone, Copy, Debug)]
pub struct PyNavFix {
    pub(crate) inner: NavFix,
}

#[pymethods]
impl PyNavFix {
    /// Optional metadata records source facts independently of heading and recorded clocks.
    #[expect(
        clippy::too_many_arguments,
        reason = "Python exposes optional metadata as keyword arguments"
    )]
    #[new]
    #[pyo3(signature = (lat, lon, *, gps_time=None, sys_time=None, heading=None, speed_mps=None, eph_m=None, state=None, method=None, dimension=None, used_count=None, hdop=None, vdop=None, pdop=None, position_provenance=Some(PyPositionProvenance::SourceProvided)))]
    fn new(
        lat: Option<f64>,
        lon: Option<f64>,
        gps_time: Option<DateTime<FixedOffset>>,
        sys_time: Option<DateTime<FixedOffset>>,
        heading: Option<f64>,
        speed_mps: Option<f64>,
        eph_m: Option<f64>,
        state: Option<PySolutionState>,
        method: Option<PySolutionMethod>,
        dimension: Option<PySolutionDimension>,
        used_count: Option<u32>,
        hdop: Option<f64>,
        vdop: Option<f64>,
        pdop: Option<f64>,
        position_provenance: Option<PyPositionProvenance>,
    ) -> PyResult<Self> {
        let time =
            timestamps::nav_fix_time_or_value_error(TimestampArguments { gps_time, sys_time })?;
        let mut inner = match (lat, lon) {
            (Some(lat), Some(lon)) => NavFix::builder()
                .time(time)
                .lat(Angle::degrees(lat))
                .lon(Angle::degrees(lon))
                .position_provenance(position_provenance.map(Into::into))
                .build(),
            (None, None) => NavFix::status_only(
                time,
                state
                    .ok_or_else(|| PyValueError::new_err("absent coordinates require state"))?
                    .into(),
            ),
            _ => {
                return Err(PyValueError::new_err(
                    "lat and lon must both be present or both None",
                ));
            }
        };
        inner.heading = heading.map(Angle::degrees);
        inner.speed = speed_mps.map(Velocity::meter_per_second);
        inner.eph_m = eph_m;
        inner.state = state.map(Into::into);
        inner.method = method.map(Into::into);
        inner.dimension = dimension.map(Into::into);
        inner.used_count = used_count;
        inner.hdop = hdop;
        inner.vdop = vdop;
        inner.pdop = pdop;
        Ok(Self { inner })
    }

    /// Latitude in degrees, expected in [-90, 90].
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.lat.as_degrees()
    }

    /// Longitude in degrees, expected in [-180, 180].
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.lon.as_degrees()
    }

    /// GPS-receiver timestamp (timezone-aware UTC), or `None` when unreported.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.gps_time().map(|time| time.fixed_offset())
    }

    /// System-clock timestamp (timezone-aware UTC), or `None` when the recorder
    /// did not supply one.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.sys_time().map(|time| time.fixed_offset())
    }

    /// Heading in degrees, expected in [0, 360), or `None` when unreported.
    #[getter]
    fn heading(&self) -> Option<f64> {
        self.inner.heading.map(|h| h.as_degrees())
    }

    /// Speed in m/s, expected to be non-negative, or `None`.
    #[getter]
    fn speed_mps(&self) -> Option<f64> {
        self.inner.speed.map(|s| s.as_meters_per_second())
    }

    /// Estimated horizontal accuracy radius in metres, expected to be
    /// non-negative, or `None`.
    #[getter]
    fn eph_m(&self) -> Option<f64> {
        self.inner.eph_m
    }

    /// Reported state, or `None` when unreported.
    /// Explicit `SolutionState.UNKNOWN` blocks fallback.
    #[getter]
    fn state(&self) -> PyResult<Option<PySolutionState>> {
        self.inner.state.map(TryInto::try_into).transpose()
    }

    /// Reported solution method, independent of state, or `None` when unreported.
    #[getter]
    fn method(&self) -> PyResult<Option<PySolutionMethod>> {
        self.inner.method.map(TryInto::try_into).transpose()
    }

    /// Reported solution dimension, independent of state, or `None` when unreported.
    #[getter]
    fn dimension(&self) -> PyResult<Option<PySolutionDimension>> {
        self.inner.dimension.map(TryInto::try_into).transpose()
    }

    /// Reported global used count, independent of identity rows, or `None` when unreported.
    #[getter]
    fn used_count(&self) -> Option<u32> {
        self.inner.used_count
    }

    /// Dimensionless horizontal dilution of precision, or `None` when unreported.
    #[getter]
    fn hdop(&self) -> Option<f64> {
        self.inner.hdop
    }

    /// Dimensionless vertical dilution of precision, or `None` when unreported.
    #[getter]
    fn vdop(&self) -> Option<f64> {
        self.inner.vdop
    }

    /// Dimensionless position dilution of precision, or `None` when unreported.
    #[getter]
    fn pdop(&self) -> Option<f64> {
        self.inner.pdop
    }

    /// Coordinate origin, or `None` when unreported.
    #[getter]
    fn position_provenance(&self) -> PyResult<Option<PyPositionProvenance>> {
        self.inner
            .position_provenance
            .map(TryInto::try_into)
            .transpose()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.equals_recorded_fix(other.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "NavFix(lat={:.6}, lon={:.6})",
            self.inner.lat.as_degrees(),
            self.inner.lon.as_degrees(),
        )
    }
}

impl PyNavFix {
    pub(crate) fn equals_recorded_fix(&self, mut other: NavFix) -> bool {
        let mut recorded = self.inner;
        if recorded.position_provenance.is_none()
            && other.position_provenance.is_none()
            && recorded.state.is_some()
            && other.state.is_some()
            && recorded.lat.as_degrees().is_nan()
            && recorded.lon.as_degrees().is_nan()
            && other.lat.as_degrees().is_nan()
            && other.lon.as_degrees().is_nan()
        {
            recorded.lat = Angle::degrees(0.0);
            recorded.lon = Angle::degrees(0.0);
            other.lat = Angle::degrees(0.0);
            other.lon = Angle::degrees(0.0);
        }
        recorded == other
    }
}
