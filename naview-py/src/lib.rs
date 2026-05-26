//! Python bindings for `naview-sdk`.
//!
//! Exposes the core read/write API as a Python extension module named
//! `naview_sdk._naview_sdk`.  The public `naview_sdk` package re-exports
//! everything from this module via `python/naview_sdk/__init__.py`.

use chrono::{DateTime, FixedOffset, Utc};
use std::path::PathBuf;
use naview_sdk::{
    Angle, Annotation, BuildError, Constellation, Marker, MarkerIcon, Meta, NavFile,
    NavFileBuilder, NavFix, NavPoint, Satellite, SatelliteReport, Velocity, degree,
    meter_per_second,
};
use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;


fn to_fixed(dt: DateTime<Utc>) -> DateTime<FixedOffset> {
    dt.fixed_offset()
}

fn build_err(e: BuildError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn io_err(e: naview_sdk::Error) -> PyErr {
    PyIOError::new_err(e.to_string())
}

fn consumed_err() -> PyErr {
    PyRuntimeError::new_err("builder already consumed by finish()")
}



/// GNSS constellation identifier.
#[pyclass(eq, eq_int, from_py_object, name = "Constellation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyConstellation {
    #[pyo3(name = "GPS")]
    Gps = 0,
    #[pyo3(name = "GLONASS")]
    Glonass = 1,
    #[pyo3(name = "GALILEO")]
    Galileo = 2,
    #[pyo3(name = "BEIDOU")]
    Beidou = 3,
}

impl From<Constellation> for PyConstellation {
    fn from(c: Constellation) -> Self {
        match c {
            Constellation::Gps => Self::Gps,
            Constellation::Glonass => Self::Glonass,
            Constellation::Galileo => Self::Galileo,
            Constellation::Beidou => Self::Beidou,
        }
    }
}

impl From<PyConstellation> for Constellation {
    fn from(c: PyConstellation) -> Self {
        match c {
            PyConstellation::Gps => Self::Gps,
            PyConstellation::Glonass => Self::Glonass,
            PyConstellation::Galileo => Self::Galileo,
            PyConstellation::Beidou => Self::Beidou,
        }
    }
}



/// Visual icon for a map annotation marker.
#[pyclass(eq, eq_int, from_py_object, name = "MarkerIcon")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyMarkerIcon {
    #[pyo3(name = "PIN")]
    Pin = 0,
    #[pyo3(name = "CROSS")]
    Cross = 1,
    #[pyo3(name = "CIRCLE")]
    Circle = 2,
    #[pyo3(name = "LIGHTNING")]
    Lightning = 3,
    #[pyo3(name = "WARNING")]
    Warning = 4,
    #[pyo3(name = "ERROR")]
    Error = 5,
    #[pyo3(name = "CHECK")]
    Check = 6,
}

impl From<MarkerIcon> for PyMarkerIcon {
    fn from(i: MarkerIcon) -> Self {
        match i {
            MarkerIcon::Pin => Self::Pin,
            MarkerIcon::Cross => Self::Cross,
            MarkerIcon::Circle => Self::Circle,
            MarkerIcon::Lightning => Self::Lightning,
            MarkerIcon::Warning => Self::Warning,
            MarkerIcon::Error => Self::Error,
            MarkerIcon::Check => Self::Check,
        }
    }
}

impl From<PyMarkerIcon> for MarkerIcon {
    fn from(i: PyMarkerIcon) -> Self {
        match i {
            PyMarkerIcon::Pin => Self::Pin,
            PyMarkerIcon::Cross => Self::Cross,
            PyMarkerIcon::Circle => Self::Circle,
            PyMarkerIcon::Lightning => Self::Lightning,
            PyMarkerIcon::Warning => Self::Warning,
            PyMarkerIcon::Error => Self::Error,
            PyMarkerIcon::Check => Self::Check,
        }
    }
}



/// One tracked satellite with optional signal metrics.
#[pyclass(from_py_object, name = "Satellite")]
#[derive(Debug, Clone, Copy)]
pub struct PySatellite {
    inner: Satellite,
}

