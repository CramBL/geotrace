use chrono::{DateTime, FixedOffset};
use geotrace_sdk::{Constellation, Satellite, SatelliteReport};
use geotrace_sdk_units::snr;
use pyo3::prelude::*;

use crate::enums::PyConstellation;
use crate::timestamps::{self, TimestampArguments};

/// One tracked satellite with optional signal metrics.
///
/// `elevation`, `azimuth` and `snr` are stored as 32-bit floats: a value read
/// back can differ from the value written in its eighth significant digit.
#[pyclass(from_py_object, name = "Satellite")]
#[derive(Clone, Copy, Debug)]
pub struct PySatellite {
    inner: Satellite,
}

#[pymethods]
impl PySatellite {
    /// Create a satellite entry.
    ///
    /// `elevation` and `azimuth` are in degrees. `snr` is in dB-Hz.
    #[new]
    #[pyo3(signature = (constellation, prn, *, in_fix=false, elevation=None, azimuth=None, snr=None))]
    fn new(
        constellation: PyConstellation,
        prn: u32,
        in_fix: bool,
        elevation: Option<f32>,
        azimuth: Option<f32>,
        snr: Option<f32>,
    ) -> Self {
        let inner = Satellite::builder()
            .constellation(Constellation::from(constellation))
            .prn(prn)
            .in_fix(in_fix)
            .maybe_elevation(elevation)
            .maybe_azimuth(azimuth)
            .maybe_snr(snr)
            .build();
        Self { inner }
    }

    #[getter]
    fn constellation(&self) -> PyConstellation {
        self.inner.constellation.into()
    }

    #[getter]
    fn prn(&self) -> u32 {
        self.inner.prn
    }

    #[getter]
    fn in_fix(&self) -> bool {
        self.inner.in_fix
    }

    /// Elevation above horizon in degrees, or `None`.
    #[getter]
    fn elevation(&self) -> Option<f32> {
        self.inner.elevation
    }

    /// Azimuth from true north in degrees, or `None`.
    #[getter]
    fn azimuth(&self) -> Option<f32> {
        self.inner.azimuth
    }

    /// Signal-to-noise ratio in dB-Hz, or `None`.
    ///
    /// The reader returns a stored value unchanged, which includes a reading
    /// for which `snr_is_no_data_sentinel()` returns `True`.
    #[getter]
    fn snr(&self) -> Option<f32> {
        self.inner.snr
    }

    /// Whether `snr` holds a reading for which `snr_is_no_data_sentinel()`
    /// returns `True`.
    #[getter]
    fn snr_is_no_data_sentinel(&self) -> bool {
        self.inner.snr_is_no_data_sentinel()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.constellation == other.inner.constellation
            && self.inner.prn == other.inner.prn
            && self.inner.in_fix == other.inner.in_fix
            && self.inner.elevation == other.inner.elevation
            && self.inner.azimuth == other.inner.azimuth
            && self.inner.snr == other.inner.snr
    }

    fn __repr__(&self) -> String {
        let member_name: &str = PyConstellation::from(self.inner.constellation).into();
        format!(
            "Satellite(constellation=Constellation.{member_name}, prn={})",
            self.inner.prn
        )
    }
}

/// A set of satellites tracked at a point in time.
///
/// Raises `ValueError` when `gps_time` and `sys_time` are both `None`.
/// Both must be timezone-aware `datetime.datetime` objects.
#[pyclass(skip_from_py_object, name = "SatelliteReport")]
#[derive(Clone, Debug)]
pub struct PySatelliteReport {
    pub(crate) inner: SatelliteReport,
}

#[pymethods]
impl PySatelliteReport {
    #[new]
    #[pyo3(signature = (tracked, *, gps_time=None, sys_time=None))]
    fn new(
        tracked: Vec<PySatellite>,
        gps_time: Option<DateTime<FixedOffset>>,
        sys_time: Option<DateTime<FixedOffset>>,
    ) -> PyResult<Self> {
        let time =
            timestamps::nav_fix_time_or_value_error(TimestampArguments { gps_time, sys_time })?;
        let inner = SatelliteReport::builder()
            .time(time)
            .tracked(tracked.into_iter().map(|s| s.inner).collect())
            .build();
        Ok(Self { inner })
    }

    /// All satellites currently tracked (may include satellites not in the fix).
    #[getter]
    fn tracked(&self) -> Vec<PySatellite> {
        self.inner
            .tracked
            .iter()
            .map(|s| PySatellite { inner: *s })
            .collect()
    }

    /// GPS-domain timestamp (timezone-aware UTC), or `None` for a report
    /// constructed without one, or read from a file that stores none.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.gps_time().map(|time| time.fixed_offset())
    }

    /// System-clock timestamp at capture time (timezone-aware UTC), or `None`
    /// for a report constructed without one, or read from a file that stores
    /// none.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.sys_time().map(|time| time.fixed_offset())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.gps_time() == other.inner.gps_time()
            && self.inner.sys_time() == other.inner.sys_time()
            && self.inner.tracked.len() == other.inner.tracked.len()
            && self
                .inner
                .tracked
                .iter()
                .zip(other.inner.tracked.iter())
                .all(|(a, b)| {
                    a.constellation == b.constellation
                        && a.prn == b.prn
                        && a.in_fix == b.in_fix
                        && a.elevation == b.elevation
                        && a.azimuth == b.azimuth
                        && a.snr == b.snr
                })
    }

    fn __repr__(&self) -> String {
        format!("SatelliteReport(tracked={})", self.inner.tracked.len())
    }
}

/// Whether ``snr`` is the SNR some receiver firmware sends when it has no
/// measurement: 99 dB-Hz, within the tolerance the Rust SDK sets.
///
/// The function rounds ``snr`` to a 32-bit float first, as ``Satellite`` stores it.
#[pyfunction]
pub(crate) fn snr_is_no_data_sentinel(snr: f32) -> bool {
    snr::is_no_data_sentinel(snr)
}