#[pymethods]
impl PySatellite {
    /// Create a satellite entry.
    ///
    /// `elevation` and `azimuth` are in degrees; `snr` is in dB-Hz.
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
            .constellation(constellation.into())
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
    #[getter]
    fn snr(&self) -> Option<f32> {
        self.inner.snr
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
        let c = match self.inner.constellation {
            Constellation::Gps => "GPS",
            Constellation::Glonass => "GLONASS",
            Constellation::Galileo => "GALILEO",
            Constellation::Beidou => "BEIDOU",
        };
        format!("Satellite(constellation=Constellation.{c}, prn={})", self.inner.prn)
    }
}



/// A set of satellites tracked at a point in time.
///
/// Supply at least one of `gps_time` or `sys_time`.
/// Both must be timezone-aware `datetime.datetime` objects.
#[pyclass(skip_from_py_object, name = "SatelliteReport")]
#[derive(Debug, Clone)]
pub struct PySatelliteReport {
    inner: SatelliteReport,
}

#[pymethods]
impl PySatelliteReport {
    #[new]
    #[pyo3(signature = (tracked, *, gps_time=None, sys_time=None))]
    fn new(
        tracked: Vec<PySatellite>,
        gps_time: Option<DateTime<FixedOffset>>,
        sys_time: Option<DateTime<FixedOffset>>,
    ) -> Self {
        let inner = SatelliteReport::builder()
            .tracked(tracked.iter().map(|s| s.inner).collect())
            .maybe_gps_time(gps_time.map(|t| t.to_utc()))
            .maybe_sys_time(sys_time.map(|t| t.to_utc()))
            .build();
        Self { inner }
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

    /// GPS-domain timestamp, or `None` if the receiver had no active fix.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.gps_time.map(to_fixed)
    }

    /// System-clock timestamp at capture time, or `None`.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.sys_time.map(to_fixed)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.gps_time == other.inner.gps_time
            && self.inner.sys_time == other.inner.sys_time
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



/// A single GPS/GNSS fix: position, optional heading, and optional speed.
///
/// Provide at least one of `gps_time` or `sys_time`.
/// All `datetime` arguments must be timezone-aware.
/// `lat` and `lon` are in degrees; `heading` in degrees [0, 360);
/// `speed_mps` in m/s; `eph_m` is the horizontal accuracy radius in metres.
#[pyclass(skip_from_py_object, name = "NavFix")]
#[derive(Debug, Clone, Copy)]
pub struct PyNavFix {
    inner: NavFix,
}

#[pymethods]
impl PyNavFix {
    #[new]
    #[pyo3(signature = (lat, lon, *, gps_time=None, sys_time=None, heading=None, speed_mps=None, eph_m=None))]
    fn new(
        lat: f64,
        lon: f64,
        gps_time: Option<DateTime<FixedOffset>>,
        sys_time: Option<DateTime<FixedOffset>>,
        heading: Option<f64>,
        speed_mps: Option<f64>,
        eph_m: Option<f64>,
    ) -> Self {
        let inner = NavFix::builder()
            .lat(Angle::new::<degree>(lat))
            .lon(Angle::new::<degree>(lon))
            .maybe_gps_time(gps_time.map(|t| t.to_utc()))
            .maybe_sys_time(sys_time.map(|t| t.to_utc()))
            .maybe_heading(heading.map(|h| Angle::new::<degree>(h)))
            .maybe_speed(speed_mps.map(|s| Velocity::new::<meter_per_second>(s)))
            .maybe_eph_m(eph_m)
            .build();
        Self { inner }
    }

    /// Latitude in degrees.
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.lat.get::<degree>()
    }

    /// Longitude in degrees.
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.lon.get::<degree>()
    }

    /// GPS-domain timestamp (timezone-aware UTC), or `None`.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.gps_time.map(to_fixed)
    }

    /// System-clock timestamp (timezone-aware UTC), or `None`.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.sys_time.map(to_fixed)
    }

    /// Heading in degrees [0, 360), or `None` for ghost/unknown-direction fixes.
    #[getter]
    fn heading(&self) -> Option<f64> {
        self.inner.heading.map(|h| h.get::<degree>())
    }

    /// Speed in m/s, or `None`.
    #[getter]
    fn speed_mps(&self) -> Option<f64> {
        self.inner.speed.map(|s| s.get::<meter_per_second>())
    }

    /// Estimated horizontal accuracy radius in metres, or `None`.
    #[getter]
    fn eph_m(&self) -> Option<f64> {
        self.inner.eph_m
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.gps_time == other.inner.gps_time
            && self.inner.sys_time == other.inner.sys_time
            && self.inner.lat.get::<degree>() == other.inner.lat.get::<degree>()
            && self.inner.lon.get::<degree>() == other.inner.lon.get::<degree>()
            && self.inner.heading.map(|h| h.get::<degree>())
                == other.inner.heading.map(|h| h.get::<degree>())
            && self.inner.speed.map(|s| s.get::<meter_per_second>())
                == other.inner.speed.map(|s| s.get::<meter_per_second>())
            && self.inner.eph_m == other.inner.eph_m
    }

    fn __repr__(&self) -> String {
        format!(
            "NavFix(lat={:.6}, lon={:.6})",
            self.inner.lat.get::<degree>(),
            self.inner.lon.get::<degree>(),
        )
    }
}



/// A user-defined map annotation with an optional label and icon.
///
/// `time` must be a timezone-aware `datetime.datetime`.
#[pyclass(skip_from_py_object, name = "Annotation")]
#[derive(Debug, Clone)]
pub struct PyAnnotation {
    inner: Annotation,
}

#[pymethods]
impl PyAnnotation {
    #[new]
    #[pyo3(signature = (time, *, label=None, icon=None))]
    fn new(
        time: DateTime<FixedOffset>,
        label: Option<String>,
        icon: Option<PyMarkerIcon>,
    ) -> Self {
        let inner = Annotation::builder()
            .time(time.to_utc())
            .maybe_label(label)
            .maybe_icon(icon.map(MarkerIcon::from))
            .build();
        Self { inner }
    }

    /// Timestamp (timezone-aware UTC).
    #[getter]
    fn time(&self) -> DateTime<FixedOffset> {
        to_fixed(self.inner.time)
    }

    /// Display label, or `None`.
    #[getter]
    fn label(&self) -> Option<&str> {
        self.inner.label.as_deref()
    }

    /// Visual icon, or `None` (defaults to `MarkerIcon.PIN` when rendered).
    #[getter]
    fn icon(&self) -> Option<PyMarkerIcon> {
        self.inner.icon.map(PyMarkerIcon::from)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.time == other.inner.time
            && self.inner.label == other.inner.label
            && self.inner.icon == other.inner.icon
    }

    fn __repr__(&self) -> String {
        format!("Annotation(time={:?})", self.inner.time)
    }
}



/// Optional file-level metadata for a `.nvd` file.
#[pyclass(skip_from_py_object, name = "Meta")]
#[derive(Debug, Clone)]
pub struct PyMeta {
    inner: Meta,
}

#[pymethods]
impl PyMeta {
    #[new]
    #[pyo3(signature = (*, title=None, device=None, notes=None))]
    fn new(
        title: Option<String>,
        device: Option<String>,
        notes: Option<String>,
    ) -> Self {
        let inner = Meta::builder()
            .maybe_title(title)
            .maybe_device(device)
            .maybe_notes(notes)
            .build();
        Self { inner }
    }

    /// File title, or `None`.
    #[getter]
    fn title(&self) -> Option<&str> {
        self.inner.title.as_deref()
    }

    /// Sensor or device that produced the data, or `None`.
    #[getter]
    fn device(&self) -> Option<&str> {
        self.inner.device.as_deref()
    }

    /// Free-text notes, or `None`.
    #[getter]
    fn notes(&self) -> Option<&str> {
        self.inner.notes.as_deref()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner.title == other.inner.title
            && self.inner.device == other.inner.device
            && self.inner.notes == other.inner.notes
    }

    fn __repr__(&self) -> String {
        format!("Meta(title={:?})", self.inner.title)
    }
}



/// A nav fix combined with its associated satellite report, as read from a file.
#[pyclass(skip_from_py_object, name = "NavPoint")]
#[derive(Debug, Clone)]
pub struct PyNavPoint {
    inner: NavPoint,
}

#[pymethods]
impl PyNavPoint {
    /// Latitude in degrees.
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.fix.lat.get::<degree>()
    }

    /// Longitude in degrees.
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.fix.lon.get::<degree>()
    }

    /// GPS-domain timestamp (timezone-aware UTC), or `None`.
    #[getter]
    fn gps_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.fix.gps_time.map(to_fixed)
    }

    /// System-clock timestamp (timezone-aware UTC), or `None`.
    #[getter]
    fn sys_time(&self) -> Option<DateTime<FixedOffset>> {
        self.inner.fix.sys_time.map(to_fixed)
    }

    /// Heading in degrees [0, 360), or `None`.
    #[getter]
    fn heading(&self) -> Option<f64> {
        self.inner.fix.heading.map(|h| h.get::<degree>())
    }

    /// Speed in m/s, or `None`.
    #[getter]
    fn speed_mps(&self) -> Option<f64> {
        self.inner.fix.speed.map(|s| s.get::<meter_per_second>())
    }

    /// Estimated horizontal accuracy radius in metres, or `None`.
    #[getter]
    fn eph_m(&self) -> Option<f64> {
        self.inner.fix.eph_m
    }

    /// Associated satellite report, or `None` if none was recorded at this fix.
    #[getter]
    fn satellites(&self) -> Option<PySatelliteReport> {
        self.inner
            .satellites
            .clone()
            .map(|s| PySatelliteReport { inner: s })
    }

    fn __repr__(&self) -> String {
        format!(
            "NavPoint(lat={:.6}, lon={:.6})",
            self.inner.fix.lat.get::<degree>(),
            self.inner.fix.lon.get::<degree>(),
        )
    }
}



/// A map annotation with its interpolated position on the nav track.
#[pyclass(skip_from_py_object, name = "Marker")]
#[derive(Debug, Clone)]
pub struct PyMarker {
    inner: Marker,
}

#[pymethods]
impl PyMarker {
    /// Interpolated latitude in degrees.
    #[getter]
    fn lat(&self) -> f64 {
        self.inner.lat.get::<degree>()
    }

    /// Interpolated longitude in degrees.
    #[getter]
    fn lon(&self) -> f64 {
        self.inner.lon.get::<degree>()
    }

    /// The underlying annotation.
    #[getter]
    fn annotation(&self) -> PyAnnotation {
        PyAnnotation {
            inner: self.inner.annotation.clone(),
        }
    }

    /// Display label from the annotation, or `None`.
    #[getter]
    fn label(&self) -> Option<&str> {
        self.inner.annotation.label.as_deref()
    }

    /// Visual icon from the annotation, or `None`.
    #[getter]
    fn icon(&self) -> Option<PyMarkerIcon> {
        self.inner.annotation.icon.map(PyMarkerIcon::from)
    }

    /// Annotation timestamp (timezone-aware UTC).
    #[getter]
    fn time(&self) -> DateTime<FixedOffset> {
        to_fixed(self.inner.annotation.time)
    }

    fn __repr__(&self) -> String {
        format!(
            "Marker(lat={:.6}, lon={:.6})",
            self.inner.lat.get::<degree>(),
            self.inner.lon.get::<degree>(),
        )
    }
}



/// A parsed `.nvd` navigation data file.
///
/// Construct via `NavFileBuilder.finish()` to write, or `NavFile.open(path)`
/// to read.
#[pyclass(skip_from_py_object, name = "NavFile")]
pub struct PyNavFile {
    inner: NavFile,
}

#[pymethods]
impl PyNavFile {
    /// Open and parse a `.nvd` file at `path`.
    ///
    /// Accepts any path-like value: `str`, `bytes`, or a `pathlib.Path` object.
    #[staticmethod]
    fn open(path: PathBuf) -> PyResult<Self> {
        NavFile::open(&path)
            .map(|f| Self { inner: f })
            .map_err(io_err)
    }

    /// Write this file to `path`.
    ///
    /// Accepts any path-like value: `str`, `bytes`, or a `pathlib.Path` object.
    /// Appends `.nvd` if `path` has no extension.
    fn write_to_file(&self, path: PathBuf) -> PyResult<()> {
        self.inner.write_to_file(&path).map_err(io_err)
    }

    /// Serialise the file to a `bytes` object.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = Vec::new();
        self.inner.write(&mut buf).map_err(io_err)?;
        Ok(PyBytes::new(py, &buf))
    }

    /// File-level metadata.
    #[getter]
    fn meta(&self) -> PyMeta {
        PyMeta {
            inner: self.inner.meta.clone(),
        }
    }

    /// All nav points in chronological order.
    #[getter]
    fn points(&self) -> Vec<PyNavPoint> {
        self.inner
            .nav_points()
            .iter()
            .map(|p| PyNavPoint { inner: p.clone() })
            .collect()
    }

    /// All map markers with their interpolated positions.
    #[getter]
    fn markers(&self) -> Vec<PyMarker> {
        self.inner
            .markers()
            .iter()
            .map(|m| PyMarker { inner: m.clone() })
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "NavFile(points={}, markers={})",
            self.inner.nav_points().len(),
            self.inner.markers().len(),
        )
    }
}



/// Dispatch target for [`PyNavFileBuilder::add`].
#[derive(FromPyObject)]
enum AddItem<'py> {
    Fix(Bound<'py, PyNavFix>),
    Report(Bound<'py, PySatelliteReport>),
    Annotation(Bound<'py, PyAnnotation>),
}

/// Assembles nav fixes, satellite reports, and annotations into a `NavFile`.
///
/// ```python
/// nav_file = (
///     NavFileBuilder()
///     .set_meta(Meta(title="My Track"))
///     .add(NavFix(lat=51.5, lon=-0.1, gps_time=...))
///     .finish()
/// )
/// nav_file.write_to_file("track.nvd")
/// ```
///
/// Calling `finish()` consumes the builder; further calls raise `RuntimeError`.
#[pyclass(skip_from_py_object, name = "NavFileBuilder")]
pub struct PyNavFileBuilder {
    inner: Option<NavFileBuilder>,
}

#[pymethods]
impl PyNavFileBuilder {
    #[new]
    fn new() -> Self {
        Self {
            inner: Some(NavFileBuilder::new()),
        }
    }

    /// Attach file-level metadata.
    ///
    /// Returns `self` to allow method chaining.
    fn set_meta(slf: Bound<'_, Self>, meta: &PyMeta) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            let builder = b.inner.take().ok_or_else(consumed_err)?;
            b.inner = Some(builder.with_meta(meta.inner.clone()));
        }
        Ok(slf.unbind())
    }

    /// Add a nav fix, satellite report, or annotation to the file.
    ///
    /// Accepts a `NavFix`, `SatelliteReport`, or `Annotation` and dispatches
    /// to the appropriate internal method.  Returns `self` to allow chaining:
    ///
    /// ```python
    /// builder.add(NavFix(...)).add(SatelliteReport(...)).add(Annotation(...))
    /// ```
    fn add(slf: Bound<'_, Self>, item: AddItem<'_>) -> PyResult<Py<Self>> {
        {
            let mut b = slf.borrow_mut();
            let builder = b.inner.as_mut().ok_or_else(consumed_err)?;
            match item {
                AddItem::Fix(f) => { builder.add_nav_fix(f.borrow().inner); }
                AddItem::Report(r) => { builder.add_satellite_report(r.borrow().inner.clone()); }
                AddItem::Annotation(a) => { builder.add_annotation(a.borrow().inner.clone()); }
            }
        }
        Ok(slf.unbind())
    }

    /// Process all data and return a `NavFile`.
    ///
    /// Consumes the builder — calling `finish()` again raises `RuntimeError`.
    fn finish(&mut self) -> PyResult<PyNavFile> {
        self.inner
            .take()
            .ok_or_else(consumed_err)?
            .finish()
            .map(|f| PyNavFile { inner: f })
            .map_err(build_err)
    }
}



#[pymodule]
fn _naview_sdk(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyConstellation>()?;
    m.add_class::<PyMarkerIcon>()?;
    m.add_class::<PySatellite>()?;
    m.add_class::<PySatelliteReport>()?;
    m.add_class::<PyNavFix>()?;
    m.add_class::<PyAnnotation>()?;
    m.add_class::<PyMeta>()?;
    m.add_class::<PyNavPoint>()?;
    m.add_class::<PyMarker>()?;
    m.add_class::<PyNavFile>()?;
    m.add_class::<PyNavFileBuilder>()?;
    Ok(())
}
